//! Verifies threshold-crossing back-pressure warns on `MuxSender`.
//!
//! The instrumentation fires `tracing::warn!` once on the Ok→Warn
//! transition (queue depth crosses 80% of `MuxerConfig::buffer_packets`)
//! and again on the Warn→Overflow transition (queue at cap, OR a push
//! that would overflow returned `MuxError::BufferFull`). It does NOT
//! re-fire on every poll once a state is already entered, and it does
//! NOT fire on recovery (Warn→Ok or Overflow→Warn). Two warns per
//! escalation cycle is the contract.
//!
//! Setup: an in-memory transport that always succeeds (so we don't
//! exercise the `pending_bytes` retry path; we want the muxer queue
//! itself to be the back-pressure source). The muxer is configured
//! with `buffer_packets = 10` (the minimum). The sample point is right
//! after `Muxer::push_*` queues TS packets and BEFORE `drain_muxer`
//! pulls them — i.e., the queue is at peak depth for that push cycle.
//!
//! Empirical packing for cap=10:
//!   - Push 1 (small ~50 B NAL): PSI(2) + video(1) = 3 queued. Ok.
//!   - Push 2 (medium ~750 B): video only (PSI suppressed at same PTS),
//!     queue rebuilt to 5. Ok.
//!   - Push 3 (large ~1100 B): queue rebuilt to 7. Ok.
//!   - Push 4 (XL ~1400 B): queue rebuilt to 8. Ratio 0.8 → Warn.
//!     **WARN #1 fires.**
//!   - Push 5 (same XL): queue rebuilt to 8. Warn→Warn is not a
//!     transition. **No warn fires.**
//!   - Push 6 (oversized ~1700 B): would need 11 packets which doesn't
//!     fit in the 10-packet cap; `push_video` returns `BufferFull`.
//!     The BufferFull signal counts as the Overflow transition.
//!     **WARN #2 fires.**
//!   - Push 7 (same oversized): also BufferFull, Overflow→Overflow is
//!     not a transition. **No warn fires.**

use tracing_test::traced_test;
use tst_core::mpegts::mux::{KlvStreamType, MuxerConfig, VideoCodec};
use tst_core::transport::{Transport, TransportError};
use tst_pipeline::MuxSender;

/// In-memory sink that succeeds on every send. Used so the muxer queue
/// drains cleanly (no `pending_bytes` retry path); the back-pressure
/// signal we want is the peak muxer-queue depth observed at the warn
/// sample point (between push and drain).
struct OkSink;
impl Transport for OkSink {
    fn send_bytes(&mut self, _: &[u8]) -> Result<(), TransportError> {
        Ok(())
    }
    fn max_payload(&self) -> usize {
        // Match the typical SRT live payload so chunks are 1316 bytes
        // (== 7 * 188 TS packets); irrelevant for the warn semantic but
        // matches real call shape.
        1316
    }
    fn close(&mut self) {}
    fn is_alive(&self) -> bool {
        true
    }
}

/// Build an Annex-B IDR NAL of approximately `body_bytes` total bytes.
/// 5 bytes of start code + nal-unit header, then `body_bytes - 5` of
/// 0xAA payload to give the muxer a known-size buffer to packetize.
fn idr_nal(body_bytes: usize) -> Vec<u8> {
    let mut v = vec![0x00, 0x00, 0x00, 0x01, 0x65];
    v.extend(std::iter::repeat_n(0xAA, body_bytes.saturating_sub(5)));
    v
}

#[traced_test]
#[test]
fn warn_fires_exactly_twice_on_back_pressure_escalation() {
    let cfg = MuxerConfig::builder()
        .add_program(1, 0x1000)
        .add_video(0x100, VideoCodec::H264)
        .add_klv(0x101, KlvStreamType::PrivateData, false)
        .pcr_pid(0x100)
        .end_program()
        .buffer_packets(10) // minimum cap; smallest cap to engineer threshold crossings
        .build()
        .expect("config builds");
    let s = MuxSender::new(OkSink, cfg).expect("sender builds");

    // Push 1: small. Empirical queue depth = 3 (PSI(2) + video(1)). Ok.
    s.send_video(&idr_nal(50), 0, true)
        .expect("push 1 fits comfortably");

    // Push 2: medium. PSI suppressed (same PTS, default 100 ms gating
    // window). Queue rebuilt to 5. Ok.
    s.send_video(&idr_nal(750), 0, false).expect("push 2 fits");

    // Push 3: large. Queue rebuilt to 7. Ok (ratio 0.7 < 0.8).
    s.send_video(&idr_nal(1100), 0, false).expect("push 3 fits");

    // Push 4: XL. Queue rebuilt to 8. Ratio 0.8 → Warn.
    // **WARN #1 expected here (Ok→Warn).**
    s.send_video(&idr_nal(1400), 0, false)
        .expect("push 4 fits at 0.8");

    // Push 5: same XL NAL. Queue depth = 8 again. State stays Warn.
    // No additional warn should fire.
    s.send_video(&idr_nal(1400), 0, false)
        .expect("push 5 fits at 0.8");

    // Push 6: oversized. ts_packets_for(~1715) = ceil(1715/184)+1 = 11
    // packets; doesn't fit in cap=10 — `push_video` returns `BufferFull`.
    // **WARN #2 (Warn→Overflow) expected here.**
    let res6 = s.send_video(&idr_nal(1700), 0, false);
    assert!(
        matches!(
            res6,
            Err(tst_pipeline::MuxSenderError::Mux(
                tst_core::error::MuxError::BufferFull { .. }
            ))
        ),
        "push 6 should hit BufferFull, got {res6:?}",
    );

    // Push 7: same shape — also BufferFull. Overflow→Overflow not a
    // transition. No additional warn.
    let _ = s.send_video(&idr_nal(1700), 0, false);

    logs_assert(|lines: &[&str]| {
        let warn_count = lines
            .iter()
            .filter(|l| l.contains("WARN") && l.contains("back-pressure"))
            .count();
        if warn_count == 2 {
            Ok(())
        } else {
            Err(format!(
                "expected exactly 2 back-pressure WARN lines, got {warn_count}; lines:\n{}",
                lines.join("\n")
            ))
        }
    });
}
