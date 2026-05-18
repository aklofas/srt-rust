//! Pair sync-KLV with video AUs over a captured `.ts` file using
//! `tst_pipeline::ext::pairing::Pairer`.
//!
//! This example is the `Pairer`-using sibling of the inline cookbook
//! recipe in `docs/cookbook.md` §12. The cookbook recipe shows the
//! ~20-line pattern using just `DemuxEvent` matches; this example
//! shows the same workflow expressed through the opt-in `Pairer`
//! convenience type.
//!
//! The Pairer's value-add: bounded KLV history, a `Buffered` mode that
//! does bidirectional matching, telemetry counters, and typed
//! projection structs (`VideoSample` + `KlvSample`) so caller code
//! doesn't have to re-match `DemuxEvent` arms after the pair.
//!
//! Usage:
//!
//! ```text
//! cargo run -p tst-examples --example pair_klv_pipeline -- path/to/capture.ts
//! ```
//!
//! For a live SRT feed, the same pattern works — replace the file
//! reader with `DemuxReceiver::new(transport)` and call `recv_event`
//! in the loop.

use std::env;
use std::fs;
use std::process::ExitCode;
use std::time::Duration;
use tst_core::mpegts::demux::Demuxer;
use tst_pipeline::ext::pairing::{Pairer, PairerConfig, PairerMode, PairerOutput};

fn main() -> ExitCode {
    // --- Argument parsing -------------------------------------------------
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("usage: pair_klv_pipeline <path/to/capture.ts>");
        return ExitCode::from(2);
    }
    let path = &args[1];

    // --- Stream parameters ------------------------------------------------
    //
    // For real captures we'd discover these from a `ProgramMap` event
    // and either configure the pairer once topology is known, or
    // reconfigure when the PMT version bumps. For a teaching example
    // we hard-code the canonical synthetic-fixture PIDs.
    //
    // 0.3 s tolerance: wide enough to absorb encoder timestamp drift;
    // narrow enough to reject coincidental near-matches from the next
    // GOP.
    const VIDEO_PID: u16 = 0x100;
    const KLV_PID: u16 = 0x102;
    const TOLERANCE: Duration = Duration::from_millis(300);
    // 32 buffered entries covers ~1 s at 30 fps + 1:1 KLV cadence;
    // 32 s at 1 Hz async cadence.
    const MAX_BUFFERED_KLV: u64 = 32;

    // --- Read + demux + pair ---------------------------------------------
    //
    // For a finite file we feed all bytes at once and then call
    // `flush()` on both the demuxer and the pairer. For a live SRT
    // feed, you'd read in 1316-byte chunks (libsrt live MTU) inside a
    // loop, call `feed` per chunk, and `flush` only on `Closed`.
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::from(1);
        }
    };
    let mut demux = Demuxer::new();
    if let Err(e) = demux.feed(&bytes) {
        eprintln!("demux feed error: {e:?}");
        return ExitCode::from(1);
    }
    // Flush pulls the trailing partial PES out of reassembly state.
    // On live feeds, this is what `DemuxReceiver` does at
    // `TransportError::Closed`.
    demux.flush();

    // Realtime mode: video events emit immediately. The bookmark for
    // when to pick `Buffered` instead is "I have lots of UnpairedVideo
    // because the encoder ships KLV PES after video PES." See
    // `docs/troubleshooting.md`.
    //
    // PairerConfig is `#[non_exhaustive]`, so construct via
    // `Default::default()` + assignment rather than struct literal.
    let mut opts = PairerConfig::default();
    opts.mode = PairerMode::Realtime;
    opts.tolerance = TOLERANCE;
    opts.max_buffered_klv = MAX_BUFFERED_KLV;
    opts.max_buffered_video = MAX_BUFFERED_KLV;
    let mut pairer = Pairer::with_options(VIDEO_PID, KLV_PID, opts);

    while let Some(event) = demux.next_event() {
        for output in pairer.feed(event) {
            match output {
                PairerOutput::Paired { video, klv } => {
                    // Real consumer: feed `video.payload` into a
                    // decoder (Annex-B reconstitute via cookbook
                    // recipe 18) and `klv.payload` into
                    // `tst_core::klv::st0601::decode`.
                    let _ = (video, klv);
                }
                PairerOutput::UnpairedVideo(_v) => {}
                PairerOutput::UnpairedKlv(_k) => {}
                PairerOutput::PassThrough(_e) => {
                    // PMT (use to confirm topology), NonConformant
                    // (log for diagnostics), audio Sample, etc.
                }
            }
        }
    }
    // Drain pending video buffer / unused KLV so the counters reflect
    // the full stream.
    for _ in pairer.flush() {}

    // --- Summary ----------------------------------------------------------
    let stats = pairer.stats();
    println!("paired         = {}", stats.paired);
    println!("unpaired_video = {}", stats.unpaired_video);
    println!("unpaired_klv   = {}", stats.unpaired_klv);
    println!("pass_through   = {}", stats.pass_through);
    let pairing_rate = if stats.paired + stats.unpaired_video > 0 {
        100.0 * stats.paired as f64 / (stats.paired + stats.unpaired_video) as f64
    } else {
        0.0
    };
    println!("video pairing rate = {pairing_rate:.1}%");

    ExitCode::SUCCESS
}
