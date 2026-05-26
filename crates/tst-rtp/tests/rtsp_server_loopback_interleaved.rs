//! Phase 3 Wave H Task 1 — server-side TCP-interleaved loopback.
//!
//! After T1 the per-session TCP is split at `handle_connection_inner`
//! and the write half is shared with the per-peer fanout task, so
//! `handle_play` for the `TcpInterleaved` transport branch spawns the
//! fanout instead of returning 200 with a `tracing::warn`. This file's
//! test exercises the server-side control-plane (SETUP + PLAY both
//! return 200 against a TCP-transport mount); the byte-level
//! end-to-end assertion lives in `rtsp_client_interleaved_e2e.rs` and
//! still depends on T4 (client-side pump wire-up).

use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
use tst_rtp::{RtspClient, RtspServer};

fn make_muxer_cfg() -> MuxerConfig {
    let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
    prog.add_video(0x1011, VideoCodec::H264);
    let mut b = MuxerConfig::builder();
    b.add_program(prog.build());
    b.build().unwrap()
}

/// Server accepts a `?transport=tcp` client through PLAY without
/// erroring. The fanout task is now spawned on the interleaved branch
/// (T1) — it'll sit on `rx.recv()` waiting for frames that the test
/// doesn't push, which is the success path here.
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
    // Full byte-identical round-trip assertion lives in
    // `rtsp_client_interleaved_e2e.rs` (gated on T4 client pump
    // wire-up).
}
