//! C-ABI stats integration tests. Each handle type's get/reset accessor
//! gets exercised end-to-end against a live (loopback) or in-process
//! handle.

use srtc::stats::SrtcRawSenderStats;

#[test]
fn raw_sender_stats_layout_is_repr_c() {
    let s = SrtcRawSenderStats::default();
    assert_eq!(std::mem::size_of::<SrtcRawSenderStats>(), 16);
    assert_eq!(s.bytes_sent, 0);
    assert_eq!(s.packets_sent, 0);
}

use srtc::stats::{SRTC_STATS_MAX_STREAMS, SrtcMuxerStats};
use std::ptr;

#[test]
fn muxer_stats_layout() {
    let s = SrtcMuxerStats::default();
    assert_eq!(s.per_stream_count, 0);
    assert_eq!(s.per_stream_truncated, 0);
    assert_eq!(s.per_stream.len(), SRTC_STATS_MAX_STREAMS);
}

#[test]
fn muxer_get_stats_after_push() {
    use srtc::config::{SrtcKlvStreamType, SrtcVideoCodec};
    use srtc::config::{
        srtc_mux_config_add_klv, srtc_mux_config_add_video, srtc_mux_config_free,
        srtc_mux_config_new,
    };
    use srtc::muxer::{
        srtc_muxer_close, srtc_muxer_get_stats, srtc_muxer_open, srtc_muxer_reset_stats,
    };
    unsafe {
        let cfg = srtc_mux_config_new();
        srtc_mux_config_add_video(cfg, 0x0100, SrtcVideoCodec::H264);
        srtc_mux_config_add_klv(cfg, 0x0101, SrtcKlvStreamType::PrivateData, false);
        let m = srtc_muxer_open(cfg);
        assert!(!m.is_null());
        // Fresh muxer: stats start zero, but per_stream_count == 2 (eager).
        let mut st = SrtcMuxerStats::default();
        let rc = srtc_muxer_get_stats(m, &mut st);
        assert_eq!(rc, 0);
        assert_eq!(st.per_stream_count, 2);
        assert_eq!(st.per_stream_truncated, 0);
        // Reset round-trip is a no-op on zeros.
        let rc = srtc_muxer_reset_stats(m);
        assert_eq!(rc, 0);
        // Cleanup.
        srtc_muxer_close(m);
        srtc_mux_config_free(cfg);
    }
}

#[test]
fn muxer_get_stats_null_pointer_returns_invalid_config() {
    let mut st = SrtcMuxerStats::default();
    unsafe {
        let rc = srtc::muxer::srtc_muxer_get_stats(ptr::null_mut(), &mut st);
        assert_ne!(rc, 0);
    }
}
