//! `rtsps://` feature-gate smoke test.
//!
//! The live handshake itself is covered elsewhere: `tls_keepalive.rs`
//! drives a real in-process `rtsps://` server (client-side keepalive
//! over TLS) and `rtsp_server/tls.rs` covers the full server-side
//! handshake + request round-trip. This file only validates that the
//! `tls` feature gates correctly and the wrapper types stay visible.

#![cfg(feature = "tls")]

/// The wrapper types should be visible under the `tls` feature.
/// Smoke-tests that the public re-exports / module path stays stable.
#[test]
fn tls_module_compiles() {
    // Pure type check — does not allocate or hit the network.
    fn _assert_send<T: Send>() {}
    _assert_send::<tst_rtp::rtsp::client::tls::TlsStream>();
}
