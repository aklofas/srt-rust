//! C-ABI URL parsing tests for `tst_mux_sender_*` (plain + managed).
//! Per spec §8.3 second paragraph (per-sender-variant roundtrip).

use std::ffi::CString;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tst_srt::ListenerBuilder;
use tstrans::config::{
    TstKlvStreamType, TstVideoCodec, tst_mux_config_add_klv_stream, tst_mux_config_add_program,
    tst_mux_config_add_video_stream, tst_mux_config_free, tst_mux_config_new,
};
use tstrans::sender::mux_sender::{
    tst_managed_mux_sender_close, tst_managed_mux_sender_open, tst_mux_sender_close,
    tst_mux_sender_open,
};

use super::last_error_msg;

#[test]
fn variant_mux_sender_open_with_url() {
    let (port_tx, port_rx) = mpsc::channel::<u16>();
    let (sid_tx, sid_rx) = mpsc::channel::<Option<String>>();

    let listener_thread = thread::spawn(move || {
        let mut listener = ListenerBuilder::new()
            .recv_timeout(Duration::from_secs(5))
            .bind("127.0.0.1:0")
            .expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        port_tx.send(port).expect("send port");
        let (accepted, _peer) = listener.accept().expect("accept");
        sid_tx
            .send(accepted.stream_id().map(str::to_string))
            .expect("send stream_id");
    });

    let port = port_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("listener did not bind in time");
    let url = CString::new(format!("srt://127.0.0.1:{port}?streamid=mux-plain")).unwrap();

    unsafe {
        let cfg = tst_mux_config_new();
        let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
        tst_mux_config_add_video_stream(cfg, prog, 0x1011, TstVideoCodec::H264);
        tst_mux_config_add_klv_stream(cfg, prog, 0x1031, TstKlvStreamType::PrivateData, false);
        let s = tst_mux_sender_open(url.as_ptr(), cfg);
        assert!(!s.is_null(), "open failed: {}", last_error_msg());

        let observed = sid_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("listener did not send stream_id in time");
        assert_eq!(
            observed.as_deref(),
            Some("mux-plain"),
            "expected stream_id 'mux-plain', got {:?}",
            observed
        );

        tst_mux_sender_close(s);
        tst_mux_config_free(cfg);
    }

    listener_thread.join().expect("listener thread panicked");
}

#[test]
fn variant_managed_mux_sender_open_with_url() {
    let (port_tx, port_rx) = mpsc::channel::<u16>();
    let (sid_tx, sid_rx) = mpsc::channel::<Option<String>>();

    let listener_thread = thread::spawn(move || {
        let mut listener = ListenerBuilder::new()
            .recv_timeout(Duration::from_secs(5))
            .bind("127.0.0.1:0")
            .expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        port_tx.send(port).expect("send port");
        let (accepted, _peer) = listener.accept().expect("accept");
        sid_tx
            .send(accepted.stream_id().map(str::to_string))
            .expect("send stream_id");
    });

    let port = port_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("listener did not bind in time");
    let url = CString::new(format!("srt://127.0.0.1:{port}?streamid=mux-managed")).unwrap();

    unsafe {
        let cfg = tst_mux_config_new();
        let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
        tst_mux_config_add_video_stream(cfg, prog, 0x1011, TstVideoCodec::H264);
        tst_mux_config_add_klv_stream(cfg, prog, 0x1031, TstKlvStreamType::PrivateData, false);
        let s = tst_managed_mux_sender_open(url.as_ptr(), cfg, std::ptr::null());
        assert!(!s.is_null(), "open failed: {}", last_error_msg());

        let observed = sid_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("listener did not send stream_id in time");
        assert_eq!(
            observed.as_deref(),
            Some("mux-managed"),
            "expected stream_id 'mux-managed', got {:?}",
            observed
        );

        tst_managed_mux_sender_close(s);
        tst_mux_config_free(cfg);
    }

    listener_thread.join().expect("listener thread panicked");
}
