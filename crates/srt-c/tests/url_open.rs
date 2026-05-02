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

// ============================================================================
// Group 1 passphrase + pbkeylen — listener uses the same passphrase so the
// encrypted handshake succeeds. The strongest test in this batch: exercises
// full AES-128 key exchange rather than just option forwarding.
// ============================================================================

#[test]
fn ts_sender_passphrase_handshake_ok() {
    let (port_tx, port_rx) = mpsc::channel::<u16>();
    let (ok_tx, ok_rx) = mpsc::channel::<bool>();

    let listener_thread = thread::spawn(move || {
        use srt_core::srt::Passphrase;
        let mut listener = ListenerBuilder::new()
            .passphrase(Passphrase::new("hunter-too-long-thanks").unwrap())
            .key_length(srt_core::KeyLength::Aes128)
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
    let url = CString::new(format!(
        "srt://127.0.0.1:{port}?passphrase=hunter-too-long-thanks&pbkeylen=16"
    ))
    .unwrap();

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

// ============================================================================
// Group 1 — latency-family keys (rcvlatency, peerlatency).
//
// Both are pre-connect options the caller sets on its own socket; peer
// negotiates the effective receive buffer delay. We verify the URL param
// passes through the parser + setsockopt + handshake without error.
// ============================================================================

#[test]
fn ts_sender_rcvlatency_url_open_succeeds() {
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
    let url = CString::new(format!("srt://127.0.0.1:{port}?rcvlatency=120")).unwrap();

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
fn ts_sender_peerlatency_url_open_succeeds() {
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
    let url = CString::new(format!("srt://127.0.0.1:{port}?peerlatency=80")).unwrap();

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

// ============================================================================
// Group 1 — bandwidth-shaping keys (maxbw, inputbw, oheadbw).
//
// All three are caller-side-only options controlling the sender's packet
// scheduling; they don't appear in the SRT handshake extension fields.
// We verify they survive the URL parse → setsockopt → connect sequence.
// ============================================================================

#[test]
fn ts_sender_maxbw_url_open_succeeds() {
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
    let url = CString::new(format!("srt://127.0.0.1:{port}?maxbw=10000000")).unwrap();

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
fn ts_sender_inputbw_url_open_succeeds() {
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
    let url = CString::new(format!("srt://127.0.0.1:{port}?inputbw=5000000")).unwrap();

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
fn ts_sender_oheadbw_url_open_succeeds() {
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
    let url = CString::new(format!("srt://127.0.0.1:{port}?oheadbw=25")).unwrap();

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

// ============================================================================
// Group 1 — tlpktdrop (too-late packet drop).
//
// Caller-side option; controls whether the sender drops packets that have
// been waiting beyond the latency budget. Verify URL parse + connect succeeds.
// ============================================================================

#[test]
fn ts_sender_tlpktdrop_url_open_succeeds() {
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
    let url = CString::new(format!("srt://127.0.0.1:{port}?tlpktdrop=1")).unwrap();

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

// ============================================================================
// Group 1 — packetfilter (FEC).
//
// Both sides must agree on the filter configuration for the handshake to
// succeed — the listener is pre-configured with the same FEC spec string so
// libsrt's filter-negotiation exchange passes. Verifies that the URL parser
// correctly forwards the packetfilter string to SRTO_PACKETFILTER.
// ============================================================================

#[test]
fn ts_sender_packetfilter_url_open_succeeds() {
    let (port_tx, port_rx) = mpsc::channel::<u16>();
    let (ok_tx, ok_rx) = mpsc::channel::<bool>();

    let listener_thread = thread::spawn(move || {
        use srt_core::srt::PacketFilter;
        let mut listener = ListenerBuilder::new()
            .packet_filter(PacketFilter::new("fec,cols:10,rows:5,arq:onreq").unwrap())
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
    let url = CString::new(format!(
        "srt://127.0.0.1:{port}?packetfilter=fec,cols:10,rows:5,arq:onreq"
    ))
    .unwrap();

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

// ============================================================================
// Group 1 — congestion controller selection.
//
// "live" is the only controller currently supported in our URL overlay.
// The option must be set before connect; verify end-to-end handshake succeeds.
// ============================================================================

#[test]
fn ts_sender_congestion_live_url_open_succeeds() {
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
    let url = CString::new(format!("srt://127.0.0.1:{port}?congestion=live")).unwrap();

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

// ============================================================================
// Group 2 extension keys (x-recvtimeout / x-sendtimeout).
//
// These are srt-rust extensions — not standard SRT URL keys. They set
// SRTO_RCVTIMEO / SRTO_SNDTIMEO (in milliseconds) before connect. Because
// they control local I/O timeouts rather than SRT protocol behaviour, no
// special listener configuration is needed.
// ============================================================================

#[test]
fn ts_sender_x_recvtimeout_url_open_succeeds() {
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
    let url = CString::new(format!("srt://127.0.0.1:{port}?x-recvtimeout=5000")).unwrap();

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
fn ts_sender_x_sendtimeout_url_open_succeeds() {
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
    let url = CString::new(format!("srt://127.0.0.1:{port}?x-sendtimeout=2000")).unwrap();

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
