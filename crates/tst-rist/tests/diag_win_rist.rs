//! TEMPORARY windows-only diagnostic — localize the librist Windows hang.
//!
//! The whole `loopback.rs` is gated off windows, so we don't actually know
//! whether *unencrypted* Simple-Profile RIST works on Windows or only the
//! Main-Profile AES-256 handshake stalls. This probe runs each librist
//! operation linearly with a marker printed immediately before and after, so
//! when a step hangs (nextest kills the test at its ~20s deadline) the LAST
//! printed marker pinpoints exactly which call blocked.
//!
//! Read via the dedicated `--no-capture` CI step. DELETE this file + that CI
//! step once the hang is localized and RIST-on-Windows is fixed or precisely
//! documented. Tests carry `loopback` in their name so nextest's `network`
//! test-group applies its tight per-test timeout (fast hang-kill).
#![cfg(target_os = "windows")]

use std::io::Write;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use tst_core::transport::{RecvTransport, Transport, TransportError};
use tst_rist::{RistProfile, RistRecvTransportBuilder, RistTransportBuilder};

fn mark(s: &str) {
    eprintln!("[DIAG_WIN_RIST] {s}");
    let _ = std::io::stderr().flush();
}

#[test]
fn diag_win_rist_simple_loopback() {
    let port = 33040u16;
    let bind_url = format!("rist://@127.0.0.1:{port}");
    let connect_url = format!("rist://127.0.0.1:{port}");

    mark("SIMPLE: about to build listener");
    let recv = RistRecvTransportBuilder::new(&bind_url)
        .unwrap()
        .profile(RistProfile::Simple)
        .listen()
        .expect("listen");
    mark("SIMPLE: listener built");

    let (tx, rx) = mpsc::channel::<usize>();
    let _h = thread::spawn(move || {
        let mut recv = recv;
        let mut buf = vec![0u8; recv.max_payload() + 64];
        let start = Instant::now();
        loop {
            if start.elapsed() > Duration::from_secs(5) {
                let _ = tx.send(0);
                return;
            }
            match recv.recv_bytes(&mut buf) {
                Ok(_) => {
                    let _ = tx.send(1);
                    return;
                }
                Err(TransportError::Backpressure { .. }) => continue,
                Err(_) => {
                    let _ = tx.send(0);
                    return;
                }
            }
        }
    });

    thread::sleep(Duration::from_millis(200));
    mark("SIMPLE: about to connect()");
    let mut send = RistTransportBuilder::new(&connect_url)
        .unwrap()
        .profile(RistProfile::Simple)
        .connect()
        .expect("connect");
    mark("SIMPLE: connect() returned");

    thread::sleep(Duration::from_millis(600));
    mark("SIMPLE: about to send_bytes x5");
    let pkt = [0x47u8; 188];
    for _ in 0..5 {
        let _ = send.send_bytes(&pkt);
    }
    mark("SIMPLE: send_bytes returned");

    let delivered = rx.recv_timeout(Duration::from_secs(7)).unwrap_or(0);
    mark(&format!(
        "SIMPLE: delivered={delivered} (1=yes,0=no/timeout)"
    ));
}

#[cfg(feature = "mbedtls")]
#[test]
fn diag_win_rist_main_aes_loopback() {
    use tst_rist::EncryptionKey;

    let port = 33043u16;
    let bind_url = format!("rist://@127.0.0.1:{port}");
    let connect_url = format!("rist://127.0.0.1:{port}");
    let psk = "diag-loopback-secret";

    mark("MAIN+AES: about to build listener");
    let recv = RistRecvTransportBuilder::new(&bind_url)
        .unwrap()
        .profile(RistProfile::Main)
        .encryption(EncryptionKey::aes256(psk))
        .listen()
        .expect("listen");
    mark("MAIN+AES: listener built");

    let (tx, rx) = mpsc::channel::<usize>();
    let _h = thread::spawn(move || {
        let mut recv = recv;
        let mut buf = vec![0u8; recv.max_payload() + 64];
        let start = Instant::now();
        loop {
            if start.elapsed() > Duration::from_secs(6) {
                let _ = tx.send(0);
                return;
            }
            match recv.recv_bytes(&mut buf) {
                Ok(_) => {
                    let _ = tx.send(1);
                    return;
                }
                Err(TransportError::Backpressure { .. }) => continue,
                Err(_) => {
                    let _ = tx.send(0);
                    return;
                }
            }
        }
    });

    thread::sleep(Duration::from_millis(300));
    mark("MAIN+AES: about to connect()");
    let mut send = RistTransportBuilder::new(&connect_url)
        .unwrap()
        .profile(RistProfile::Main)
        .encryption(EncryptionKey::aes256(psk))
        .connect()
        .expect("connect");
    mark("MAIN+AES: connect() returned");

    thread::sleep(Duration::from_millis(1200));
    mark("MAIN+AES: about to send_bytes x5");
    let pkt = [0x47u8; 188];
    for _ in 0..5 {
        let _ = send.send_bytes(&pkt);
    }
    mark("MAIN+AES: send_bytes returned");

    let delivered = rx.recv_timeout(Duration::from_secs(8)).unwrap_or(0);
    mark(&format!(
        "MAIN+AES: delivered={delivered} (1=yes,0=no/timeout)"
    ));
}
