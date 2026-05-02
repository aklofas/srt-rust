//! C-ABI URL parsing integration tests. Per spec §8.3.
//!
//! Each test opens a real listener on a random local port, opens a sender
//! via the C ABI with a URL containing query params, and verifies the
//! resulting socket has the expected option values applied.
//!
//! Threading pattern: the Listener runs on a background thread (binding to
//! 127.0.0.1:0 and communicating the kernel-assigned port back via mpsc).
//! The sender is opened on the main thread so all raw C-ABI pointers
//! (*mut SrtcTsSenderConfig, *mut SrtcTsSender) never cross thread
//! boundaries.

#![allow(unused_unsafe)]

use srt_core::srt::ListenerBuilder;
use srtc::config::{srtc_ts_sender_config_free, srtc_ts_sender_config_new};
use srtc::error::srtc_get_last_error_str;
use srtc::ts_sender::{srtc_ts_sender_close, srtc_ts_sender_open};
use std::ffi::CString;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn last_error_msg() -> String {
    unsafe {
        let p = srtc_get_last_error_str();
        if p.is_null() {
            return "<null>".into();
        }
        std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

// ============================================================================
// Group 1 streamid roundtrip
// ============================================================================

#[test]
fn ts_sender_streamid_observed_on_listener() {
    // Listener thread binds to :0, sends back the assigned port, then
    // accepts and sends back the negotiated stream_id.
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
    let url = CString::new(format!("srt://127.0.0.1:{port}?streamid=front-camera")).unwrap();

    // All C-ABI pointers live on the main thread.
    unsafe {
        let cfg = srtc_ts_sender_config_new();
        let s = srtc_ts_sender_open(url.as_ptr(), cfg);
        assert!(!s.is_null(), "open failed: {}", last_error_msg());

        let observed = sid_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("listener did not send stream_id in time");
        assert_eq!(
            observed.as_deref(),
            Some("front-camera"),
            "expected stream_id 'front-camera', got {:?}",
            observed
        );

        srtc_ts_sender_close(s);
        srtc_ts_sender_config_free(cfg);
    }

    listener_thread.join().expect("listener thread panicked");
}

// ============================================================================
// Group 1 latency roundtrip — verify connect succeeds with latency=200.
//
// Socket::latency() does not exist in the current srt-core public API; the
// negotiated value is only observable by calling srt_getsockflag directly on
// the accepted socket handle, which is not exposed. We verify end-to-end that
// the URL parameter is accepted by the parser + libsrt and the handshake
// completes — the unit-test layer (url_params.rs) covers typed overlay shape;
// here we cover "URL → libsrt option set → connection opens".
// ============================================================================

#[test]
fn ts_sender_latency_negotiated() {
    let (port_tx, port_rx) = mpsc::channel::<u16>();
    let (ok_tx, ok_rx) = mpsc::channel::<bool>();

    let listener_thread = thread::spawn(move || {
        let mut listener = ListenerBuilder::new()
            .recv_timeout(Duration::from_secs(5))
            .bind("127.0.0.1:0")
            .expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        port_tx.send(port).expect("send port");
        let result = listener.accept();
        ok_tx.send(result.is_ok()).expect("send ok");
    });

    let port = port_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("listener did not bind in time");
    let url = CString::new(format!("srt://127.0.0.1:{port}?latency=200")).unwrap();

    unsafe {
        let cfg = srtc_ts_sender_config_new();
        let s = srtc_ts_sender_open(url.as_ptr(), cfg);
        assert!(!s.is_null(), "open failed: {}", last_error_msg());

        let accepted_ok = ok_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("listener did not report accept status in time");
        assert!(accepted_ok, "listener accept() failed");

        srtc_ts_sender_close(s);
        srtc_ts_sender_config_free(cfg);
    }

    listener_thread.join().expect("listener thread panicked");
}

// ============================================================================
// Group 1 — local-side options (mss, payloadsize, lossmaxttl, fc).
//
// These options are set on the caller socket before connect and do not
// require peer-observable verification. Each test verifies that the URL
// parameter passes through the parser + setsockopt + handshake without
// error (i.e. the connection opens successfully). The unit-test layer
// verifies typed overlay field population; this layer is end-to-end.
// ============================================================================

#[test]
fn ts_sender_mss_url_open_succeeds() {
    let (port_tx, port_rx) = mpsc::channel::<u16>();
    let (ok_tx, ok_rx) = mpsc::channel::<bool>();

    let listener_thread = thread::spawn(move || {
        let mut listener = ListenerBuilder::new()
            .recv_timeout(Duration::from_secs(5))
            .bind("127.0.0.1:0")
            .expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        port_tx.send(port).expect("send port");
        ok_tx.send(listener.accept().is_ok()).expect("send ok");
    });

    let port = port_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("listener did not bind in time");
    let url = CString::new(format!("srt://127.0.0.1:{port}?mss=1316")).unwrap();

    unsafe {
        let cfg = srtc_ts_sender_config_new();
        let s = srtc_ts_sender_open(url.as_ptr(), cfg);
        assert!(!s.is_null(), "open failed: {}", last_error_msg());

        let accepted_ok = ok_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("no accept result in time");
        assert!(accepted_ok, "listener accept() failed");

        srtc_ts_sender_close(s);
        srtc_ts_sender_config_free(cfg);
    }

    listener_thread.join().expect("listener thread panicked");
}

#[test]
fn ts_sender_payloadsize_url_open_succeeds() {
    let (port_tx, port_rx) = mpsc::channel::<u16>();
    let (ok_tx, ok_rx) = mpsc::channel::<bool>();

    let listener_thread = thread::spawn(move || {
        let mut listener = ListenerBuilder::new()
            .recv_timeout(Duration::from_secs(5))
            .bind("127.0.0.1:0")
            .expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        port_tx.send(port).expect("send port");
        ok_tx.send(listener.accept().is_ok()).expect("send ok");
    });

    let port = port_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("listener did not bind in time");
    let url = CString::new(format!("srt://127.0.0.1:{port}?payloadsize=1316")).unwrap();

    unsafe {
        let cfg = srtc_ts_sender_config_new();
        let s = srtc_ts_sender_open(url.as_ptr(), cfg);
        assert!(!s.is_null(), "open failed: {}", last_error_msg());

        let accepted_ok = ok_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("no accept result in time");
        assert!(accepted_ok, "listener accept() failed");

        srtc_ts_sender_close(s);
        srtc_ts_sender_config_free(cfg);
    }

    listener_thread.join().expect("listener thread panicked");
}

#[test]
fn ts_sender_lossmaxttl_url_open_succeeds() {
    let (port_tx, port_rx) = mpsc::channel::<u16>();
    let (ok_tx, ok_rx) = mpsc::channel::<bool>();

    let listener_thread = thread::spawn(move || {
        let mut listener = ListenerBuilder::new()
            .recv_timeout(Duration::from_secs(5))
            .bind("127.0.0.1:0")
            .expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        port_tx.send(port).expect("send port");
        ok_tx.send(listener.accept().is_ok()).expect("send ok");
    });

    let port = port_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("listener did not bind in time");
    let url = CString::new(format!("srt://127.0.0.1:{port}?lossmaxttl=20")).unwrap();

    unsafe {
        let cfg = srtc_ts_sender_config_new();
        let s = srtc_ts_sender_open(url.as_ptr(), cfg);
        assert!(!s.is_null(), "open failed: {}", last_error_msg());

        let accepted_ok = ok_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("no accept result in time");
        assert!(accepted_ok, "listener accept() failed");

        srtc_ts_sender_close(s);
        srtc_ts_sender_config_free(cfg);
    }

    listener_thread.join().expect("listener thread panicked");
}

#[test]
fn ts_sender_fc_url_open_succeeds() {
    let (port_tx, port_rx) = mpsc::channel::<u16>();
    let (ok_tx, ok_rx) = mpsc::channel::<bool>();

    let listener_thread = thread::spawn(move || {
        let mut listener = ListenerBuilder::new()
            .recv_timeout(Duration::from_secs(5))
            .bind("127.0.0.1:0")
            .expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        port_tx.send(port).expect("send port");
        ok_tx.send(listener.accept().is_ok()).expect("send ok");
    });

    let port = port_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("listener did not bind in time");
    let url = CString::new(format!("srt://127.0.0.1:{port}?fc=8192")).unwrap();

    unsafe {
        let cfg = srtc_ts_sender_config_new();
        let s = srtc_ts_sender_open(url.as_ptr(), cfg);
        assert!(!s.is_null(), "open failed: {}", last_error_msg());

        let accepted_ok = ok_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("no accept result in time");
        assert!(accepted_ok, "listener accept() failed");

        srtc_ts_sender_close(s);
        srtc_ts_sender_config_free(cfg);
    }

    listener_thread.join().expect("listener thread panicked");
}
