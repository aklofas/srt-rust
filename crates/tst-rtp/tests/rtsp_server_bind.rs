//! Phase 3 Task 22 — RtspServer bind / start / stop / cancel-handle
//! lifecycle integration tests. No RTP/RTCP flow exercised here;
//! T23-T26 cover the actual streaming paths.

use tst_rtp::{RtspServer, RtspServerError};

#[test]
fn bind_succeeds_on_loopback_port_zero() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    // Before start(), local_addr() is None — the listener hasn't bound yet.
    assert!(server.local_addr().is_none());
}

#[test]
fn start_populates_local_addr() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    server.start().unwrap();
    let addr = server.local_addr().expect("listener bound after start()");
    assert_eq!(addr.ip().to_string(), "127.0.0.1");
    assert!(addr.port() > 0, "kernel-assigned port must be non-zero");
}

#[test]
fn bind_rejects_dns_hostname() {
    let res = RtspServer::bind("rtsp://example.com:8554");
    assert!(
        matches!(res, Err(RtspServerError::UrlParse(_))),
        "DNS host must be rejected at parse time; got {res:?}"
    );
}

#[test]
fn bind_rejects_malformed_url() {
    let res = RtspServer::bind("not-a-url");
    assert!(
        matches!(res, Err(RtspServerError::UrlParse(_))),
        "malformed URL must be rejected; got {res:?}"
    );
}

#[test]
fn start_twice_errors_already_started() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    server.start().unwrap();
    let e = server.start().unwrap_err();
    assert!(matches!(e, RtspServerError::AlreadyStarted));
}

#[test]
fn stop_before_start_errors_not_started() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    let e = server.stop().unwrap_err();
    assert!(matches!(e, RtspServerError::NotStarted));
}

#[test]
fn cancel_handle_obtainable_pre_start() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    // cancel_handle() must work before start() — callers may stash it
    // ahead of time for cross-thread shutdown coordination.
    let h = server.cancel_handle();
    assert!(!h.is_canceled());
    h.cancel();
    // The same handle observes its own flip.
    assert!(h.is_canceled());
    // A second handle obtained from the same server observes it too.
    let h2 = server.cancel_handle();
    assert!(h2.is_canceled());
}

#[test]
fn drop_unstarted_server_does_not_panic() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    drop(server);
}

#[test]
fn drop_started_server_completes_within_runtime_budget() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    server.start().unwrap();
    // Drop's hard-cancel + shutdown_timeout(5s) must complete cleanly.
    drop(server);
}
