//! Validate that RTCP rides the SETUP-negotiated UDP port pair when
//! the transport is UDP.

mod fixtures;
use fixtures::rtsp_loopback_server::*;

#[test]
fn rtcp_endpoint_extracted_from_session() {
    let h = FixtureHandle::spawn(FixtureConfig::default());
    let url = format!("rtsp://127.0.0.1:{}/test", h.port);
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    let sdp = client.describe().unwrap();
    let session = client.setup_mp2t_auto(&sdp).unwrap();
    // For UDP transport, rtcp_endpoint should be Some(host:server_port+1).
    let rtcp = session.rtcp_endpoint();
    assert!(rtcp.is_some());
    let addr = rtcp.unwrap();
    assert_eq!(addr.port(), 6971); // fixture's server_port range is 6970-6971
    drop(client);
    drop(h);
}
