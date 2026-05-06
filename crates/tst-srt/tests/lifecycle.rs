//! Drop semantics + explicit close.

mod common;

use std::thread;
use std::time::Duration;
use tst_srt::{ListenerBuilder, SocketBuilder};

#[test]
fn drop_closes_cleanly() {
    let mut listener = ListenerBuilder::new()
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");
    let port = listener.local_addr().unwrap().port();

    let lh = thread::spawn(move || {
        let _ = listener.accept();
    });

    common::settle();
    {
        let socket = SocketBuilder::new()
            .recv_timeout(Duration::from_secs(5))
            .connect(format!("127.0.0.1:{port}"))
            .expect("connect");
        // Drop here.
        drop(socket);
    }
    let _ = lh.join();
}

#[test]
fn explicit_close_succeeds() {
    let mut listener = ListenerBuilder::new()
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");
    let port = listener.local_addr().unwrap().port();

    let lh = thread::spawn(move || {
        let _ = listener.accept();
    });

    common::settle();

    let socket = SocketBuilder::new()
        .recv_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    socket.close().expect("close");
    let _ = lh.join();
}
