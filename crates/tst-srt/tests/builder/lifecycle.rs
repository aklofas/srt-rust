//! Drop semantics + explicit close.

use std::time::Duration;
use tst_srt::SocketBuilder;

#[test]
fn drop_closes_cleanly() {
    require_loopback!();
    let lb = crate::common::Loopback::bind();
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
    let lb = crate::common::Loopback::bind();
    let port = lb.port;

    let accept = lb.spawn_accept(|sock| {
        let _ = sock;
    });
    accept.wait_ready();

    let socket = SocketBuilder::new()
        .recv_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    // Wait for accept to complete BEFORE closing — otherwise on fast
    // hardware (observed on linux-aarch64) socket.close() can win the
    // race against listener.accept() returning, and accept then panics
    // with "Connection was broken" because the handshake completed but
    // the peer immediately closed. Swapping the order keeps the
    // verification intent (close doesn't crash) while eliminating the
    // race.
    accept.join();
    socket.close().expect("close");
}
