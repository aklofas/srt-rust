//! End-to-end RIST loopback tests.
//!
//! Verifies that bytes pushed into a `RistTransport` on 127.0.0.1 reach a
//! `RistRecvTransport` listening on the same port. Covers Simple Profile
//! (unencrypted) and Main Profile with AES-256 (mbedtls feature only).
//!
//! librist's handshake is slower than UDP — Simple is ~500ms, Main+AES is
//! ~800-1500ms — so we sleep before the first send and use a retry loop
//! on the recv side that tolerates Backpressure timeouts.

use std::net::UdpSocket;
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use tst_core::transport::{RecvTransport, Transport, TransportError};
use tst_rist::{
    EncryptionKey, RistProfile, RistRecvTransportBuilder, RistTransportBuilder,
};

/// Serializes RIST loopback tests within the process.
///
/// Cargo runs tests on multiple threads by default. The ephemeral-port +
/// librist-bind race is brittle: two tests can each "find" the same free
/// UDP port in their respective bind+drop windows, then race librist to
/// rebind. Serializing the setup phase avoids the collision entirely.
static SERIAL: Mutex<()> = Mutex::new(());

/// Bind a UDP socket to 127.0.0.1:0 to get an OS-assigned port, drop it, and
/// return the port. Called under [`SERIAL`] so two tests don't race for the
/// same port window.
fn find_free_udp_port() -> u16 {
    let s = UdpSocket::bind("127.0.0.1:0").expect("bind ephemeral");
    let port = s.local_addr().expect("local_addr").port();
    drop(s);
    port
}

/// 188 bytes of arbitrary payload — one MPEG-TS packet's worth.
fn synthetic_ts_packet(seq_byte: u8) -> [u8; 188] {
    let mut out = [seq_byte; 188];
    out[0] = 0x47; // TS sync byte
    out
}

/// Read up to `n_packets` from `recv` within `overall_timeout`, retrying on
/// Backpressure (librist's poll timeout). Returns the collected payloads.
fn drain_n(
    mut recv: tst_rist::RistRecvTransport,
    n_packets: usize,
    overall_timeout: Duration,
) -> Vec<Vec<u8>> {
    let mut out = Vec::with_capacity(n_packets);
    let mut buf = vec![0u8; recv.max_payload() + 64];
    let start = Instant::now();
    while out.len() < n_packets {
        if start.elapsed() >= overall_timeout {
            break;
        }
        match recv.recv_bytes(&mut buf) {
            Ok(n) => out.push(buf[..n].to_vec()),
            Err(TransportError::Backpressure { .. }) => continue,
            Err(e) => {
                eprintln!("recv error: {e:?}");
                break;
            }
        }
    }
    out
}

#[test]
fn simple_profile_unicast_loopback_round_trip() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let port = find_free_udp_port();
    let bind_url = format!("rist://@127.0.0.1:{port}");
    let connect_url = format!("rist://127.0.0.1:{port}");

    let recv = RistRecvTransportBuilder::new(&bind_url)
        .unwrap()
        .profile(RistProfile::Simple)
        .listen()
        .expect("listen");

    let (tx_payloads, rx_payloads) = mpsc::channel::<Vec<Vec<u8>>>();
    let _recv_thread = thread::spawn(move || {
        // librist Simple handshake ~500ms; give 8s overall for safety
        // on slow CI runners.
        let collected = drain_n(recv, 5, Duration::from_secs(8));
        let _ = tx_payloads.send(collected);
    });

    // Connect after the listener thread is running. Sleep gives the
    // recv-side a head-start to fully bind before we initiate.
    thread::sleep(Duration::from_millis(200));
    let mut send = RistTransportBuilder::new(&connect_url)
        .unwrap()
        .profile(RistProfile::Simple)
        .connect()
        .expect("connect");

    // Sleep again to let the librist handshake settle.
    thread::sleep(Duration::from_millis(600));

    let pkts: Vec<[u8; 188]> = (1..=5).map(|i| synthetic_ts_packet(i as u8)).collect();
    for p in &pkts {
        send.send_bytes(p).expect("send");
    }

    let collected = rx_payloads.recv_timeout(Duration::from_secs(10))
        .expect("recv thread didn't return in time");

    // librist's first few packets sometimes go missing during the
    // handshake settling phase. Accept any 3+ of the 5 reaching us — the
    // test is verifying the data-plane works, not that librist is
    // lossless across the first packet boundary.
    assert!(
        collected.len() >= 3,
        "expected at least 3 of 5 packets; got {}",
        collected.len()
    );

    // Each collected payload should be one of the originals (any order).
    for got in &collected {
        let matched = pkts.iter().any(|orig| got.as_slice() == orig.as_slice());
        assert!(
            matched,
            "received payload did not match any sent packet: {:?}",
            &got[..8.min(got.len())]
        );
    }
}

#[cfg(feature = "mbedtls")]
#[test]
fn main_profile_aes256_loopback_round_trip() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let port = find_free_udp_port();
    let bind_url = format!("rist://@127.0.0.1:{port}");
    let connect_url = format!("rist://127.0.0.1:{port}");
    let psk = "loopback-test-secret-keep-private";

    let recv = RistRecvTransportBuilder::new(&bind_url)
        .unwrap()
        .profile(RistProfile::Main)
        .encryption(EncryptionKey::aes256(psk))
        .listen()
        .expect("listen");

    let (tx_payloads, rx_payloads) = mpsc::channel::<Vec<Vec<u8>>>();
    let _recv_thread = thread::spawn(move || {
        // AES handshake takes longer; give 12s overall.
        let collected = drain_n(recv, 5, Duration::from_secs(12));
        let _ = tx_payloads.send(collected);
    });

    thread::sleep(Duration::from_millis(300));
    let mut send = RistTransportBuilder::new(&connect_url)
        .unwrap()
        .profile(RistProfile::Main)
        .encryption(EncryptionKey::aes256(psk))
        .connect()
        .expect("connect");

    // AES handshake is the slow one — 800ms-1.2s typical on Linux loopback.
    thread::sleep(Duration::from_millis(1200));

    let pkts: Vec<[u8; 188]> = (1..=5).map(|i| synthetic_ts_packet(i as u8)).collect();
    for p in &pkts {
        send.send_bytes(p).expect("send");
    }

    let collected = rx_payloads.recv_timeout(Duration::from_secs(15))
        .expect("recv thread didn't return in time");
    assert!(
        collected.len() >= 3,
        "expected at least 3 of 5 packets; got {}",
        collected.len()
    );
    for got in &collected {
        let matched = pkts.iter().any(|orig| got.as_slice() == orig.as_slice());
        assert!(matched, "decrypted payload mismatch");
    }
}
