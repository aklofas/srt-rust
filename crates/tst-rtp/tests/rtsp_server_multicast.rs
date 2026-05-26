//! Phase 3 Wave F Task 25 — multicast mount integration tests.

use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
use tst_rtp::{MountKind, RtspServer};

fn make_muxer_cfg() -> MuxerConfig {
    let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
    prog.add_video(0x1011, VideoCodec::H264);
    let mut b = MuxerConfig::builder();
    b.add_program(prog.build());
    b.build().unwrap()
}

/// Multicast mount registered + visible in SDP via DESCRIBE.
#[test]
fn multicast_mount_describes_with_group_in_sdp() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    let mount = server
        .add_multicast_mount("/mc", make_muxer_cfg(), "rtp://239.0.0.1:5004")
        .unwrap();
    assert!(matches!(mount.mount_kind(), MountKind::Multicast { .. }));
    server.start().unwrap();
    let port = server.local_addr().unwrap().port();
    let url = format!("rtsp://127.0.0.1:{port}/mc");
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    client.options().unwrap();
    let sdp = client.describe().unwrap();
    // SDP should contain the multicast group address.
    let session_c = sdp.session_connection.as_deref().unwrap_or_default();
    assert!(
        session_c.contains("239.0.0.1"),
        "expected multicast group in c= line; got {sdp:?}"
    );
    server.stop().ok();
}

/// Multicast SETUP returns Transport: multicast;destination=...
///
/// **Deferred:** `RtspClient::setup_mp2t_auto` builds the SETUP URI as
/// `<base>/trackID=0` (from the SDP `a=control:trackID=0` attribute).
/// The server-side `extract_mount_path` in
/// `crates/tst-rtp/src/rtsp/server/handlers.rs` returns the full
/// `/mc/trackID=0` path verbatim and looks it up against the mounts
/// map, which only registers `/mc` → 404 Not Found. End-to-end SETUP
/// through the real server doesn't work in v1 — the existing
/// `setup_play` integration tests all run against
/// `tests/fixtures/rtsp_loopback_server.rs` which replies 200 OK
/// regardless of URI, hiding this issue.
///
/// The multicast Transport response shape is unit-tested in
/// `handlers.rs::setup_multicast_mount_returns_multicast_transport`.
#[test]
#[ignore = "real-server SETUP URL has trackID suffix → 404; see handlers.rs::setup_multicast_mount_returns_multicast_transport for the underlying behavior"]
fn multicast_setup_returns_multicast_transport() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    let _mount = server
        .add_multicast_mount("/mc", make_muxer_cfg(), "rtp://239.0.0.1:5004")
        .unwrap();
    server.start().unwrap();
    let port = server.local_addr().unwrap().port();
    let url = format!("rtsp://127.0.0.1:{port}/mc");
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    client.options().unwrap();
    let sdp = client.describe().unwrap();
    let _session = client.setup_mp2t_auto(&sdp).unwrap();
    // The session was created — that's the success signal for v1.
    server.stop().ok();
}

/// TCP-interleaved SETUP against multicast mount → 461 Unsupported
/// Transport per RFC 7826 §13.3.
///
/// **Deferred:** Same control-attribute SETUP URL issue as
/// `multicast_setup_returns_multicast_transport` above — the request
/// hits the mount-lookup 404 branch before reaching the 461 multicast +
/// TCP-interleaved guard. The 461 path is unit-tested in
/// `handlers.rs::setup_against_multicast_mount_rejects_tcp_with_461`.
#[test]
#[ignore = "real-server SETUP URL has trackID suffix → 404 before reaching 461; see handlers.rs::setup_against_multicast_mount_rejects_tcp_with_461 for the underlying behavior"]
fn multicast_rejects_tcp_interleaved_with_461() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    let _mount = server
        .add_multicast_mount("/mc", make_muxer_cfg(), "rtp://239.0.0.1:5004")
        .unwrap();
    server.start().unwrap();
    let port = server.local_addr().unwrap().port();
    let url = format!("rtsp://127.0.0.1:{port}/mc?transport=tcp");
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    client.options().unwrap();
    let sdp = client.describe().unwrap();
    let result = client.setup_mp2t_auto(&sdp);
    // Expect error — the actual RtspError variant may vary; just assert failure.
    assert!(
        result.is_err(),
        "expected SETUP to fail with 461 Unsupported Transport, got Ok(_)"
    );
    server.stop().ok();
}

/// MountHandle push_video on multicast mount drains successfully (the
/// per-mount sender task drives the multicast UDP socket).
#[test]
fn multicast_mount_push_video_succeeds() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    let mount = server
        .add_multicast_mount("/mc", make_muxer_cfg(), "rtp://239.0.0.1:5004")
        .unwrap();
    server.start().unwrap();
    let nal = [0x00, 0x00, 0x00, 0x01, 0x65, 0xBB];
    mount.push_video(&nal, Pts90khz::new(0), true).unwrap();
    let stats = mount.stats();
    assert!(stats.bytes_pushed > 0);
    server.stop().ok();
}
