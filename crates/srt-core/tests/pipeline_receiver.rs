// crates/srt-core/tests/pipeline_receiver.rs
//! Pipeline integration: drive `Receiver` with an in-memory `CannedTransport`.
//!
//! These tests verify the composition layer rather than the mux or demux
//! internals in isolation. `CannedTransport` replays pre-muxed TS bytes
//! as if they arrived over a live SRT connection.

use std::collections::VecDeque;

use srt_core::mpegts::demux::DemuxEvent;
use srt_core::mpegts::mux::{ConfigBuilder, KlvStreamType, Muxer, VideoCodec as MuxVideoCodec};
use srt_core::pipeline::Receiver;
use srt_core::pipeline::recv_transport::RecvTransport;
use srt_core::pipeline::transport::TransportError;

// ---------------------------------------------------------------------------
// CannedTransport — replay a queue of byte chunks, then signal Closed.
// ---------------------------------------------------------------------------

/// In-memory `RecvTransport` mock. Each `recv_bytes` call pops one chunk from
/// the queue and copies it into the caller's buffer. When the queue is empty,
/// returns `TransportError::Closed`.
///
/// This mirrors the SRT live-mode framing contract: one `recv_bytes` call
/// returns one message, never more.
struct CannedTransport {
    chunks: VecDeque<Vec<u8>>,
}

impl RecvTransport for CannedTransport {
    fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        match self.chunks.pop_front() {
            Some(c) => {
                let n = c.len().min(buf.len());
                buf[..n].copy_from_slice(&c[..n]);
                Ok(n)
            }
            None => Err(TransportError::Closed),
        }
    }

    fn max_payload(&self) -> usize {
        // 7 × 188 = 1316 bytes — the standard SRT TS payload size.
        1316
    }

    fn is_alive(&self) -> bool {
        !self.chunks.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Helper: mux one H.264 AU and drain all output chunks.
// ---------------------------------------------------------------------------

/// Build a minimal H.264 Annex-B AU: AUD (nal_type=9) byte.
fn minimal_h264_au() -> Vec<u8> {
    vec![0x00, 0x00, 0x00, 0x01, 0x09, 0x10]
}

/// Build a minimal KLV blob with a valid SMPTE UL prefix so the demuxer
/// recognizes it as ST 0601 async KLV. Matches the pattern used in
/// `mpegts_demux.rs`.
fn minimal_klv() -> Vec<u8> {
    // 16-byte SMPTE UL + BER-short length + minimal body (tag 2, 8 zero bytes).
    let body: &[u8] = &[2u8, 8, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut out = Vec::with_capacity(17 + body.len());
    out.extend_from_slice(&[
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
        0x00,
    ]);
    out.push(body.len() as u8);
    out.extend_from_slice(body);
    out
}

/// Drain all output from a `Muxer` into a queue of chunks, each at most
/// `chunk_size` bytes, and return both the queue and the total byte count.
fn drain_to_chunks(mux: &mut Muxer, chunk_size: usize) -> (VecDeque<Vec<u8>>, usize) {
    let mut chunks = VecDeque::new();
    let mut buf = vec![0u8; chunk_size];
    let mut total = 0;
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        total += n;
        chunks.push_back(buf[..n].to_vec());
    }
    (chunks, total)
}

/// Build a muxer preloaded with enough frames to guarantee the TsReceiver sync
/// state machine can lock.
///
/// The TsReceiver `Syncer` requires 4 back-to-back 0x47 sync bytes spaced
/// 188 bytes apart before it will emit packets — that means it needs at least
/// 5 * 188 + 1 = 941 bytes of contiguous TS data in its internal buffer before
/// transitioning from VERIFY to LOCKED.
///
/// Strategy: use `psi_interval_ms(10)` (the minimum allowed, 900 ticks at
/// 90 kHz) so PAT + PMT are re-emitted on every push. Then push 3 video + KLV
/// frames. Each PSI interval boundary emits 2 more packets, so after 3 pushes
/// we get PAT+PMT (×3) + 3 video + 3 KLV ≥ 9 packets = 1692 bytes — well
/// above the 941-byte lock threshold. A single 1316-byte pull chunk then
/// contains ≥ 7 packets (or two pulls totalling ≥ 9 packets), letting the
/// syncer lock and emit events.
fn build_and_preload_muxer() -> Muxer {
    let cfg = ConfigBuilder::default()
        .add_video(0x100, MuxVideoCodec::H264)
        .add_klv(0x101, KlvStreamType::PrivateData, false)
        .psi_interval_ms(10)
        .build()
        .unwrap();
    let mut m = Muxer::new(cfg).unwrap();
    let au = minimal_h264_au();
    let klv = minimal_klv();
    // Push 3 frames at PTS 0 / 9001 / 18002 (each ≥ 9000 ticks = 10 ms apart
    // at 90 kHz) so the PSI re-emission threshold is crossed on every push.
    for i in 0i64..3 {
        let pts = i * 9001;
        m.push_video(&au, pts, true).unwrap();
        m.push_klv(&klv, pts).unwrap();
    }
    m
}

// ---------------------------------------------------------------------------
// Test 1: Receiver emits at least a ProgramMap event for a well-formed stream.
// ---------------------------------------------------------------------------

/// Mux several H.264 frames, replay them through `CannedTransport`, and confirm
/// the `Receiver` emits a `ProgramMap` event. This exercises the full
/// composition path: transport → TsReceiver sync → Demuxer → event queue.
///
/// Multiple frames are required because the `TsReceiver` sync state machine
/// needs at least 941 bytes of contiguous TS data before it will lock and emit
/// packets. See `build_and_preload_muxer` for the exact strategy.
#[test]
fn receiver_emits_events_through_canned_transport() {
    let mut m = build_and_preload_muxer();

    let (chunks, _) = drain_to_chunks(&mut m, 1316);

    let mut rx = Receiver::new(CannedTransport { chunks });
    let mut saw_pmap = false;
    for item in &mut rx {
        let e = item.unwrap();
        if matches!(e, DemuxEvent::ProgramMap(_)) {
            saw_pmap = true;
        }
    }
    assert!(saw_pmap, "expected at least one ProgramMap event");
}

// ---------------------------------------------------------------------------
// Test 2: Byte sinks see every TS byte pulled from the transport.
// ---------------------------------------------------------------------------

/// Confirms that `add_byte_sink` receives every TS packet byte before the
/// demuxer processes it. The total bytes seen by the sink must equal the total
/// bytes produced by the muxer (within ±188 bytes to account for the
/// TsReceiver syncer's internal packet alignment buffer, which may withhold
/// up to one packet until sync is confirmed).
///
/// The ±188 window is intentionally loose: `TsReceiver` may buffer one packet
/// internally during VERIFY-phase sync before emitting it, so the sink can
/// see up to one packet fewer than `total`. The upper bound guards against
/// double-delivery bugs.
#[test]
fn byte_sinks_see_every_chunk() {
    use std::sync::{Arc, Mutex};

    let mut m = build_and_preload_muxer();

    let (chunks, total) = drain_to_chunks(&mut m, 1316);

    let mut rx = Receiver::new(CannedTransport { chunks });

    // Shared counter: the sink accumulates bytes across all packets.
    let captured = Arc::new(Mutex::new(0usize));
    let cap = captured.clone();
    rx.add_byte_sink(Box::new(move |b| {
        *cap.lock().unwrap() += b.len();
    }));

    // Drive the receiver to EOF. CannedTransport only emits Closed (never
    // Broken), so any Err here would be a Demux strict-mode rejection — none
    // are expected on a clean stream. The `is_some` discard pattern is safe
    // *here*; production code should match each `Item` and surface errors.
    while rx.next().is_some() {}

    let saw = *captured.lock().unwrap();
    assert!(
        saw >= total.saturating_sub(188) && saw <= total + 188,
        "byte sink saw {saw} bytes; expected {total} ± 188"
    );
}
