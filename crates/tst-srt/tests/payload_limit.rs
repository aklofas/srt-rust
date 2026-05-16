//! Verifies `PayloadTooLarge` reports the actual configured limit, not 1316.
//! Regression for audit Issue 5.

mod common;

use std::time::Duration;
use tst_srt::error::SendError;
use tst_srt::{ListenerBuilder, SocketBuilder};

#[test]
fn payload_too_large_reports_configured_limit() {
    require_loopback!();
    let mut builder = ListenerBuilder::new();
    builder
        .payload_size(1456)
        .recv_timeout(Duration::from_secs(5));
    let lb = common::Loopback::bind_with(builder);
    let port = lb.port;

    let accept = lb.spawn_accept(|sock| sock);
    accept.wait_ready();

    let mut sender = SocketBuilder::new()
        .payload_size(1456)
        .send_timeout(Duration::from_secs(5))
        .connect(("127.0.0.1", port))
        .expect("connect");

    let _accepted = accept.join();

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
