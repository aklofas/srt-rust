//! Phase 3 Wave F Task 26 — verification of Phase 2 deferred fix 1
//! (TCP-interleaved producer thread wiring).
//!
//! All tests `#[ignore]` until Wave H lands the wire-up of
//! [`spawn_client_pump`](tst_rtp::rtsp::client::interleaved_pump::spawn_client_pump)
//! into [`RtspClient::play`](tst_rtp::RtspClient::play). The primitive
//! itself is tested by the unit tests inside
//! `crates/tst-rtp/src/rtsp/client/interleaved_pump.rs` (Task 20); the
//! server-side mirror pump is wired into per-session task already (Task
//! 19). The remaining gap is client-side `RtspClient::play` -> spawn
//! pump and route the produced `mpsc::Receiver<Bytes>` into
//! `RtpRecvTransport::from_mpsc_placeholder` (currently a never-fed
//! channel — see `crates/tst-rtp/src/rtsp/client/session.rs` line ~100).

use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
use tst_rtp::{RtspClient, RtspServer};

fn make_muxer_cfg() -> MuxerConfig {
    let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
    prog.add_video(0x1011, VideoCodec::H264);
    let mut b = MuxerConfig::builder();
    b.add_program(prog.build());
    b.build().unwrap()
}

/// `rtsp:// ?transport=tcp` end-to-end: [`RtspClient::play`] succeeds AND
/// the underlying [`RtpRecvTransport`](tst_rtp::RtpRecvTransport)
/// actually receives bytes. Phase 2's
/// [`RtpRecvTransport::from_mpsc_placeholder`](tst_rtp::RtpRecvTransport)
/// returned an unfed channel; this test verifies the Wave H wire-up
/// bridges the
/// [`InterleavedReader`](tst_rtp::InterleavedReader)
/// into that channel.
#[test]
#[ignore = "client-side interleaved pump wire-up deferred to Wave H follow-up — \
             spawn_client_pump primitive exists at \
             crates/tst-rtp/src/rtsp/client/interleaved_pump.rs but \
             RtspClient::play doesn't spawn it yet, and \
             RtspSession::into_recv_transport hands the consumer a \
             never-fed channel for TcpInterleaved"]
fn tcp_interleaved_end_to_end_round_trips_ts_bytes() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    let _mount = server.add_mount("/live", make_muxer_cfg()).unwrap();
    server.start().unwrap();
    let port = server.local_addr().unwrap().port();
    let url = format!("rtsp://127.0.0.1:{port}/live?transport=tcp");

    let mut client = RtspClient::connect(&url).unwrap();
    let sdp = client.describe().unwrap();
    let session = client.setup_mp2t_auto(&sdp).unwrap();
    let _recv = session.into_recv_transport();
    client.play().unwrap();

    // Wave H: push synthetic NAL into mount, run DemuxReceiver against
    // recv, assert byte-identical TS bytes received.
    server.stop().ok();
}
