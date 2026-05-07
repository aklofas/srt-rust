//! Tests for `Listener::accept_timeout`. Requires libsrt loopback.

use std::time::Duration;
use tst_srt::{AcceptError, ListenerBuilder};

#[test]
fn accept_timeout_returns_timed_out_when_no_connection() {
    let mut listener = ListenerBuilder::new().bind("127.0.0.1:0").expect("bind");

    let start = std::time::Instant::now();
    let result = listener.accept_timeout(Duration::from_millis(200));
    let elapsed = start.elapsed();

    match result {
        Err(AcceptError::TimedOut) => {}
        Ok(_) => panic!("expected TimedOut; got Ok"),
        Err(e) => panic!("expected TimedOut; got Err({e:?})"),
    }
    // Must have actually waited at least most of the timeout.
    assert!(elapsed >= Duration::from_millis(150), "elapsed={elapsed:?}");
    // But not absurdly long.
    assert!(elapsed < Duration::from_millis(1000), "elapsed={elapsed:?}");
}

#[test]
fn accept_timeout_succeeds_when_peer_connects() {
    let mut listener = ListenerBuilder::new().bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();

    // Spawn a connector in another thread with a short delay so the
    // accept_timeout call is already waiting when the connection arrives.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        // Ignore errors — the listener may have already returned.
        let _ = tst_srt::SocketBuilder::new().connect(format!("127.0.0.1:{port}"));
    });

    let result = listener.accept_timeout(Duration::from_secs(2));
    assert!(
        result.is_ok(),
        "accept should succeed; got {}",
        result.err().map(|e| format!("{e:?}")).unwrap_or_default()
    );
}
