//! Loopback round-trip tests for TcpTransport (plain, all 4 caller/listener × send/recv combos).

use std::io::{Read, Write};
use std::net::TcpListener as StdTcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use tst_core::transport::{RecvTransport, Transport};
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
