//! Phase 3 Wave F Task 25 — lagging-peer behavior.

/// One slow peer (consumes broadcast slowly) should not block the muxer
/// or other peers — broadcast lag policy is drop-oldest with a counter.
///
/// Deterministically reproducing peer-lag in a unit test requires
/// carefully throttled UDP send paths or a synthetic Read implementor
/// that blocks selectively. Mark as `#[ignore]` until Wave H plumbs a
/// proper test harness; the underlying behavior is verified by
/// `spawn_peer_fanout`'s unit test `drop_counter_ticks_on_lag` in
/// `crates/tst-rtp/src/rtsp/server/fanout.rs`.
#[test]
#[ignore = "deterministic lag reproduction needs a throttled test harness; see fanout.rs::drop_counter_ticks_on_lag unit test for the underlying behavior"]
fn slow_peer_does_not_block_muxer() {
    // Placeholder.
}
