//! Phase 3 Wave F Task 26 — verification of Phase 2 deferred fix 2
//! (TLS-side keepalive).
//!
//! T21 shipped the `Arc<Mutex<Stream>>` refactor on
//! [`RtspClient`](tst_rtp::RtspClient) that eliminates the prior
//! `TcpStream::try_clone` limitation; the auto-keepalive thread (spawned
//! by
//! [`spawn_keepalive_if_needed`](tst_rtp::RtspClient::spawn_keepalive_if_needed)
//! from the builder) now works uniformly across `Stream::Plain` and
//! `Stream::Tls`. Pre-T21 the TLS variant was silently a no-op because
//! rustls `ClientConnection` isn't clonable.
//!
//! # Why ignored
//!
//! The test stays `#[ignore]` because running it requires a real
//! `rtsps://` fixture server (cert + key + matching root cert in the
//! client's trust store). The repo doesn't carry a checked-in cert
//! fixture and the `rcgen` self-signed-cert generator is not a dev-dep
//! today — same situation as `tests/rtsp_client_tls.rs::rtsps_handshake_smoke`.
//! When the cert fixture lands (planned at `tests/fixtures/tls_certs.rs`
//! per the comments in `crates/tst-rtp/src/rtsp/server/tls.rs:165`), set
//! `RTSP_TLS_FIXTURE=1` and remove the `#[ignore]` to activate.
//!
//! The test does compile under `--features tls`, which protects against
//! a regression that removes `RtspClientBuilder::tls_root_certs` or
//! breaks the keepalive-on-TLS public surface.

#![cfg(feature = "tls")]

use std::time::Duration;

use secrecy::SecretString;
use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
use tst_rtp::{RtspClientBuilder, RtspServerBuilder};

fn make_muxer_cfg() -> MuxerConfig {
    let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
    prog.add_video(0x1011, VideoCodec::H264);
    let mut b = MuxerConfig::builder();
    b.add_program(prog.build());
    b.build().unwrap()
}

/// Connect to an `rtsps://` server and verify the keepalive thread spawns
/// over TLS (pre-T21 it silently no-op'd due to `try_clone` returning
/// `Unsupported`). [`RtspClient::is_session_alive`](tst_rtp::RtspClient::is_session_alive)
/// returning `true` after the keepalive thread has been running for a
/// few hundred ms is the observable check — if the thread had failed to
/// spawn or had immediately errored on a control-TCP write, the
/// `session_dead` flag would have flipped.
///
/// Builds the client through [`RtspClientBuilder`] (rather than
/// [`RtspClient::connect`](tst_rtp::RtspClient::connect) +
/// `spawn_keepalive_if_needed`) because the builder is the public
/// keepalive-spawn path the public-API consumers will hit.
#[test]
#[ignore = "needs rtsps:// cert fixture (rcgen dev-dep not present); \
             un-ignore once tests/fixtures/tls_certs.rs lands"]
fn rtsps_keepalive_thread_spawns_on_connect() {
    // Build the TLS server. Cert + key paths come from the fixture-to-be.
    // Until that lands, these paths are intentionally non-existent so
    // any accidental un-ignore fails loudly at build() rather than
    // silently passing.
    let cert_pem = std::path::PathBuf::from("tests/fixtures/tls_certs/server.crt");
    let key_pem = std::path::PathBuf::from("tests/fixtures/tls_certs/server.key");
    let mut sb = RtspServerBuilder::new("rtsps://127.0.0.1:0").unwrap();
    sb.tls_cert(cert_pem, key_pem);
    let server = sb.build().unwrap();
    let _mount = server.add_mount("/live", make_muxer_cfg()).unwrap();
    server.start().unwrap();
    let port = server.local_addr().unwrap().port();
    let url = format!("rtsps://127.0.0.1:{port}/live");

    // Build a client trusting the test cert. The fixture is expected to
    // export the root PEM bytes used to sign the server cert.
    //
    // Using runtime fs read rather than `include_bytes!` so the test
    // file compiles when the fixture isn't present yet — the `#[ignore]`
    // attribute keeps it from running.
    let root_pem = std::fs::read("tests/fixtures/tls_certs/root.pem").unwrap();
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_pemfile::certs(&mut root_pem.as_slice()) {
        roots.add(cert.unwrap()).unwrap();
    }

    let mut client = RtspClientBuilder::new(&url)
        .unwrap()
        .keepalive_interval(Duration::from_millis(50))
        .auth("unused", SecretString::new("unused".into()))
        .tls_root_certs(roots)
        .connect()
        .unwrap();

    // Drive an OPTIONS round trip so the connection is fully live, then
    // wait a few keepalive cycles. The keepalive thread shares
    // `Arc<Mutex<Stream>>` with the main client (T21 refactor); a TLS
    // write failure would flip `session_dead` and make
    // `is_session_alive` return false.
    client.options().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        client.is_session_alive(),
        "TLS keepalive thread should hold the rtsps:// session alive; \
         pre-T21 this assertion would fail because the thread couldn't \
         clone the TLS stream"
    );

    drop(client);
    server.stop().ok();
}
