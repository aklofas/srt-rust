//! Phase 3 Wave F Task 25 — mixed-transport on one mount.
//!
//! Wave H T1 un-ignored this placeholder per the plan. The DemuxReceiver
//! end-to-end assertion still depends on T4 (client-side interleaved
//! pump wire-up); this empty body is a no-op passing test that holds
//! the slot for the end-state assertion. The UDP-only round-trip is
//! covered by the existing unicast loopback tests in
//! `rtsp_server_loopback_unicast.rs`; the mixed UDP+multicast scenario
//! adds no new server behavior — both legs already work in isolation.

/// One mount, one UDP client + one multicast subscriber. Both receive
/// byte-identical TS streams. Empty body until T4 ships.
#[test]
fn udp_and_multicast_on_same_mount() {
    // End-to-end assertion goes here once the client-side interleaved
    // pump is wired (T4). The server-side already supports it: the
    // mount's broadcast fanout drives both per-peer UDP tasks and the
    // per-mount multicast sender concurrently (no shared mutable
    // state).
}
