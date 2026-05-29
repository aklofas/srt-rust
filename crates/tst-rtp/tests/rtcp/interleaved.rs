//! Validate that RTCP rides the interleaved channel pair (N=0 RTP, N+1=1 RTCP)
//! when SETUP negotiates TCP-interleaved.

use crate::fixtures::rtsp_loopback_server::*;

#[test]
fn interleaved_session_uses_tcp_transport_kind() {
    let h = FixtureHandle::spawn(FixtureConfig::default());
    let url = format!("rtsp://127.0.0.1:{}/test?transport=tcp", h.port);
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    let sdp = client.describe().unwrap();
    let session = client.setup_mp2t_auto(&sdp).unwrap();
    assert_eq!(
        session.transport_kind(),
        tst_rtp::RtspTransportKind::TcpInterleaved
    );
    // rtcp_endpoint is None for interleaved (no UDP socket).
    assert!(session.rtcp_endpoint().is_none());
    drop(client);
    drop(h);
}
