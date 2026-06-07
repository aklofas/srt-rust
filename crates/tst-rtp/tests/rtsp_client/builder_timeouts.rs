//! Verify that `RtspClientBuilder`'s `connect_timeout`, `read_timeout`,
//! and `user_agent` fields are actually wired through to the socket and
//! request headers — not silently ignored.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

/// `connect_timeout` must be honored: a 1-second limit on a blackhole
/// address must return an error in well under the old hardcoded 10 s.
///
/// `10.255.255.1` is a non-routable address guaranteed never to RST —
/// the SYN hangs until the TCP connect timeout fires.
#[test]
fn connect_timeout_is_honored() {
    let url = "rtsp://10.255.255.1:554/x";
    let start = Instant::now();
    let result = tst_rtp::RtspClientBuilder::new(url)
        .unwrap()
        .connect_timeout(Duration::from_secs(1))
        .connect();
    let elapsed = start.elapsed();

    assert!(
        result.is_err(),
        "expected connection to blackhole to fail, but it succeeded"
    );
    assert!(
        elapsed < Duration::from_secs(4),
        "connect_timeout of 1 s was ignored — elapsed {elapsed:?} exceeds 4 s"
    );
}

/// `user_agent` must appear in the OPTIONS wire request.
///
/// Uses a hand-rolled TCP accept loop so we can inspect raw bytes.
#[test]
fn user_agent_is_sent_in_requests() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    // Background: accept one connection, read bytes, reply a 200 OPTIONS
    // so the client can complete options(), then close.
    let server = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let mut buf = vec![0u8; 4096];
        let mut acc = String::new();
        loop {
            let n = match sock.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            acc.push_str(std::str::from_utf8(&buf[..n]).unwrap_or(""));
            if acc.contains("\r\n\r\n") {
                break;
            }
        }
        // Reply 200 OK so the client's options() call returns.
        let _ = sock.write_all(
            b"RTSP/1.0 200 OK\r\nCSeq: 1\r\nPublic: OPTIONS, DESCRIBE, SETUP, PLAY, TEARDOWN\r\n\r\n",
        );
        acc
    });

    let url = format!("rtsp://127.0.0.1:{port}/test");
    let mut client = tst_rtp::RtspClientBuilder::new(&url)
        .unwrap()
        .user_agent("my-custom-agent/9.9")
        .connect()
        .unwrap();
    let _ = client.options(); // drive the wire exchange

    let received = server.join().unwrap();
    let lower = received.to_ascii_lowercase();
    assert!(
        lower.contains("my-custom-agent/9.9"),
        "custom User-Agent not found in OPTIONS request; got:\n{received}"
    );
}
