//! Verifies `PayloadTooLarge` reports the actual configured limit, not 1316.
//! Regression for audit Issue 5.

use tst_srt::error::SendError;
use tst_srt::{ListenerBuilder, SocketBuilder};
use std::thread;
use std::time::Duration;

#[test]
fn payload_too_large_reports_configured_limit() {
    let mut listener = ListenerBuilder::new()
        .payload_size(1456)
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");
    let port = listener.local_addr().unwrap().port();

    let accept_thread = thread::spawn(move || {
        let (sock, _peer) = listener.accept().expect("accept");
        sock
    });

    thread::sleep(Duration::from_millis(50));

    let mut sender = SocketBuilder::new()
        .payload_size(1456)
        .send_timeout(Duration::from_secs(5))
        .connect(("127.0.0.1", port))
        .expect("connect");

    let _accepted = accept_thread.join().expect("join");

    let big = vec![0u8; 2000];
    match sender.send(&big) {
        Err(SendError::PayloadTooLarge { actual, limit }) => {
            assert_eq!(actual, 2000);
            assert_eq!(
                limit, 1456,
                "limit should be configured payload size, not the libsrt default"
            );
        }
        other => panic!("expected PayloadTooLarge {{ limit: 1456 }}, got {other:?}"),
    }
}
