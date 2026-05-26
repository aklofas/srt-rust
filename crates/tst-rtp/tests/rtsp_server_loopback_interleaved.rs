//! Phase 3 Wave F Task 23 — TCP-interleaved loopback control-plane test.
//!
//! As of Wave H Task 4 (2026-05-26) the client-side pump wire-up has
//! landed, so the control-plane handshake (CONNECT / DESCRIBE / SETUP /
//! PLAY) now passes through the activated pump:
//! `RtspClient::activate_interleaved_pump` is called at SETUP, and
//! subsequent `send_and_read` requests poll the pump's `ctrl_rx`.
//!
//! Server-side TCP-interleaved fanout is still deferred (T17 — see
//! `crates/tst-rtp/src/rtsp/server/handlers.rs::handle_play` Wave-E
//! note: "TCP-interleaved fanout deferred to Wave E; PLAY returns 200
//! but RTP won't flow"). The byte-identical round-trip assertion in
//! this file remains a TODO until the server-side fanout lands.

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
