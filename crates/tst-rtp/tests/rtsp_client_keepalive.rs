//! Verifies the rtsp-keepalive background thread emits OPTIONS pings on
//! the control TCP within `session_timeout / 2` (or the explicit
//! override), and that drop-cleanup joins the thread cleanly.
//!
//! Uses a hand-rolled TCP `accept()` loop instead of a full RTSP server
//! — we only care about observing the wire format the keepalive emits.

use std::io::{Read, Write};
use std::net::TcpListener;

#[test]
fn keepalive_thread_pings_within_session_timeout() {
    // Bind a loopback listener and capture the port for the URL.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    // Background server: accept one connection, expect the keepalive
    // thread's OPTIONS ping at CSeq 1000001 within ~10 s, then reply
    // 200 OK so the keepalive's read-loop sees a clean close on drop.
    let h = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        sock.set_read_timeout(Some(std::time::Duration::from_millis(500)))
            .ok();
        let mut buf = vec![0u8; 4096];
        let mut total = String::new();
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(10) {
            let n = match sock.read(&mut buf) {
                Ok(n) => n,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(_) => break,
            };
            if n == 0 {
                break;
            }
            total.push_str(std::str::from_utf8(&buf[..n]).unwrap_or(""));
            // The encoder canonicalizes header names via per-segment
            // first-letter capitalization — `cseq` renders as `Cseq:`
            // (not `CSeq:`). Match either spelling case-insensitively
            // since RTSP headers are case-insensitive per RFC 2326 §12.
            let lower = total.to_ascii_lowercase();
            if lower.contains("options") && lower.contains("cseq: 1000001") {
                // Reply 200 OK and return.
                let _ = sock.write_all(b"RTSP/1.0 200 OK\r\nCSeq: 1000001\r\n\r\n");
                return;
            }
        }
        panic!("no OPTIONS ping seen within 10 s; got: {total}");
    });

    // Build the client. RtspClientBuilder is Task 16 (parallel land);
    // until that merges, drive the keepalive via the lower-level
    // `spawn_keepalive_if_needed` helper directly.
    let url = format!("rtsp://127.0.0.1:{port}/test");
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    client.spawn_keepalive_if_needed(Some(std::time::Duration::from_secs(2)));

    // Hold the client for 5 s — at a 2 s cadence the keepalive
    // emits its first ping at ~t+2 s, well inside the 10 s budget.
    std::thread::sleep(std::time::Duration::from_secs(5));
    drop(client);
    h.join().unwrap();
}
