//! Phase 3 Wave H Task 3 — RFC 7826 §13.5.1 Notice 5402
//! ("Server-Initiated TEARDOWN") wire delivery.
//!
//! Drives a raw TCP client through OPTIONS / DESCRIBE / SETUP / PLAY
//! against a real `RtspServer`, then calls `server.stop()` on a worker
//! thread (so `stop()`'s drain sleep doesn't block this test thread)
//! and reads the bytes the server pushes out before the TCP closes.
//! Asserts the ANNOUNCE arrives with `Notice: 5402`.
//!
//! We can't use `RtspClient` here: it's a request/response client and
//! has no path to surface a server-initiated ANNOUNCE to the caller.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
use tst_rtp::RtspServer;

fn make_muxer_cfg() -> MuxerConfig {
    let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
    prog.add_video(0x1011, VideoCodec::H264);
    let mut b = MuxerConfig::builder();
    b.add_program(prog.build());
    b.build().unwrap()
}

/// Read from `stream` until `terminator` appears anywhere in the
/// accumulated buffer, EOF, or `deadline` passes. Returns the
/// accumulated bytes (which may not contain `terminator` if we hit EOF
/// or timeout). Uses a short per-read timeout so we don't block the
/// whole way to `deadline` after the peer goes idle.
fn read_until(stream: &mut TcpStream, terminator: &[u8], deadline: Instant) -> Vec<u8> {
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let mut buf = Vec::with_capacity(2048);
    while Instant::now() < deadline {
        let mut chunk = [0u8; 1024];
        match stream.read(&mut chunk) {
            Ok(0) => break, // EOF — peer closed.
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(terminator.len()).any(|w| w == terminator) {
                    return buf;
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(_) => break,
        }
    }
    buf
}

/// Issue an RTSP request over `stream`, then read the response until
/// the first CRLFCRLF (we don't exercise bodies > the header block in
/// this test — DESCRIBE responses with a body are read up through the
/// SDP terminator).
fn send_and_read_response(stream: &mut TcpStream, req: &[u8]) -> String {
    stream.write_all(req).unwrap();
    stream.flush().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    let buf = read_until(stream, b"\r\n\r\n", deadline);
    String::from_utf8_lossy(&buf).into_owned()
}

#[test]
fn stop_sends_notice_5402_announce_to_active_session() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    let _mount = server.add_mount("/live", make_muxer_cfg()).unwrap();
    server.start().unwrap();
    let port = server.local_addr().unwrap().port();

    // Connect a raw TCP client + drive OPTIONS → DESCRIBE → SETUP →
    // PLAY by hand. We use `?transport=tcp` so SETUP completes without
    // the server needing to allocate UDP sockets (which would expand
    // the test's port surface).
    let mut tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
    tcp.set_nodelay(true).unwrap();

    let opts = send_and_read_response(
        &mut tcp,
        format!("OPTIONS rtsp://127.0.0.1:{port}/live RTSP/1.0\r\nCSeq: 1\r\n\r\n").as_bytes(),
    );
    assert!(opts.contains("200 OK"), "OPTIONS failed: {opts}");

    let describe = send_and_read_response(
        &mut tcp,
        format!("DESCRIBE rtsp://127.0.0.1:{port}/live RTSP/1.0\r\nCSeq: 2\r\nAccept: application/sdp\r\n\r\n")
            .as_bytes(),
    );
    assert!(describe.contains("200 OK"), "DESCRIBE failed: {describe}");

    let setup = send_and_read_response(
        &mut tcp,
        format!(
            "SETUP rtsp://127.0.0.1:{port}/live RTSP/1.0\r\nCSeq: 3\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n"
        )
        .as_bytes(),
    );
    assert!(setup.contains("200 OK"), "SETUP failed: {setup}");

    // Extract the Session: header so we can assert it round-trips into
    // the ANNOUNCE.
    let session_id = setup
        .lines()
        .find_map(|l| {
            l.strip_prefix("Session: ")
                .or_else(|| l.strip_prefix("session: "))
        })
        .map(|s| s.split(';').next().unwrap().trim().to_string())
        .expect("SETUP response should carry Session header");

    let play = send_and_read_response(
        &mut tcp,
        format!(
            "PLAY rtsp://127.0.0.1:{port}/live RTSP/1.0\r\nCSeq: 4\r\nSession: {session_id}\r\n\r\n"
        )
        .as_bytes(),
    );
    assert!(play.contains("200 OK"), "PLAY failed: {play}");

    // Kick stop() off on a worker thread — stop() blocks for the full
    // graceful drain (~1.1 s by default), and we want to read the
    // server's outbound bytes from this thread while that's happening.
    // We can't `drop(server)` from another thread without moving it;
    // wrap in an Arc so both threads can observe.
    let server = std::sync::Arc::new(server);
    let server_clone = server.clone();
    let stop_thread = std::thread::spawn(move || {
        server_clone.stop().unwrap();
    });

    // Now read whatever bytes the server pushes before TCP close.
    // stop() should have queued the ANNOUNCE before this read starts;
    // we give ourselves up to 3 s of read budget.
    let deadline = Instant::now() + Duration::from_secs(3);
    let buf = read_until(&mut tcp, b"\r\n\r\n", deadline);
    let text = String::from_utf8_lossy(&buf);

    stop_thread.join().unwrap();

    // Assertions on the received bytes.
    assert!(
        text.starts_with("ANNOUNCE "),
        "expected ANNOUNCE start-line; got: {text:?}"
    );
    assert!(
        text.contains("Notice: 5402"),
        "expected `Notice: 5402` header; got: {text:?}"
    );
    assert!(
        text.contains("Server-Initiated TEARDOWN"),
        "expected Notice reason phrase; got: {text:?}"
    );
    assert!(
        text.contains(&format!("Session: {session_id}")),
        "expected Session: {session_id}; got: {text:?}"
    );
    assert!(
        text.contains("/live"),
        "expected mount path /live in request URI; got: {text:?}"
    );
    assert!(
        text.contains("CSeq:"),
        "expected server-allocated CSeq; got: {text:?}"
    );

    // Drop the server reference so its Drop hard-cancels the runtime.
    drop(server);
}

#[test]
fn stop_without_active_sessions_is_clean_noop_path() {
    // Smoke test: stop() with zero active sessions should not iterate
    // anything, not panic, and complete in graceful_shutdown_drain.
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    let _mount = server.add_mount("/live", make_muxer_cfg()).unwrap();
    server.start().unwrap();
    let started = Instant::now();
    server.stop().unwrap();
    // Loose upper bound — drain is 100ms + 1s safety buffer in the
    // default builder + per-session timeouts (none here).
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "stop() with no sessions took too long: {:?}",
        started.elapsed()
    );
}
