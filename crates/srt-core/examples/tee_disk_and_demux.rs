//! Tee MPEG-TS bytes to disk while typed `DemuxEvent`s flow concurrently.
//!
//! Why this exists: a common ISR workflow is "save the raw `.ts`
//! capture AND parse for KLV/video metadata in one pass." Doing those
//! as two separate passes wastes I/O; doing them in two threads with
//! the demuxer reading bytes off a channel is more code than the
//! problem warrants. `DemuxReceiver::add_byte_sink` is the canonical answer:
//! register a fan-out callback that sees every 188-byte TS packet
//! before the demuxer parses it, and the typed event stream still
//! comes out the iterator unchanged. One byte stream, two consumers,
//! one thread.
//!
//! Usage: `cargo run --example tee_disk_and_demux -- <input.ts> <output.ts>`
//!
//! This example uses a hand-rolled `RecvTransport` over a `.ts` file so
//! it's runnable without a live SRT publisher. In production you would
//! plug a real `SrtTransport` in instead — the byte-sink mechanism
//! works identically. See `srt_recv_typed.rs` for the SRT side.
//!
//! What to look for in the output: the output `.ts` file is byte-for-
//! byte the input file (rounded to 188-byte TS packet boundaries; the
//! demuxer pulls packet-aligned chunks from the transport via
//! `Receiver`). The `samples=N` count tells you the demux side ran
//! to completion in lock-step with the disk write — both observed the
//! same byte stream.

use srt_core::mpegts::demux::DemuxEvent;
use srt_core::pipeline::TransportError;
use srt_core::pipeline::{DemuxReceiver, RecvTransport};
use std::collections::VecDeque;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::sync::{Arc, Mutex};

/// Minimal `RecvTransport` that hands out pre-chunked byte slices and
/// returns `Closed` once the queue is empty. Useful for offline replay
/// of `.ts` captures through the same `DemuxReceiver` plumbing that
/// `SrtTransport` uses live.
///
/// In a real consumer you'd typically use `SrtTransport::new(socket)`
/// instead of rolling your own. This impl exists so the example is
/// runnable without a network peer.
struct FileTransport {
    chunks: VecDeque<Vec<u8>>,
}

impl RecvTransport for FileTransport {
    fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        match self.chunks.pop_front() {
            Some(c) => {
                // Cap the copy at `buf.len()` even though our chunks
                // are sized to fit. `DemuxReceiver` always passes a buffer
                // sized at least `max_payload()`, but defensive
                // bookkeeping is cheap and keeps the impl honest.
                let n = c.len().min(buf.len());
                buf[..n].copy_from_slice(&c[..n]);
                Ok(n)
            }
            // Empty queue is the canonical end-of-stream signal.
            // `DemuxReceiver` reacts to `Closed` by flushing the demuxer's
            // partial PES (catches the trailing AU) and then returning
            // `Ok(None)` from the iterator. This is the contract the
            // example relies on.
            None => Err(TransportError::Closed),
        }
    }

    fn max_payload(&self) -> usize {
        // 1316 = the SRT live-mode default `SRTO_PAYLOADSIZE`. It's
        // not arbitrary — 1316 = 7 × 188, where 188 is the MPEG-TS
        // packet size, so SRT's payload exactly fits seven TS packets
        // with no waste. We chunk our file at the same boundary
        // below so this transport behaves like a real SRT link's
        // recv pattern.
        1316
    }

    fn is_alive(&self) -> bool {
        !self.chunks.is_empty()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: tee_disk_and_demux <input.ts> <output.ts>");
        std::process::exit(2);
    }
    let input = &args[1];
    let output = &args[2];
    let bytes = fs::read(input)?;

    // Pretend the bytes arrived in 1316-byte SRT packets. This makes
    // the offline replay match the chunk granularity that a live
    // `SrtTransport` would produce, so the byte-sink observation
    // pattern below is identical to what you'd see in production.
    let mut chunks = VecDeque::new();
    for c in bytes.chunks(1316) {
        chunks.push_back(c.to_vec());
    }
    let mut rx = DemuxReceiver::new(FileTransport { chunks });

    // The output writer is shared via `Arc<Mutex<...>>` so the closure
    // can capture-by-clone and live longer than the call stack
    // (`add_byte_sink` takes `Box<dyn FnMut(&[u8]) + Send>` — it
    // demands `Send` so the same `DemuxReceiver` could be moved to a
    // worker thread later). For this single-threaded example the
    // lock contention is zero.
    let writer = Arc::new(Mutex::new(File::create(output)?));
    let writer_cl = writer.clone();

    // Byte-sink contract (full version in
    // `crates/srt-core/src/pipeline/receiver.rs` rustdoc):
    //
    // - Called once per TS packet (188 bytes, NOT 1316). The receiver
    //   pulls 1316-byte SRT messages from the transport and breaks
    //   them down to TS-packet alignment via `Receiver` before the
    //   sink fires.
    // - Multiple sinks fire in registration order, all before the
    //   demuxer parses the packet. So a sink that wants "the bytes
    //   exactly as they arrived on the wire, before any
    //   interpretation" is in the right place.
    // - The slice is valid only for the duration of the call. Copy
    //   bytes into an owned buffer if they need to outlive the
    //   callback. (Here we pass straight to `write_all` which
    //   completes synchronously, so no copy is needed.)
    // - Sinks must not panic — a panic unwinds through `recv_event`
    //   and aborts the demuxer. Wrap fallible work in your own
    //   error handling, never `.expect()` inside a sink in
    //   production.
    // - Sink runs synchronously on the receive thread. For high-
    //   throughput workflows or expensive work (encryption,
    //   forwarding to a remote service) push to a channel and let
    //   a worker thread do the slow work. For plain disk I/O the
    //   synchronous write is fine — the OS handles buffering.
    rx.add_byte_sink(Box::new(move |pkt: &[u8]| {
        writer_cl
            .lock()
            .expect("writer mutex poisoned")
            .write_all(pkt)
            .expect("write to output file");
    }));

    let mut samples = 0usize;
    let mut metadata = 0usize;
    // Clean EOF is iterator termination — `DemuxReceiver::recv_event`
    // translates `TransportError::Closed` into `Ok(None)` after
    // auto-flushing the demuxer, and the `Iterator` impl turns that
    // into `None`. Any `Err` here is a real error (a `Demux` strict
    // rejection or malformed PES on corrupt input).
    for item in &mut rx {
        match item {
            Ok(DemuxEvent::Sample { .. }) => samples += 1,
            Ok(DemuxEvent::Metadata { .. }) => metadata += 1,
            Ok(_) => {}
            Err(e) => {
                eprintln!("receiver error: {e}");
                break;
            }
        }
    }

    // Flush + drop the writer explicitly. `Drop` would do this
    // implicitly but a stray `Result` from `flush` would be silently
    // ignored; the explicit form surfaces it.
    writer.lock().expect("writer mutex poisoned").flush()?;
    eprintln!("wrote {output} from {input}; samples={samples} metadata={metadata}");
    Ok(())
}
