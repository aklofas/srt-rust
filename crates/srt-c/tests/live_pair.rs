//! End-to-end: srtc_mux_sender_t → real Listener → accept → recv.
//!
//! Binds a Listener on 127.0.0.1:0, opens srtc_mux_sender_t against the
//! kernel-assigned port, sends a synthetic NAL, and asserts the Listener
//! receives TS-shaped bytes (first byte 0x47).
//!
//! Uses the rlib output (crate-type includes "rlib") so that this
//! integration test can call the crate's Rust API directly — the same
//! `unsafe extern "C"` functions exported to C consumers.

use srt_core::srt::ListenerBuilder;
use srtc::config::{SrtcKlvStreamType, SrtcMuxConfig, SrtcVideoCodec};
use srtc::config::{
    srtc_mux_config_add_klv, srtc_mux_config_add_video, srtc_mux_config_free, srtc_mux_config_new,
};
use srtc::error::srtc_get_last_error_str;
use srtc::mux_sender::{srtc_mux_sender_close, srtc_mux_sender_open, srtc_mux_sender_send_video};
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

#[test]
fn mux_sender_to_listener_roundtrip() {
    let (port_tx, port_rx) = mpsc::channel::<u16>();
    let (recv_tx, recv_rx) = mpsc::channel::<Vec<u8>>();

    let listener_thread = thread::spawn(move || {
        let mut listener = ListenerBuilder::new()
            .recv_timeout(Duration::from_secs(5))
            .bind("127.0.0.1:0")
            .expect("bind");
        let port = listener.local_addr().expect("local_addr").port();
        port_tx.send(port).expect("send port");
        let (mut accepted, _peer) = listener.accept().expect("accept");
        let mut buf = vec![0u8; 4096];
        let n = accepted.recv(&mut buf).expect("recv");
        recv_tx.send(buf[..n].to_vec()).expect("send recv'd bytes");
    });

    let port = port_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("listener didn't bind in time");
    let url = CString::new(format!("srt://127.0.0.1:{port}")).unwrap();

    unsafe {
        let cfg: *mut SrtcMuxConfig = srtc_mux_config_new();
        srtc_mux_config_add_video(cfg, 0x1011, SrtcVideoCodec::H264);
        srtc_mux_config_add_klv(cfg, 0x1031, SrtcKlvStreamType::PrivateData, false);

        let s = srtc_mux_sender_open(url.as_ptr(), cfg);
        assert!(!s.is_null(), "open failed: {}", last_error_msg());
        srtc_mux_config_free(cfg);

        let nal: [u8; 9] = [0, 0, 0, 1, 0x65, 0xAA, 0xAA, 0xAA, 0xAA];
        let rc = srtc_mux_sender_send_video(s, nal.as_ptr(), nal.len(), 0, true);
        assert_eq!(rc, 0, "send_video failed: {}", last_error_msg());

        let received = recv_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("no bytes received within 5s");
        assert!(!received.is_empty(), "received empty buffer");
        assert_eq!(
            received[0], 0x47,
            "expected TS sync byte 0x47; got 0x{:02x}",
            received[0]
        );

        srtc_mux_sender_close(s);
    }

    listener_thread.join().expect("listener thread panicked");
}
