//! Phase 3 Wave F Task 23 — TCP-interleaved loopback test placeholders.
//!
//! TCP-interleaved RTP flow is deferred to a Wave H follow-up:
//!
//! - T17 (PLAY handler) shipped the SETUP-allocated channel pair but
//!   doesn't yet spawn `spawn_peer_fanout` for the Interleaved variant
//!   — that requires plumbing the per-session `OwnedWriteHalf` through
//!   the per-session task so the fanout's `InterleavedWriter` can share
//!   the control TCP for binary RTP frames.
//! - T20 (client-side pump) shipped the `spawn_client_pump` primitive
//!   but `RtspClient::play` does not yet call it; the producer thread
//!   feeding `RtspSession::into_recv_transport`'s mpsc placeholder is
//!   therefore not wired.
//!
//! All tests here are `#[ignore]` until those follow-ups land in
//! Wave H. The control-plane handshake (SETUP / PLAY) already returns
//! 200 today, so the test bodies are written against the eventual
//! end-state — they will exercise the real RTP byte flow once the
//! deferred wiring lands.

use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
use tst_rtp::{RtspClient, RtspServer};

fn make_muxer_cfg() -> MuxerConfig {
    let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
    prog.add_video(0x1011, VideoCodec::H264);
    let mut b = MuxerConfig::builder();
    b.add_program(prog.build());
    b.build().unwrap()
}

#[test]
#[ignore = "TCP-interleaved RTP flow deferred to Wave H (T17 fanout + T20 client pump wire-up)"]
fn client_setup_with_transport_tcp_round_trips_ts() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    let _mount = server.add_mount("/live", make_muxer_cfg()).unwrap();
    server.start().unwrap();
    let port = server.local_addr().unwrap().port();
    let url = format!("rtsp://127.0.0.1:{port}/live?transport=tcp");

    let mut client = RtspClient::connect(&url).unwrap();
    let sdp = client.describe().unwrap();
    let session = client.setup_mp2t_auto(&sdp).unwrap();
    assert_eq!(
        session.transport_kind(),
        tst_rtp::RtspTransportKind::TcpInterleaved
    );
    let _recv = session.into_recv_transport();
    client.play().unwrap();
    // Full byte-identical round-trip assertion goes here once Wave H
    // wires the producer-side spawn for the interleaved path.
}
