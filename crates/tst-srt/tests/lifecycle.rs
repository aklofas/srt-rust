//! Drop semantics + explicit close.

mod common;

use std::time::Duration;
use tst_srt::SocketBuilder;

#[test]
fn drop_closes_cleanly() {
    require_loopback!();
    let lb = common::Loopback::bind();
    let port = lb.port;

    let accept = lb.spawn_accept(|sock| {
        let _ = sock;
    });
    accept.wait_ready();

    {
        let socket = SocketBuilder::new()
            .recv_timeout(Duration::from_secs(5))
            .connect(format!("127.0.0.1:{port}"))
            .expect("connect");
        // Drop here.
        drop(socket);
    }
    accept.join();
}

#[test]
fn explicit_close_succeeds() {
    require_loopback!();
    let lb = common::Loopback::bind();
    let port = lb.port;

    let accept = lb.spawn_accept(|sock| {
        let _ = sock;
    });
    accept.wait_ready();

    let socket = SocketBuilder::new()
        .recv_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    socket.close().expect("close");
    accept.join();
}
