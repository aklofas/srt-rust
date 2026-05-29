#![allow(clippy::field_reassign_with_default)]

use crate::fixtures::rtsp_loopback_server::*;

#[test]
fn auto_fallback_udp_to_tcp_on_461() {
    let mut cfg = FixtureConfig::default();
    cfg.force_461_on_udp = true;
    let h = FixtureHandle::spawn(cfg);
    let url = format!("rtsp://127.0.0.1:{}/test", h.port);
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    let sdp = client.describe().unwrap();
    let session = client.setup_mp2t_auto(&sdp).unwrap();
    // After auto-fallback we should land on TCP-interleaved.
    assert_eq!(
        session.transport_kind(),
        tst_rtp::RtspTransportKind::TcpInterleaved
    );
    drop(client);
    drop(h);
}

#[test]
fn force_udp_does_not_fall_back() {
    let mut cfg = FixtureConfig::default();
    cfg.force_461_on_udp = true;
    let h = FixtureHandle::spawn(cfg);
    let url = format!("rtsp://127.0.0.1:{}/test?transport=udp", h.port);
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    let sdp = client.describe().unwrap();
    let res = client.setup_mp2t_auto(&sdp);
    assert!(matches!(
        res,
        Err(tst_rtp::RtspError::Protocol { code: 461, .. })
    ));
    drop(client);
    drop(h);
}
