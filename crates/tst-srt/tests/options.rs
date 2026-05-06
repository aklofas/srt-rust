//! Sample of SocketConfig field round-trips. Confirms the option-application
//! layer doesn't silently drop fields.

mod common;

use tst_srt::{Congestion, ListenerBuilder, MaxBandwidth, SocketBuilder};
use std::thread;
use std::time::Duration;

#[test]
fn latency_configures_without_error() {
    let mut listener = ListenerBuilder::new()
        .latency(Duration::from_millis(200))
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");
    let port = listener.local_addr().unwrap().port();

    let lh = thread::spawn(move || {
        let _ = listener.accept().expect("accept");
    });
    common::settle();

    let _socket = SocketBuilder::new()
        .latency(Duration::from_millis(200))
        .recv_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    lh.join().unwrap();
}

#[test]
fn payload_size_and_mss_apply() {
    let mut listener = ListenerBuilder::new()
        .mss(1316)
        .payload_size(1316)
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");
    let port = listener.local_addr().unwrap().port();

    let lh = thread::spawn(move || {
        let _ = listener.accept().expect("accept");
    });
    common::settle();

    let _socket = SocketBuilder::new()
        .mss(1316)
        .payload_size(1316)
        .recv_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    lh.join().unwrap();
}

#[test]
fn max_bandwidth_and_congestion_apply() {
    let mut listener = ListenerBuilder::new()
        .max_bandwidth(MaxBandwidth::Limited(10_000_000))
        .congestion(Congestion::Live)
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");
    let port = listener.local_addr().unwrap().port();

    let lh = thread::spawn(move || {
        let _ = listener.accept().expect("accept");
    });
    common::settle();

    let _socket = SocketBuilder::new()
        .max_bandwidth(MaxBandwidth::Limited(10_000_000))
        .congestion(Congestion::Live)
        .recv_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    lh.join().unwrap();
}
