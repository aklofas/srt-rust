//! Verifies `Transport::socket_stats()` / `RecvTransport::socket_stats()` map
//! libsrt's `CBytePerfMon` into `tst_core::transport::SocketStats` and return
//! `None` after the socket is closed.

mod common;

use crate::common::Loopback;
use std::time::Duration;
use tst_core::transport::{RecvTransport, Transport};
use tst_srt::{SocketBuilder, SrtTransport};

#[test]
fn socket_stats_returns_some_on_live_send() {
    require_loopback!();
    let lb = Loopback::bind();
    let port = lb.port;

    // Accept thread holds the socket open while we measure stats on the
    // connecting side. Returning the socket back through join would close
    // it on drop; instead, the thread parks until we kill it via close.
    let accept = lb.spawn_accept(|mut sock| {
        let mut buf = [0u8; 1500];
        // Recv loop; exit when peer closes (returns Broken/Closed).
        loop {
            match sock.recv(&mut buf) {
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    });
    accept.wait_ready();

    let socket = SocketBuilder::new()
        .send_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");
    let mut sender = SrtTransport::new(socket);

    sender.send_bytes(&[0u8; 188]).expect("send");
    // Settle so libsrt's accounting catches up.
    std::thread::sleep(Duration::from_millis(50));

    // SrtTransport implements both Transport AND RecvTransport (it's
    // bidirectional in libsrt's model) — disambiguate the trait.
    let stats = <SrtTransport as Transport>::socket_stats(&sender)
        .expect("live socket has stats");
    assert!(stats.bytes_sent >= 188, "bytes_sent={}", stats.bytes_sent);
    assert!(stats.packets_sent >= 1, "packets_sent={}", stats.packets_sent);
    assert_eq!(stats.bytes_received, 0, "sender should read 0 received");

    <SrtTransport as Transport>::close(&mut sender);
    let _ = accept.join();
}

#[test]
fn socket_stats_returns_none_after_close() {
    require_loopback!();
    let lb = Loopback::bind();
    let port = lb.port;

    let accept = lb.spawn_accept(|mut sock| {
        let mut buf = [0u8; 1500];
        let _ = sock.recv(&mut buf);
    });
    accept.wait_ready();

    let socket = SocketBuilder::new()
        .send_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");
    let mut sender = SrtTransport::new(socket);
    <SrtTransport as Transport>::close(&mut sender);

    assert!(
        <SrtTransport as Transport>::socket_stats(&sender).is_none(),
        "closed socket has no stats"
    );

    let _ = accept.join();
}

#[test]
fn managed_socket_stats_forwards_when_alive_and_none_after_close() {
    use tst_pipeline::{ManagedTransport, ReconnectPolicy};

    require_loopback!();
    let lb = Loopback::bind();
    let port = lb.port;

    let accept = lb.spawn_accept(|mut sock| {
        let mut buf = [0u8; 1500];
        loop {
            match sock.recv(&mut buf) {
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    });
    accept.wait_ready();

    let initial = SocketBuilder::new()
        .send_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");
    let initial_transport = SrtTransport::new(initial);

    // factory is a no-op for this test — we never trigger a reconnect.
    let factory = move || -> Result<SrtTransport, tst_core::transport::TransportError> {
        Err(tst_core::transport::TransportError::Broken(
            "reconnect not exercised by this test".into(),
        ))
    };
    let mut managed = ManagedTransport::new(initial_transport, factory, ReconnectPolicy::default());

    managed
        .send_bytes(&[0u8; 188])
        .expect("send through managed");
    std::thread::sleep(Duration::from_millis(50));

    // ALIVE: forwards to SrtTransport which returns Some.
    let stats = managed
        .socket_stats()
        .expect("alive managed forwards Some");
    assert!(stats.bytes_sent >= 188, "bytes_sent={}", stats.bytes_sent);

    // CLOSED: inner Option goes to None → returns None.
    managed.close();
    assert!(
        managed.socket_stats().is_none(),
        "closed managed returns None"
    );

    let _ = accept.join();
}

#[test]
fn recv_transport_socket_stats_returns_some_on_live_recv() {
    require_loopback!();
    let lb = Loopback::bind();
    let port = lb.port;

    // Accept thread: receive one chunk, then convert the socket into an
    // SrtTransport-as-RecvTransport and read stats. We return the stats out
    // of the closure so the main thread can assert on them.
    let accept = lb.spawn_accept(|mut sock| {
        let mut buf = [0u8; 1500];
        let n = sock.recv(&mut buf).expect("recv");
        // Settle for libsrt's recv-side accounting.
        std::thread::sleep(Duration::from_millis(50));
        let recv_transport = SrtTransport::new(sock);
        let s = <SrtTransport as RecvTransport>::socket_stats(&recv_transport)
            .expect("live recv socket has stats");
        (n, s)
    });
    accept.wait_ready();

    let mut socket = SocketBuilder::new()
        .send_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");
    socket.send(&[0xAB; 188]).expect("send");

    let (n, stats) = accept.join();
    assert_eq!(n, 188);
    assert!(stats.bytes_received >= 188, "bytes_received={}", stats.bytes_received);
    assert!(stats.packets_received >= 1, "packets_received={}", stats.packets_received);
    assert_eq!(stats.bytes_sent, 0, "receiver should read 0 sent");
}
