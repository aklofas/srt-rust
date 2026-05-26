//! Smoke tests: each `tst_rtp_*_open` with a kernel-picked port returns
//! a non-null handle, and that handle closes cleanly.
//!
//! These tests are gated on `feature = "rtp"` so they compile only in
//! builds that include the RTP transport. They do NOT send or receive
//! actual TS data — that belongs to the data-path tasks (Wave B/C).
//! The goal here is: open succeeds, handle is non-null, close is
//! UB-free (no double-free, no leak detected by Miri/Valgrind).
#![cfg(feature = "rtp")]

use std::ffi::CString;

use tstrans::error::tst_get_last_error;
use tstrans::rtp::{
    tst_rtp_receiver_close, tst_rtp_recv_open, tst_rtp_sender_close, tst_rtp_sender_open,
};

/// Bind a receiver on the loopback with port 0 (kernel-assigned).
/// The handle must be non-null and must close without crashing.
#[test]
fn rtp_recv_open_unicast_zero_port_returns_handle() {
    // rtp://127.0.0.1:0 — port 0 tells the kernel to assign an
    // ephemeral port. Unicast bind to loopback. No actual sender
    // exists, but the socket bind itself must succeed.
    let url = CString::new("rtp://127.0.0.1:0").unwrap();
    let handle = unsafe { tst_rtp_recv_open(url.as_ptr()) };
    assert!(
        !handle.is_null(),
        "tst_rtp_recv_open returned null for rtp://127.0.0.1:0"
    );
    // Verify that tst_rtp_receiver_close is safe to call on a live
    // handle and does not double-free or crash.
    unsafe { tst_rtp_receiver_close(handle) };
}

/// Open an RTP sender to a loopback unicast address. The sender
/// creates a UDP socket and targets 127.0.0.1:0, which the kernel
/// assigns to a local port on first send. No receiver is needed for
/// the open itself — RTP sender open does NOT wait for a peer (UDP is
/// connectionless).
#[test]
fn rtp_sender_open_unicast_returns_handle() {
    // Port 0 on the sender side means the kernel picks the LOCAL bind
    // port for the underlying RTCP socket (the RTP destination port
    // is the one in the URL — for this smoke test we pick a high-
    // numbered port that is unlikely to be in use; no packet is sent).
    let url = CString::new("rtp://127.0.0.1:54321").unwrap();
    let handle = unsafe { tst_rtp_sender_open(url.as_ptr()) };
    assert!(
        !handle.is_null(),
        "tst_rtp_sender_open returned null for rtp://127.0.0.1:54321"
    );
    unsafe { tst_rtp_sender_close(handle) };
}

/// A malformed URL must return null and set a meaningful last-error.
/// The URL `not-a-url` has no scheme separator — `RtpUrl::parse` will
/// reject it.
#[test]
fn rtp_recv_open_malformed_url_returns_null() {
    let url = CString::new("not-a-url").unwrap();
    let handle = unsafe { tst_rtp_recv_open(url.as_ptr()) };
    assert!(
        handle.is_null(),
        "tst_rtp_recv_open should return null for a malformed URL"
    );
    // Verify the last-error code is set (non-zero = failure).
    let code = unsafe { tst_get_last_error() };
    assert!(
        code < 0,
        "last-error code should be negative after parse failure, got {code}"
    );
}
