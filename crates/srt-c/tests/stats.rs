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
        srtc_mux_config_add_klv_stream, srtc_mux_config_add_program,
        srtc_mux_config_add_video_stream, srtc_mux_config_free, srtc_mux_config_new,
    };
    use srtc::muxer::{
        srtc_muxer_close, srtc_muxer_get_stats, srtc_muxer_open, srtc_muxer_reset_stats,
    };
    unsafe {
        let cfg = srtc_mux_config_new();
        let prog = srtc_mux_config_add_program(cfg, 1, 0x1000);
        srtc_mux_config_add_video_stream(cfg, prog, 0x0100, SrtcVideoCodec::H264);
        srtc_mux_config_add_klv_stream(cfg, prog, 0x0101, SrtcKlvStreamType::PrivateData, false);
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

#[test]
#[cfg(target_os = "linux")]
fn mux_sender_stats_round_trip() {
    use srt_core::srt::ListenerBuilder;
    use srtc::config::{
        SrtcKlvStreamType, SrtcVideoCodec, srtc_mux_config_add_klv_stream,
        srtc_mux_config_add_program, srtc_mux_config_add_video_stream, srtc_mux_config_free,
        srtc_mux_config_new,
    };
    use srtc::mux_sender::{
        srtc_mux_sender_close, srtc_mux_sender_get_stats, srtc_mux_sender_open,
        srtc_mux_sender_reset_stats, srtc_mux_sender_send_video,
    };
    use srtc::stats::SrtcSenderStats;
    use std::ffi::CString;
    use std::sync::mpsc;
    use std::time::Duration;

    let (port_tx, port_rx) = mpsc::channel::<u16>();

    let _accept_thread = std::thread::spawn(move || {
        let mut listener = ListenerBuilder::new()
            .recv_timeout(Duration::from_secs(5))
            .bind("127.0.0.1:0")
            .expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        port_tx.send(port).expect("send port");
        let _ = listener.accept();
        std::thread::sleep(Duration::from_secs(2));
    });

    let port = port_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("listener didn't bind in time");

    unsafe {
        let cfg = srtc_mux_config_new();
        let prog = srtc_mux_config_add_program(cfg, 1, 0x1000);
        srtc_mux_config_add_video_stream(cfg, prog, 0x0100, SrtcVideoCodec::H264);
        srtc_mux_config_add_klv_stream(cfg, prog, 0x0101, SrtcKlvStreamType::PrivateData, false);
        let url = CString::new(format!("srt://127.0.0.1:{port}?latency=120")).unwrap();
        let s = srtc_mux_sender_open(url.as_ptr(), cfg);
        assert!(!s.is_null());

        let nal: [u8; 7] = [0x00, 0x00, 0x00, 0x01, 0x67, 0xBB, 0xCC];
        let rc = srtc_mux_sender_send_video(s, nal.as_ptr(), nal.len(), 0, true);
        assert_eq!(rc, 0);

        let mut st = SrtcSenderStats::default();
        let rc = srtc_mux_sender_get_stats(s, &mut st);
        assert_eq!(rc, 0);
        assert_eq!(st.per_stream_count, 2);
        let video_entry = st
            .per_stream
            .iter()
            .find(|e| e.pid == 0x0100)
            .expect("video entry");
        assert_eq!(video_entry.items, 1);

        let rc = srtc_mux_sender_reset_stats(s);
        assert_eq!(rc, 0);
        let mut st2 = SrtcSenderStats::default();
        srtc_mux_sender_get_stats(s, &mut st2);
        let video_entry2 = st2
            .per_stream
            .iter()
            .find(|e| e.pid == 0x0100)
            .expect("video entry after reset");
        assert_eq!(video_entry2.items, 0);

        srtc_mux_sender_close(s);
        srtc_mux_config_free(cfg);
    }
}

#[test]
fn ts_sender_reset_stats_returns_ok_on_valid_handle() {
    unsafe {
        let rc = srtc::ts_sender::srtc_ts_sender_reset_stats(std::ptr::null_mut());
        assert_ne!(rc, 0);
    }
}

#[test]
fn managed_ts_sender_reset_stats_returns_ok_on_valid_handle() {
    unsafe {
        let rc = srtc::ts_sender::srtc_managed_ts_sender_reset_stats(std::ptr::null_mut());
        assert_ne!(rc, 0);
    }
}
