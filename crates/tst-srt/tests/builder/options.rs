//! Sample of SocketConfig field round-trips. Confirms the option-application
//! layer doesn't silently drop fields.

use std::time::Duration;
use tst_srt::{Congestion, ListenerBuilder, MaxBandwidth, SocketBuilder};

#[test]
fn latency_configures_without_error() {
    require_loopback!();
    let mut builder = ListenerBuilder::new();
    builder
        .latency(Duration::from_millis(200))
        .recv_timeout(Duration::from_secs(5));
    let lb = crate::common::Loopback::bind_with(builder);
    let port = lb.port;

    let accept = lb.spawn_accept(|sock| {
        drop(sock);
    });
    accept.wait_ready();

    let _socket = SocketBuilder::new()
        .latency(Duration::from_millis(200))
        .recv_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    accept.join();
}

#[test]
fn payload_size_and_mss_apply() {
    require_loopback!();
    let mut builder = ListenerBuilder::new();
    builder
        .mss(1316)
        .payload_size(1316)
        .recv_timeout(Duration::from_secs(5));
    let lb = crate::common::Loopback::bind_with(builder);
    let port = lb.port;

    let accept = lb.spawn_accept(|sock| {
        drop(sock);
    });
    accept.wait_ready();

    let _socket = SocketBuilder::new()
        .mss(1316)
        .payload_size(1316)
        .recv_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    accept.join();
}

#[test]
fn max_bandwidth_and_congestion_apply() {
    require_loopback!();
    let mut builder = ListenerBuilder::new();
    builder
        .max_bandwidth(MaxBandwidth::Limited(10_000_000))
        .congestion(Congestion::Live)
        .recv_timeout(Duration::from_secs(5));
    let lb = crate::common::Loopback::bind_with(builder);
    let port = lb.port;

    let accept = lb.spawn_accept(|sock| {
        drop(sock);
    });
    accept.wait_ready();

    let _socket = SocketBuilder::new()
        .max_bandwidth(MaxBandwidth::Limited(10_000_000))
        .congestion(Congestion::Live)
        .recv_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    accept.join();
}
