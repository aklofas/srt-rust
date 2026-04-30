//! Send/recv round-trip; timeouts; buffer-too-small.

mod common;

use srt_core::error::RecvError;
use srt_core::srt::{ListenerBuilder, SocketBuilder};
use std::thread;
use std::time::Duration;

#[test]
fn small_payload_round_trips() {
    let mut listener = ListenerBuilder::new()
        .recv_timeout(Duration::from_secs(5))
        .send_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");

    let port = listener.local_addr().unwrap().port();
    let payload: &[u8] = b"hello, srt-core";
    let expected = payload.to_vec();

    let lh = thread::spawn(move || {
        let (mut sock, _peer) = listener.accept().expect("accept");
        let mut buf = [0u8; 1500];
        let n = sock.recv(&mut buf).expect("recv");
        buf[..n].to_vec()
    });

    common::settle();

    let mut socket = SocketBuilder::new()
        .recv_timeout(Duration::from_secs(5))
        .send_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");
    socket.send(payload).expect("send");

    let received = lh.join().expect("listener");
    assert_eq!(received, expected);
}

#[test]
fn recv_timeout_trips_typed_error() {
    let mut listener = ListenerBuilder::new()
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");
    let port = listener.local_addr().unwrap().port();

    let lh = thread::spawn(move || {
        // Accept but never send — caller should hit recv timeout.
        let (sock, _peer) = listener.accept().expect("accept");
        std::thread::sleep(Duration::from_secs(2));
        drop(sock);
    });

    common::settle();

    let mut socket = SocketBuilder::new()
        .recv_timeout(Duration::from_millis(500))
        .send_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    let mut buf = [0u8; 1500];
    let result = socket.recv(&mut buf);
    match result {
        Err(RecvError::TimedOut) => { /* expected */ }
        other => panic!("expected RecvError::TimedOut; got {other:?}"),
    }

    let _ = lh.join();
}
