mod fixtures;
use fixtures::rtsp_loopback_server::*;

#[test]
fn setup_play_udp_succeeds() {
    let h = FixtureHandle::spawn(FixtureConfig::default());
    let url = format!("rtsp://127.0.0.1:{}/test", h.port);
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    let opts = client.options().unwrap();
    assert!(opts.public_methods.contains(&"PLAY".to_string()));
    let sdp = client.describe().unwrap();
    let session = client.setup_mp2t_auto(&sdp).unwrap();
    assert_eq!(session.transport_kind(), tst_rtp::RtspTransportKind::Udp);
    let info = client.play().unwrap();
    assert_eq!(info.seq, Some(1234));
    client.teardown().unwrap();
    drop(client);
    drop(h);
}

#[test]
fn setup_play_tcp_interleaved_succeeds() {
    let h = FixtureHandle::spawn(FixtureConfig::default());
    let url = format!("rtsp://127.0.0.1:{}/test?transport=tcp", h.port);
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    let _opts = client.options().unwrap();
    let sdp = client.describe().unwrap();
    let session = client.setup_mp2t_auto(&sdp).unwrap();
    assert_eq!(
        session.transport_kind(),
        tst_rtp::RtspTransportKind::TcpInterleaved
    );
    client.play().unwrap();
    client.teardown().unwrap();
    drop(client);
    drop(h);
}
