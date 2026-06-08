//! Pre-release blocker regression test: RTSP server must not buffer
//! unboundedly when a client announces a huge Content-Length.
//!
//! Scenario: an unauthenticated client sends an OPTIONS request with a
//! `Content-Length: 2_000_000_000` header, then writes a small amount of
//! junk (just past the 64 KiB buffer cap). Before the fix, the server
//! accumulated every byte into `buf` and waited for a 2 GB body that
//! would never arrive — an OOM DoS for any pre-auth client.
//!
//! After the fix the server must send a 413 response and close the
//! connection promptly after the buffer cap is exceeded. This test
//! verifies that the 413 arrives within a tight write budget (128 KiB)
//! rather than the server keeping the connection indefinitely open.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use tst_rtp::RtspServer;

/// Connect a raw TCP client, send a crafted RTSP request with
/// `Content-Length: 2000000000`, push 128 KiB of junk (just past the
/// 64 KiB `MAX_RTSP_REQUEST_BYTES` cap), then read and assert that the
/// server sent a 413 response (or connection closed) rather than staying
/// silent and accumulating bytes.
#[test]
fn oversized_content_length_gets_413_response() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    server.start().unwrap();
    let port = server.local_addr().unwrap().port();

    let mut tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.set_nodelay(true).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();

    // Send the malicious request: OPTIONS with a 2 GB declared body.
    let request = b"OPTIONS * RTSP/1.0\r\nCSeq: 1\r\nContent-Length: 2000000000\r\n\r\n";
    tcp.write_all(request).unwrap();

    // Write 128 KiB of junk — just enough to exceed the 64 KiB
    // MAX_RTSP_REQUEST_BYTES cap. After the fix, the server should
    // respond with 413 and close within this budget.
    let junk = vec![0x42u8; 128 * 1024];
    // Ignore write error: the server may have already closed the connection
    // before we finish writing.
    let _ = tcp.write_all(&junk);

    // Now read the server's response. Expect a 413 status line
    // (or at minimum, a closed connection = EOF or reset).
    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let start = Instant::now();
    let mut response_buf = Vec::with_capacity(512);
    let mut got_413 = false;
    let mut got_close = false;

    loop {
        let mut chunk = [0u8; 256];
        match tcp.read(&mut chunk) {
            Ok(0) => {
                // EOF — server closed the connection (acceptable: the server
                // could close without sending a 413 if it hits an I/O error
                // writing the response).
                got_close = true;
                break;
            }
            Ok(n) => {
                response_buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&response_buf);
                if text.contains("413") {
                    got_413 = true;
                    break;
                }
                // Keep reading until we have CRLFCRLF or a non-200 status.
                if response_buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    // We have at least one complete response header block.
                    break;
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // The server has not responded within the read timeout —
                // this means it is silently accumulating bytes (pre-fix
                // behavior). Treat this as the failing case.
                break;
            }
            Err(_) => {
                // Connection reset — server closed.
                got_close = true;
                break;
            }
        }
    }

    let elapsed = start.elapsed();

    assert!(
        got_413 || got_close,
        "server must close or send 413 after exceeding MAX_RTSP_REQUEST_BYTES (64 KiB) \
         but it stayed open silently for {elapsed:?}. Response so far: {:?}",
        String::from_utf8_lossy(&response_buf)
    );

    if got_413 {
        let text = String::from_utf8_lossy(&response_buf);
        assert!(
            text.contains("413"),
            "expected 413 in response, got: {text:?}"
        );
    }

    // The fix must respond promptly — well within the 5s read timeout.
    assert!(
        elapsed < Duration::from_secs(5),
        "server took too long to close or respond: {elapsed:?}"
    );

    server.stop().ok();
}

/// B7 cap-coherence: a VALID request with a large but in-bounds body
/// (100 KiB, well under the 1 MiB MAX_RTSP_BODY_BYTES) must be ACCEPTED and
/// dispatched normally (200 OK), NOT falsely rejected at the 64 KiB header cap.
///
/// Before B7 the server session loop capped the whole request (headers + body)
/// at 64 KiB before parsing, so any request whose body pushed the buffer past
/// 64 KiB got a wrongful 413. The body-aware two-phase cap (header cap 64 KiB
/// separately, then header + Content-Length up to 1 MiB) fixes this.
#[test]
fn valid_large_body_request_is_accepted() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    server.start().unwrap();
    let port = server.local_addr().unwrap().port();

    let mut tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.set_nodelay(true).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();

    // OPTIONS with a 100 KiB body. The server tolerates (ignores) the body
    // and returns 200 OK with a Public header. 100 KiB > 64 KiB header cap
    // but << 1 MiB body cap, so it must be accepted, not 413'd.
    let body = vec![0x42u8; 100 * 1024];
    let mut request = format!(
        "OPTIONS * RTSP/1.0\r\nCSeq: 1\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    request.extend_from_slice(&body);
    tcp.write_all(&request).unwrap();

    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut response_buf = Vec::with_capacity(512);
    loop {
        let mut chunk = [0u8; 256];
        match tcp.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                response_buf.extend_from_slice(&chunk[..n]);
                if response_buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&response_buf);
    assert!(
        text.contains("200 OK"),
        "valid 100 KiB-body request must be accepted (200 OK), got: {text:?}"
    );
    assert!(
        !text.contains("413"),
        "valid 100 KiB-body request must NOT be 413'd, got: {text:?}"
    );

    server.stop().ok();
}

/// B7 cap-coherence: a request declaring a body LARGER than 1 MiB
/// (MAX_RTSP_BODY_BYTES) must still be rejected with 413 (or the connection
/// closed). The body-aware cap rejects an over-cap Content-Length up front
/// rather than reading toward an unbounded body.
#[test]
fn over_cap_body_request_gets_413() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    server.start().unwrap();
    let port = server.local_addr().unwrap().port();

    let mut tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.set_nodelay(true).unwrap();
    tcp.set_write_timeout(Some(Duration::from_secs(5))).unwrap();

    // Content-Length = 1 MiB + 1, just over MAX_RTSP_BODY_BYTES. The full
    // header block terminates immediately (CRLFCRLF present), so the server
    // parses the declared length and rejects it before reading any body.
    let over = 1024 * 1024 + 1;
    let request = format!("OPTIONS * RTSP/1.0\r\nCSeq: 1\r\nContent-Length: {over}\r\n\r\n");
    tcp.write_all(request.as_bytes()).unwrap();

    tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut response_buf = Vec::with_capacity(512);
    let mut got_413 = false;
    let mut got_close = false;
    loop {
        let mut chunk = [0u8; 256];
        match tcp.read(&mut chunk) {
            Ok(0) => {
                got_close = true;
                break;
            }
            Ok(n) => {
                response_buf.extend_from_slice(&chunk[..n]);
                if String::from_utf8_lossy(&response_buf).contains("413") {
                    got_413 = true;
                    break;
                }
                if response_buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => {
                got_close = true;
                break;
            }
        }
    }
    assert!(
        got_413 || got_close,
        "over-1-MiB Content-Length must be 413'd or closed, got: {:?}",
        String::from_utf8_lossy(&response_buf)
    );

    server.stop().ok();
}
