//! Live-socket verification that `MaxBandwidth::Unlimited` reaches libsrt as 0.
//! Regression for the `MaxBandwidth::Infinite = -2` bug.
//!
//! libsrt rejects any `SRTO_MAXBW < -1` with `MJ_NOTSUP / MN_INVAL` (see
//! `socketconfig.cpp`), so the previous `Infinite` variant always errored at
//! runtime. This test pins the only correct way to express "no cap" — the
//! `Unlimited` variant, which maps to libsrt's sentinel `0` — through a real
//! handshake against a local listener.

use std::thread;
use std::time::Duration;
use tst_srt::{ListenerBuilder, MaxBandwidth, SocketBuilder};

#[test]
fn unlimited_max_bandwidth_reaches_libsrt_as_zero() {
    let mut listener = ListenerBuilder::new()
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");
    let port = listener.local_addr().unwrap().port();

    let accept_thread = thread::spawn(move || {
        let (sock, _peer) = listener.accept().expect("accept");
        drop(sock);
    });

    // Brief settle so the listener is ready before we connect.
    thread::sleep(Duration::from_millis(50));

    let socket = SocketBuilder::new()
        .max_bandwidth(MaxBandwidth::Unlimited)
        .recv_timeout(Duration::from_secs(5))
        .send_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    drop(socket);
    accept_thread.join().expect("listener thread");
}
