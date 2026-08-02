//! `recv` subcommand: build a live transport from a URL, drive a
//! [`DemuxReceiver`] over it until either the stream ends or a
//! wall-clock deadline passes, and check the result against a
//! [`Profile`]'s invariants — the live-capture counterpart to
//! `verify::verify_file`'s offline-file check.

use std::time::{Duration, Instant};

use tst_core::transport::RecvTransport;
use tst_pipeline::{DemuxReceiver, ShellError, ShellErrorKind};

use crate::profiles::Profile;
use crate::report_types::VerifyReport;
use crate::transport::{self, Teeing};
use crate::verify::{self, Tally};

/// How long to wait for the FIRST demuxed event before giving up
/// entirely — the sender never connected or never sent anything at all
/// (a genuinely broken run, not a slow-starting one). Generous on
/// purpose: connection setup time varies across schemes (e.g. a
/// retry-on-connect SRT caller, see `tests/loopback.rs`, can take a
/// few seconds even in the well-behaved case).
const NO_DATA_TIMEOUT: Duration = Duration::from_secs(15);

/// Once data has started arriving, how much longer than the profile's
/// `seconds` window to keep listening before closing — covers sender
/// startup jitter and in-flight packets near the end of the window.
const POST_START_GRACE: Duration = Duration::from_secs(2);

/// Build a transport from `url`, receive `seconds` of `expect`'s
/// traffic from it, and check the result against `expect`'s
/// invariants. Writes the same [`VerifyReport`] as JSON to `json_out`
/// (or stdout for `"-"`) when given.
pub fn run(
    url: &str,
    expect: &Profile,
    seconds: f64,
    json_out: Option<&str>,
) -> Result<VerifyReport, String> {
    let transport = transport::make_recv(url)?;
    let report = recv_over_transport(transport, expect, seconds)?;
    if let Some(target) = json_out {
        write_json(target, &report)?;
    }
    Ok(report)
}

/// Core of [`run`], split out so a caller that already holds a
/// constructed transport (e.g. this crate's `tests/loopback.rs`, which
/// binds/listens on its own thread before spawning a sender so the two
/// sides can't race without a fixed sleep) can drive the same receive
/// loop without going through `--url` parsing twice.
///
/// Drives `rx.recv_event()` in a loop, folding every [`tst_core::
/// mpegts::demux::DemuxEvent`] into a [`Tally`], until one of:
/// - the transport reports a clean/broken end (`Ok(None)`, or an error
///   whose kind is `Closed`/`EndOfStream`/`TransportBroken` — this
///   crate runs bounded test cells, so any of these three just means
///   "the capture is over," not a process-level failure; genuine
///   problems fold into the resulting `VerifyReport.pass` instead, the
///   same way `verify::verify_file` never distinguishes "no data" from
///   an IO error),
/// - or a wall-clock deadline passes, in which case `rx.close()` is
///   called (same-thread close-then-recv — every transport this crate
///   builds maps that to a `Closed`/`EndOfStream` on the very next
///   call, see `transport.rs`'s per-scheme docs) and the loop drains to
///   `Ok(None)`.
///
/// The deadline starts at [`NO_DATA_TIMEOUT`] (waiting for the stream
/// to start) and is re-anchored to `seconds + `[`POST_START_GRACE`]
/// once the first event arrives, so a slow connection setup doesn't eat
/// into the profile's own capture window.
pub fn recv_over_transport(
    transport: Box<dyn RecvTransport>,
    expect: &Profile,
    seconds: f64,
) -> Result<VerifyReport, String> {
    let (teeing, tap) = Teeing::new(transport);
    let mut rx = DemuxReceiver::new(teeing);

    let mut deadline = Instant::now() + NO_DATA_TIMEOUT;
    let mut streaming = false;
    let mut closed = false;
    let mut tally = Tally::new();

    loop {
        if !closed && Instant::now() >= deadline {
            rx.close();
            closed = true;
        }
        match rx.recv_event() {
            Ok(Some(ev)) => {
                if !streaming {
                    streaming = true;
                    deadline = Instant::now() + Duration::from_secs_f64(seconds) + POST_START_GRACE;
                }
                tally.feed(&ev);
            }
            Ok(None) => break,
            Err(e) => match e.kind() {
                ShellErrorKind::Backpressure => continue,
                ShellErrorKind::Closed
                | ShellErrorKind::EndOfStream
                | ShellErrorKind::TransportBroken => break,
                other => return Err(format!("recv_event: {e} (kind {other:?})")),
            },
        }
    }
    // Explicit drop before reading the tee tally back — `tee_tally`
    // requires the `Teeing` (owned by `rx`'s inner transport state) to
    // have no other owner.
    drop(rx);

    let mut report = tally.finish(expect, seconds, verify::NOMINAL_COUNT_SLACK);
    // `Tally`'s own bytes/stream_sha256 fields were never fed (we never
    // called `note_bytes` on it) — the `Teeing` tap captured the exact
    // bytes at the transport boundary instead, which is the
    // byte-transparent ground truth this crate wants (independent of
    // the demuxer's internal packet-alignment chunking). Overwrite the
    // two fields `finish` computed from the unfed (empty) hasher with
    // the real tally.
    let (bytes, stream_sha256) = transport::tee_tally(tap);
    report.metrics.bytes = bytes;
    report.metrics.stream_sha256 = stream_sha256;
    Ok(report)
}

fn write_json(target: &str, report: &VerifyReport) -> Result<(), String> {
    let json = serde_json::to_string_pretty(report).expect("VerifyReport always serializes");
    if target == "-" {
        println!("{json}");
    } else {
        std::fs::write(target, json).map_err(|e| format!("write {target}: {e}"))?;
    }
    Ok(())
}
