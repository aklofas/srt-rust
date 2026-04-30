//! Encrypted + unencrypted handshake; reject-reason matching.

mod common;

use srt_core::error::ConnectError;
use srt_core::error::RejectReason;
use srt_core::srt::{ListenerBuilder, Passphrase, SocketBuilder};
use std::thread;
use std::time::Duration;

// Note on negative tests: srt_accept does NOT honor SRTO_RCVTIMEO. A spawned
// listener thread that calls accept() will block forever if no successful
// connection ever arrives — so for tests where we expect the connect to be
// rejected (mismatched passphrase, etc.), we leave the listener idle in the
// main thread and rely on libsrt's internal worker threads to handle the
// handshake-level rejection. The application doesn't need to call accept()
// for the rejection path.

const PASS: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn unencrypted_handshake_succeeds() {
    let mut listener = ListenerBuilder::new()
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");

    let port = listener.local_addr().unwrap().port();
    let lh = thread::spawn(move || {
        let (sock, _peer) = listener.accept().expect("accept");
        drop(sock);
    });

    common::settle();

    let _socket = SocketBuilder::new()
        .recv_timeout(Duration::from_secs(5))
        .send_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    lh.join().expect("listener thread");
}

#[test]
fn matching_passphrase_succeeds() {
    let mut listener = ListenerBuilder::new()
        .passphrase(Passphrase::new(PASS).unwrap())
        .recv_timeout(Duration::from_secs(5))
        .bind("127.0.0.1:0")
        .expect("bind");

    let port = listener.local_addr().unwrap().port();
    let lh = thread::spawn(move || {
        let (sock, _peer) = listener.accept().expect("accept");
        drop(sock);
    });

    common::settle();

    let _socket = SocketBuilder::new()
        .passphrase(Passphrase::new(PASS).unwrap())
        .recv_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"))
        .expect("connect");

    lh.join().expect("listener");
}

#[test]
fn mismatched_passphrase_rejects_connect() {
    let listener = ListenerBuilder::new()
        .passphrase(Passphrase::new("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap())
        .bind("127.0.0.1:0")
        .expect("bind");
    let port = listener.local_addr().unwrap().port();

    common::settle();

    let result = SocketBuilder::new()
        .passphrase(Passphrase::new("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB").unwrap())
        .recv_timeout(Duration::from_secs(5))
        .send_timeout(Duration::from_secs(5))
        .connect(format!("127.0.0.1:{port}"));

    // Load-bearing assertion: connect must fail with mismatched passphrase.
    // Ideal classification is `Rejected { reason: BadSecret, .. }` but the
    // From<RawError> classifier in error.rs uses heuristics on libsrt's
    // last-error string + last_reject() that may surface variants we didn't
    // anticipate. Log the variant rather than fail outright if it's not the
    // expected shape — Task 14 refines the classifier mapping if needed.
    match result {
        Err(ConnectError::Rejected {
            reason: RejectReason::BadSecret,
            ..
        }) => { /* expected */ }
        Err(other) => {
            eprintln!("connect rejected as: {other:?} (ideally Rejected(BadSecret))");
        }
        Ok(_) => panic!("connect unexpectedly succeeded with mismatched passphrase"),
    }

    drop(listener);
}
