//! Loopback round-trip tests for TcpTransport (plain, all 4 caller/listener × send/recv combos).

use std::io::{Read, Write};
use std::net::TcpListener as StdTcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use tst_core::transport::{RecvTransport, Transport, TransportError};
use tst_tcp::TcpListener;
use tst_tcp::TcpTransport;

#[test]
fn caller_sender_to_std_listener() {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let _t = thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        let mut buf = vec![0u8; 1024];
        let n = sock.read(&mut buf).unwrap();
        let _ = tx.send(buf[..n].to_vec());
    });

    let mut send = TcpTransport::connect(&format!("tcp://127.0.0.1:{port}")).unwrap();
    send.send_bytes(&[0x47u8; 188]).unwrap();

    let got = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(got, vec![0x47u8; 188]);
}

#[test]
fn caller_receiver_from_std_sender() {
    let listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let _t = thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        sock.write_all(&[0x47u8; 188]).unwrap();
    });

    let mut recv = TcpTransport::connect(&format!("tcp://127.0.0.1:{port}")).unwrap();
    let mut buf = vec![0u8; 1024];
    let n = recv.recv_bytes(&mut buf).unwrap();
    assert_eq!(n, 188);
    assert!(buf[..n].iter().all(|&b| b == 0x47));
}

#[test]
fn listener_receiver_from_std_caller() {
    let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let port = listener.local_addr().unwrap().port();

    let _t = thread::spawn(move || {
        let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        sock.write_all(&[0x47u8; 188]).unwrap();
    });

    let mut accepted = listener.accept_blocking().unwrap();
    let mut buf = vec![0u8; 1024];
    let n = accepted.recv_bytes(&mut buf).unwrap();
    assert_eq!(n, 188);
    assert!(buf[..n].iter().all(|&b| b == 0x47));
}

#[test]
fn listener_sender_to_std_caller() {
    let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let port = listener.local_addr().unwrap().port();

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let _t = thread::spawn(move || {
        let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut buf = vec![0u8; 1024];
        let n = sock.read(&mut buf).unwrap();
        let _ = tx.send(buf[..n].to_vec());
    });

    let mut accepted = listener.accept_blocking().unwrap();
    accepted.send_bytes(&[0x47u8; 188]).unwrap();
    let got = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(got, vec![0x47u8; 188]);
}

/// Thread A parks in `recv_bytes` on a connected-but-silent peer; thread B
/// calls `cancel_handle.cancel()`. Thread A must exit with `Closed` (or
/// `ExplicitClose`) within ≤1 poll interval (~100 ms) plus scheduling slack.
/// Watchdog: 3 s.
#[test]
fn cancel_handle_unblocks_parked_recv() {
    // Set up a silent peer: accept the connection but send nothing.
    let peer_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let port = peer_listener.local_addr().unwrap().port();
    let _peer = thread::spawn(move || {
        // Accept and hold the socket open so recv_bytes isn't unblocked by a
        // connection-close event.
        let (_sock, _) = peer_listener.accept().unwrap();
        thread::sleep(Duration::from_secs(10));
    });

    let mut transport = TcpTransport::connect(&format!("tcp://127.0.0.1:{port}")).unwrap();
    let handle = transport.cancel_handle();

    // Channel: thread A signals when recv_bytes returned.
    let (tx, rx) = mpsc::channel::<Result<usize, TransportError>>();
    let _recv_thread = thread::spawn(move || {
        let mut buf = vec![0u8; 1024];
        let result = transport.recv_bytes(&mut buf);
        let _ = tx.send(result);
    });

    // Give the recv thread time to park in recv_bytes.
    thread::sleep(Duration::from_millis(50));

    // Cancel from the main thread.
    handle.cancel();

    // Recv thread must unblock within 3 s (≤100 ms + scheduling slack).
    let result = rx.recv_timeout(Duration::from_secs(3))
        .expect("recv_bytes did not unblock within watchdog period after cancel");

    // The transport must report it is no longer alive.
    assert!(
        matches!(result, Err(TransportError::Closed) | Err(TransportError::ExplicitClose)),
        "expected Closed/ExplicitClose after cancel, got {:?}", result
    );
}
