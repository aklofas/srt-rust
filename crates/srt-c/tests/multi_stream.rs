//! Multi-stream `mpegts::mux` fan-out via the C ABI.

use srtc::config::{
    SrtcKlvStreamType, SrtcVideoCodec, srtc_mux_config_add_klv_stream,
    srtc_mux_config_add_video_stream, srtc_mux_config_free, srtc_mux_config_new,
};
use srtc::handle::SRTC_INVALID_STREAM_HANDLE;
use srtc::muxer::{
    srtc_muxer_close, srtc_muxer_open, srtc_muxer_pull, srtc_muxer_push_video_to,
};

const NAL_SPS: &[u8] = &[
    0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xc0, 0x1e, 0xda, 0x02, 0x80, 0xf6, 0xc0,
];

#[test]
fn muxer_push_video_to_routes_to_correct_handle() {
    unsafe {
        let cfg = srtc_mux_config_new();
        let h_eo = srtc_mux_config_add_video_stream(cfg, 0x1011, SrtcVideoCodec::H264);
        let h_ir = srtc_mux_config_add_video_stream(cfg, 0x1012, SrtcVideoCodec::H264);
        let h_klv =
            srtc_mux_config_add_klv_stream(cfg, 0x1031, SrtcKlvStreamType::PrivateData, false);
        assert_ne!(h_eo, SRTC_INVALID_STREAM_HANDLE);
        assert_ne!(h_ir, SRTC_INVALID_STREAM_HANDLE);
        assert_ne!(h_klv, SRTC_INVALID_STREAM_HANDLE);

        let mux = srtc_muxer_open(cfg);
        srtc_mux_config_free(cfg);
        assert!(!mux.is_null());

        let rc_eo = srtc_muxer_push_video_to(mux, h_eo, NAL_SPS.as_ptr(), NAL_SPS.len(), 0, true);
        assert_eq!(rc_eo, 0);
        let rc_ir = srtc_muxer_push_video_to(mux, h_ir, NAL_SPS.as_ptr(), NAL_SPS.len(), 0, true);
        assert_eq!(rc_ir, 0);

        let mut buf = vec![0u8; 64 * 188];
        let n = srtc_muxer_pull(mux, buf.as_mut_ptr(), buf.len());
        assert!(n > 0, "muxer produced no output");

        srtc_muxer_close(mux);
    }
}

#[test]
fn muxer_push_video_to_invalid_handle_returns_invalid_usage() {
    unsafe {
        let cfg = srtc_mux_config_new();
        srtc_mux_config_add_video_stream(cfg, 0x1011, SrtcVideoCodec::H264);
        let mux = srtc_muxer_open(cfg);
        srtc_mux_config_free(cfg);
        assert!(!mux.is_null());

        // Handle 99 was never added.
        let rc = srtc_muxer_push_video_to(mux, 99, NAL_SPS.as_ptr(), NAL_SPS.len(), 0, true);
        assert_eq!(rc, -9 /* SRTC_E_INVALID_USAGE */);

        srtc_muxer_close(mux);
    }
}
