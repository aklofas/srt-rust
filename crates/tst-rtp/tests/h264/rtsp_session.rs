//! RTSP session integration tests for `setup_h264_auto` + `into_h264_receiver`.
//!
//! Uses the existing loopback RTSP fixture (extended with `play_data` and
//! `track_setup` fields) to serve an H.264 SDP and push canned
//! TCP-interleaved `$`-frames after PLAY.
//!
//! Tests:
//! (1) Happy path: mode-1 SDP → `setup_h264_auto` succeeds → `into_h264_receiver`
//!     receives the SPS/PPS-injected AU from a bare-IDR RTP packet.
//! (2) Mode-2 rejection: `setup_h264_auto` returns
//!     `UnsupportedPacketizationMode(2)` BEFORE any SETUP request.

use base64::Engine as _;
use tst_rtp::{ParameterSetInjection, RtspClient, RtspError};

use crate::common::packetize;
use crate::fixtures::rtsp_loopback_server::{FixtureConfig, FixtureHandle};

/// Minimal but structurally valid SPS NALU (type 7, F=0, NRI=3).
/// Bytes chosen so that `parse_sps` does not hard-reject them.  The bytes
/// match the well-known Baseline Level 3.0 sequence parameter set
/// `67 42 C0 1E D9 00 A0 46 FC B8 04 00` (a real encoder's output).
fn sps_nalu() -> Vec<u8> {
    // type 7 (SPS): 0x67 | NRI=3→0x67 (NRI occupies bits [6:5]; NRI=3 → 0x60; type=7 → 0x07; combined 0x67)
    vec![0x67u8, 0x42, 0xC0, 0x1E, 0xD9, 0x00, 0xA0, 0x46]
}

/// Minimal but structurally valid PPS NALU (type 8, F=0).
fn pps_nalu() -> Vec<u8> {
    // type 8 (PPS): 0x68
    vec![0x68u8, 0xCE, 0x38, 0x80]
}

/// Build an H.264 SDP body with `packetization-mode=<mode>` and the given
/// `sprop-parameter-sets` base64 strings.
fn h264_sdp(mode: u8, sprop_sps_b64: &str, sprop_pps_b64: &str) -> Vec<u8> {
    format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 127.0.0.1\r\n\
         s=H264 test\r\n\
         t=0 0\r\n\
         a=control:*\r\n\
         m=video 0 RTP/AVP 96\r\n\
         a=rtpmap:96 H264/90000\r\n\
         a=fmtp:96 packetization-mode={mode};sprop-parameter-sets={sprop_sps_b64},{sprop_pps_b64}\r\n\
         a=control:trackID=0\r\n"
    )
    .into_bytes()
}

/// Encode a raw RTP packet as a TCP-interleaved `$`-frame on channel 0.
fn interleaved_frame(rtp_pkt: &[u8]) -> Vec<u8> {
    // RFC 7826 §14: `0x24 <channel:u8> <length:u16 BE> <payload>`
    let len = rtp_pkt.len() as u16;
    let mut frame = vec![0x24u8, 0, (len >> 8) as u8, (len & 0xFF) as u8];
    frame.extend_from_slice(rtp_pkt);
    frame
}

/// Build the `play_data` blob: a sequence of interleaved frames.
fn build_play_data(pkts: &[Vec<u8>]) -> Vec<u8> {
    let mut buf = Vec::new();
    for pkt in pkts {
        buf.extend(interleaved_frame(pkt));
    }
    buf
}

/// Happy path: mode-1 H.264 SDP → `setup_h264_auto` → `into_h264_receiver`
/// receives the injected SPS/PPS + IDR AU.
///
/// Flow:
///  1. Fixture serves H.264 SDP (mode=1, sprop=SPS,PPS).
///  2. Client: connect (`?transport=tcp`) → describe → `setup_h264_auto` →
///     play → `into_h264_receiver(config)`.
///  3. Fixture pushes a bare-IDR single-NALU RTP packet as play_data.
///  4. Receiver: `recv_au` yields an AU whose annexb equals
///     `[SC SPS SC PPS SC IDR]` (BeforeIdr injection from the sprop cache).
#[test]
fn setup_h264_auto_mode1_roundtrip() {
    let sps = sps_nalu();
    let pps = pps_nalu();
    let sps_b64 = base64::engine::general_purpose::STANDARD.encode(&sps);
    let pps_b64 = base64::engine::general_purpose::STANDARD.encode(&pps);
    let sdp_body = h264_sdp(1, &sps_b64, &pps_b64);

    // IDR NALU to packetize and push as canned data.
    const PT: u8 = 96;
    const SSRC: u32 = 0xABCD_1234;
    let idr_nalu = vec![0x65u8, 0xAA, 0xBB, 0xCC]; // type 5 = IDR
    let aus = vec![(90_000u32, vec![idr_nalu.clone()])];
    let rtp_pkts = packetize(&aus, 1400, 1, SSRC, PT);

    // Expected Annex B: SPS + PPS + IDR (BeforeIdr injection).
    let mut expected = Vec::new();
    expected.extend_from_slice(&[0, 0, 0, 1]);
    expected.extend_from_slice(&sps);
    expected.extend_from_slice(&[0, 0, 0, 1]);
    expected.extend_from_slice(&pps);
    expected.extend_from_slice(&[0, 0, 0, 1]);
    expected.extend_from_slice(&idr_nalu);

    let play_data = build_play_data(&rtp_pkts);

    let fixture = FixtureHandle::spawn(FixtureConfig {
        sdp_body,
        play_data,
        ..FixtureConfig::default()
    });

    let url = format!("rtsp://127.0.0.1:{}/?transport=tcp", fixture.port);
    let mut client = RtspClient::connect(&url).unwrap();
    let sdp = client.describe().unwrap();
    let (session, config) = client.setup_h264_auto(&sdp).unwrap();
    // setup_h264_auto returns BeforeIdr injection by default.
    assert_eq!(
        config.parameter_set_injection,
        ParameterSetInjection::BeforeIdr
    );
    assert_eq!(config.payload_type, PT);
    assert_eq!(config.initial_parameter_sets.len(), 2);

    client.play().unwrap();
    let mut rx = session.into_h264_receiver(config);

    // Wait for the AU.
    let au = rx
        .recv_au()
        .expect("recv_au should not error")
        .expect("should get AU");
    assert_eq!(
        au.annexb, expected,
        "AU Annex B mismatch (SPS/PPS injection not applied?)"
    );
    assert!(au.key_frame, "IDR AU must have key_frame=true");
    rx.close();
}

/// Mode-2 rejection: `setup_h264_auto` must return
/// `UnsupportedPacketizationMode(2)` BEFORE sending any SETUP request.
///
/// We verify that the fixture's `setup_count` remains 0 throughout the call.
#[test]
fn setup_h264_auto_mode2_rejected_before_setup() {
    let sps = sps_nalu();
    let pps = pps_nalu();
    let sps_b64 = base64::engine::general_purpose::STANDARD.encode(&sps);
    let pps_b64 = base64::engine::general_purpose::STANDARD.encode(&pps);
    // packetization-mode=2 → must be rejected.
    let sdp_body = h264_sdp(2, &sps_b64, &pps_b64);

    let fixture = FixtureHandle::spawn(FixtureConfig {
        sdp_body,
        track_setup: true,
        ..FixtureConfig::default()
    });

    let url = format!("rtsp://127.0.0.1:{}/?transport=tcp", fixture.port);
    let mut client = RtspClient::connect(&url).unwrap();
    let sdp = client.describe().unwrap();

    let result = client.setup_h264_auto(&sdp);
    let err = match result {
        Ok(_) => panic!("setup_h264_auto with mode=2 must fail"),
        Err(e) => e,
    };
    assert!(
        matches!(err, RtspError::UnsupportedPacketizationMode(2)),
        "expected UnsupportedPacketizationMode(2), got {err:?}"
    );

    // Give the fixture a moment for any in-flight requests to arrive,
    // then verify no SETUP was sent.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let count = fixture
        .setup_count
        .load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        count, 0,
        "SETUP must NOT be sent before UnsupportedPacketizationMode is returned"
    );
}
