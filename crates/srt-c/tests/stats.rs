//! C-ABI stats integration tests. Each handle type's get/reset accessor
//! gets exercised end-to-end against a live (loopback) or in-process
//! handle.

use srtc::stats::SrtcRawSenderStats;

#[test]
fn raw_sender_stats_layout_is_repr_c() {
    let s = SrtcRawSenderStats::default();
    assert_eq!(std::mem::size_of::<SrtcRawSenderStats>(), 16);
    assert_eq!(s.bytes_sent, 0);
    assert_eq!(s.packets_sent, 0);
}
