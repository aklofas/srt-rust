//! Bind an SRT listener, accept one peer, dump the typed `DemuxEvent`
//! stream from `pipeline::Receiver`.
//!
//! Why this exists: the smallest end-to-end demonstration that
//! `pipeline::Receiver` turns "bytes in via SRT" into "typed events
//! out" with no intermediate demuxer plumbing visible to the caller.
//! `Sender` on the publishing side and `Receiver` on the consuming
//! side are deliberately mirror-image shapes — see
//! `pipeline_send_to_socket.rs` for the producer counterpart, and run
//! the two together for an end-to-end smoke.
//!
//! Usage:
//!   # terminal A (this example)
//!   cargo run --example srt_recv_typed -- 9000
//!
//!   # terminal B (the publisher)
//!   cargo run --example pipeline_send_to_socket -- 127.0.0.1:9000
//!
//! What to look for in the output:
//! - One `ProgramMap` line shortly after the peer connects (the demuxer
//!   needs the PAT + PMT before it can label streams; for a 100ms PSI
//!   cadence sender, ~100ms after first byte).
//! - `Sample` lines as video AUs arrive. The `pts` should advance by
//!   ~3000 ticks per frame at 30 fps (90 kHz / 30 = 3000).
//! - `Metadata` lines if the publisher sends KLV.
//! - Clean exit when the publisher closes — no error message, just
//!   "peer closed cleanly".

use srt_core::mpegts::demux::DemuxEvent;
use srt_core::pipeline::transport::TransportError;
use srt_core::pipeline::{Receiver, ReceiverError, SrtTransport};
use srt_core::srt::ListenerBuilder;
use std::env;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port: u16 = env::args()
        .nth(1)
        .unwrap_or_else(|| "9000".into())
        .parse()
        .expect("port must be u16");

    // Build the listener with a 120ms latency budget. SRT's `latency`
    // option is the wall-clock delay the receiver buffers before
    // releasing packets — long enough to absorb typical jitter and a
    // handful of retransmissions, short enough that interactive use
    // doesn't suffer. Both peers must agree on this value (libsrt
    // negotiates the maximum during handshake), so the matching
    // `pipeline_send_to_socket` example uses the same 120ms.
    //
    // `bind` is the terminal call on `ListenerBuilder` — no separate
    // `.build()` step. It returns a ready-to-accept `Listener`.
    let mut listener = ListenerBuilder::new()
        .latency(Duration::from_millis(120))
        .bind(format!("0.0.0.0:{port}").as_str())?;
    eprintln!("listening on 0.0.0.0:{port}");

    // Blocking until the first handshake completes. For a long-running
    // service you would loop on `accept` and spawn a thread per peer;
    // this example is single-shot.
    let (socket, peer) = listener.accept()?;
    eprintln!("peer connected: {peer}");

    // `SrtTransport::new` wraps the connected `Socket` so it satisfies
    // the `RecvTransport` trait that `Receiver` is generic over. The
    // same wrapper also satisfies `Transport` for the send side; one
    // socket, one wrapper, both directions.
    let mut rx = Receiver::new(SrtTransport::new(socket));

    // `Receiver` implements `Iterator<Item = Result<DemuxEvent, ReceiverError>>`,
    // so `for result in &mut rx` is the idiomatic drain pattern. EOF
    // (`Ok(None)`, surfaced as iterator termination) means the
    // transport closed cleanly + the demuxer flushed its trailing PES.
    for item in &mut rx {
        match item {
            Ok(DemuxEvent::ProgramMap(m)) => {
                eprintln!(
                    "ProgramMap: program={} streams={} klv_links={}",
                    m.program_number,
                    m.streams.len(),
                    m.klv_links.len()
                );
            }
            Ok(DemuxEvent::Sample { stream, pts, .. }) => {
                eprintln!("Sample PID=0x{:04X} pts={pts}", stream.pid);
            }
            Ok(DemuxEvent::Metadata {
                stream,
                pts,
                kind,
                payload,
            }) => {
                eprintln!(
                    "Metadata PID=0x{:04X} pts={pts} kind={kind:?} bytes={}",
                    stream.pid,
                    payload.len()
                );
            }
            Ok(DemuxEvent::Discontinuity { stream, kind }) => {
                eprintln!("Discontinuity PID=0x{:04X} {kind:?}", stream.pid);
            }
            Ok(DemuxEvent::NonConformant { stream, issue }) => {
                eprintln!("NonConformant PID=0x{:04X} {issue:?}", stream.pid);
            }
            // `Transport(Closed)` is the canonical "we're done" signal.
            // It fires when the peer closes cleanly OR when libsrt has
            // decided the link is unrecoverable; in both cases the
            // demuxer has already been flushed by `Receiver` so any
            // trailing event was emitted before this point. Note: the
            // auto-flush only fires on `Closed`, not on `Broken` —
            // a broken-mid-stream link does NOT recover the trailing
            // PES (the receive thread doesn't know the stream is
            // legitimately ending vs just hiccuping).
            Err(ReceiverError::Transport(TransportError::Closed)) => {
                eprintln!("peer closed cleanly");
                break;
            }
            // Anything else is unexpected: a transport-broken state we
            // didn't initiate, or a demuxer error (strict-mode
            // rejection or a malformed PES). Print and bail rather
            // than spinning.
            Err(e) => {
                eprintln!("receiver error: {e}");
                break;
            }
        }
    }
    Ok(())
}
