//! Smoke test for the srt-sys FFI bindings.
//!
//! Exercises srt_startup / srt_getversion / srt_create_socket /
//! srt_close / srt_cleanup to verify that:
//!
//! 1. The build links libsrt correctly.
//! 2. The bindgen allowlists capture the load-bearing public API.
//! 3. Basic round-trip socket lifecycle succeeds.

use srt_sys::*;

const SRT_INVALID_SOCK: SRTSOCKET = -1;

/// libsrt's startup is process-global and refcounted internally.
/// Cargo runs each integration-test binary in its own process,
/// giving us isolation across files. Inside one process, multiple
/// `#[test]` functions share the runtime — we re-call startup
/// /cleanup per test, which matches libsrt's documented refcounting
/// (see srt.h: srt_startup may be called multiple times).
fn ensure_startup() {
    // SAFETY: srt_startup takes no arguments and is idempotent.
    let rc = unsafe { srt_startup() };
    assert!(rc >= 0, "srt_startup failed: rc={rc}");
}

fn cleanup() {
    // SAFETY: matched against a prior srt_startup call.
    let rc = unsafe { srt_cleanup() };
    assert!(rc >= 0, "srt_cleanup failed: rc={rc}");
}

#[test]
fn version_is_at_least_1_5_0() {
    ensure_startup();
    // SAFETY: srt_getversion takes no arguments and returns a packed u32.
    let v = unsafe { srt_getversion() };
    let major = (v >> 16) & 0xff;
    let minor = (v >> 8) & 0xff;
    let patch = v & 0xff;
    println!("libsrt version: {major}.{minor}.{patch} (raw=0x{v:08x})");
    assert!(
        major == 1 && minor >= 5,
        "expected libsrt 1.5+, got {major}.{minor}.{patch}"
    );
    cleanup();
}

#[test]
fn socket_lifecycle_round_trip() {
    ensure_startup();
    // SAFETY: srt_create_socket is the documented constructor; takes no arguments.
    let sock = unsafe { srt_create_socket() };
    assert_ne!(
        sock, SRT_INVALID_SOCK,
        "srt_create_socket returned SRT_INVALID_SOCK"
    );

    // SAFETY: sock is a valid handle returned by srt_create_socket above.
    let rc = unsafe { srt_close(sock) };
    assert_eq!(rc, 0, "srt_close failed: rc={rc}");

    cleanup();
}
