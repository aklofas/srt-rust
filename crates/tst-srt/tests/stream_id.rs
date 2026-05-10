//! Stream ID negotiation: caller-set ID is visible on the accepted Socket.

mod common;

use std::thread;
use std::time::Duration;
use tst_srt::{ListenerBuilder, SocketBuilder, StreamId};

#[test]
fn stream_id_round_trips() {
    require_loopback!();
    let mut listener = ListenerBuilder::new()
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");
    let port = listener.local_addr().unwrap().port();

    let lh = thread::spawn(move || {
        let (sock, _peer) = listener.accept().expect("accept");
        sock.stream_id().map(|s| s.to_string())
    });

    common::settle();

    let _socket = SocketBuilder::new()
        .stream_id(StreamId::new("publish:cam1").unwrap())
        .recv_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    let observed = lh.join().expect("listener");
    assert_eq!(observed.as_deref(), Some("publish:cam1"));
}
