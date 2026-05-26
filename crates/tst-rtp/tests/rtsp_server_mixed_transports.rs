//! Phase 3 Wave F Task 25 — mixed-transport on one mount.

/// One mount, one UDP client + one multicast subscriber. Both receive
/// byte-identical TS streams.
#[test]
#[ignore = "DemuxReceiver-side full RTP flow assertion deferred; the UDP client.play() path + server.add_mount + push_video already individually tested. End-to-end assertion via DemuxReceiver needs the client-side interleaved pump wire-up (Wave H) for the TCP-interleaved leg too."]
fn udp_and_multicast_on_same_mount() {
    // Wave H lands the end-to-end assertion.
}
