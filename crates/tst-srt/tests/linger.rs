//! Verifies SRTO_LINGER setter reaches libsrt and short-linger lets Drop
//! return promptly. Audit Issue 4.

mod common;

use std::thread;
use std::time::{Duration, Instant};
use tst_srt::{ListenerBuilder, SocketBuilder};

#[test]
fn drop_with_zero_linger_does_not_block() {
    require_loopback!();
    let listener = ListenerBuilder::new()
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");
    let port = listener.local_addr().unwrap().port();
    let accept_thread = thread::spawn(move || {
        let mut l = listener;
        let _ = l.accept();
    });

    thread::sleep(Duration::from_millis(50));

    let socket = SocketBuilder::new()
        .linger(Duration::from_secs(0))
        .send_timeout(Duration::from_secs(5))
        .connect(("127.0.0.1", port))
        .expect("connect");

    let _ = accept_thread.join();

    let drop_started = Instant::now();
    drop(socket);
    let drop_elapsed = drop_started.elapsed();

    assert!(
        drop_elapsed < Duration::from_secs(2),
        "Drop with linger=0s should return promptly, took {drop_elapsed:?}",
    );
}
