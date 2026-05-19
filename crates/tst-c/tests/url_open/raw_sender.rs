//! C-ABI URL parsing tests for `tst_raw_sender_*` (plain + managed).
//! Per spec §8.3 second paragraph (per-sender-variant roundtrip).

use std::ffi::CString;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tst_srt::ListenerBuilder;
use tstrans::config::{tst_raw_sender_config_free, tst_raw_sender_config_new};
use tstrans::sender::raw_sender::{
    tst_managed_raw_sender_close, tst_managed_raw_sender_open, tst_raw_sender_close,
    tst_raw_sender_open,
};

use super::last_error_msg;

#[test]
fn variant_raw_sender_open_with_url() {
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
    let url = CString::new(format!("srt://127.0.0.1:{port}?streamid=raw-plain")).unwrap();

    unsafe {
        let cfg = tst_raw_sender_config_new();
        let s = tst_raw_sender_open(url.as_ptr(), cfg);
        assert!(!s.is_null(), "open failed: {}", last_error_msg());

        let observed = sid_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("listener did not send stream_id in time");
        assert_eq!(
            observed.as_deref(),
            Some("raw-plain"),
            "expected stream_id 'raw-plain', got {:?}",
            observed
        );

        tst_raw_sender_close(s);
        tst_raw_sender_config_free(cfg);
    }

    listener_thread.join().expect("listener thread panicked");
}

#[test]
fn variant_managed_raw_sender_open_with_url() {
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
    let url = CString::new(format!("srt://127.0.0.1:{port}?streamid=raw-managed")).unwrap();

    unsafe {
        let cfg = tst_raw_sender_config_new();
        let s = tst_managed_raw_sender_open(url.as_ptr(), cfg, std::ptr::null());
        assert!(!s.is_null(), "open failed: {}", last_error_msg());

        let observed = sid_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("listener did not send stream_id in time");
        assert_eq!(
            observed.as_deref(),
            Some("raw-managed"),
            "expected stream_id 'raw-managed', got {:?}",
            observed
        );

        tst_managed_raw_sender_close(s);
        tst_raw_sender_config_free(cfg);
    }

    listener_thread.join().expect("listener thread panicked");
}
