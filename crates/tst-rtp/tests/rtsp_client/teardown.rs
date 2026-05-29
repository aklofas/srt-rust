use crate::fixtures::rtsp_loopback_server::*;

#[test]
fn drop_sends_teardown_best_effort() {
    let h = FixtureHandle::spawn(FixtureConfig::default());
    let url = format!("rtsp://127.0.0.1:{}/test", h.port);
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    let sdp = client.describe().unwrap();
    let _session = client.setup_mp2t_auto(&sdp).unwrap();
    // Drop without explicit teardown — Drop impl should send TEARDOWN.
    drop(client);
    // No assertion — drop completes without panicking.
    drop(h);
}

#[test]
fn explicit_teardown_clears_session() {
    let h = FixtureHandle::spawn(FixtureConfig::default());
    let url = format!("rtsp://127.0.0.1:{}/test", h.port);
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    let sdp = client.describe().unwrap();
    let _s = client.setup_mp2t_auto(&sdp).unwrap();
    client.teardown().unwrap();
    drop(client);
    drop(h);
}
