//! Phase 3 Wave F Task 25 — concurrent unicast clients.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
use tst_rtp::{RtspClient, RtspServer, RtspServerBuilder};

fn make_muxer_cfg() -> MuxerConfig {
    let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
    prog.add_video(0x1011, VideoCodec::H264);
    let mut b = MuxerConfig::builder();
    b.add_program(prog.build());
    b.build().unwrap()
}

/// N concurrent clients all connect + DESCRIBE successfully.
#[test]
fn ten_concurrent_describes() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    let _mount = server.add_mount("/live", make_muxer_cfg()).unwrap();
    server.start().unwrap();
    let port = server.local_addr().unwrap().port();
    let url = std::sync::Arc::new(format!("rtsp://127.0.0.1:{port}/live"));

    let mut handles = vec![];
    for _ in 0..10 {
        let url = url.clone();
        handles.push(std::thread::spawn(move || {
            let mut client = RtspClient::connect(&url).unwrap();
            client.options().unwrap();
            let sdp = client.describe().unwrap();
            assert!(!sdp.media.is_empty());
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    server.stop().ok();
}

/// active_sessions count tracks correctly across concurrent connects + drops.
#[test]
fn active_sessions_count_reflects_concurrent_clients() {
    let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
    let _mount = server.add_mount("/live", make_muxer_cfg()).unwrap();
    server.start().unwrap();
    let port = server.local_addr().unwrap().port();
    let url = format!("rtsp://127.0.0.1:{port}/live");

    let clients: Vec<_> = (0..5)
        .map(|_| {
            let mut c = RtspClient::connect(&url).unwrap();
            c.options().unwrap();
            c
        })
        .collect();
    // Brief settle.
    std::thread::sleep(Duration::from_millis(200));
    assert!(server.stats().active_sessions >= 5);
    drop(clients);
    std::thread::sleep(Duration::from_millis(200));
    server.stop().ok();
}

/// Adversarial: an unauthenticated connection BURST must never push
/// `active_sessions` past `max_sessions`, and the excess connections must
/// be refused.
///
/// This reproduces the check-then-act race the atomic-reserve fix closes:
/// the single-threaded accept loop used to `load()` the counter, check the
/// cap, then spawn a task that did the `fetch_add` later. A burst that
/// arrives faster than the spawned tasks get polled to increment makes
/// every accept read the same stale low count, so all pass the check and
/// the cap is blown. With the fix the accept loop reserves the slot with a
/// single `fetch_add` BEFORE spawning, so a burst sees the running total.
///
/// Determinism: each accepted connection sends a PARTIAL request (no
/// terminating CRLFCRLF) so its session parks in the server read loop and
/// keeps holding its slot — the cap stays saturated for the whole
/// observation window. We open a large burst as fast as possible (no
/// per-connection handshake), then poll `active_sessions` repeatedly and
/// assert the cap is never exceeded. A refused connection is one the server
/// dropped (the TCP read returns EOF). Bounded + hang-proof: every socket op
/// has a read timeout and the whole test runs under a wall-clock ceiling.
#[test]
fn unauth_connection_burst_never_exceeds_max_sessions() {
    const CAP: usize = 2;
    const BURST: usize = 32;

    let mut builder = RtspServerBuilder::new("rtsp://127.0.0.1:0").unwrap();
    builder.max_sessions(CAP);
    let server = builder.build().unwrap();
    let _mount = server.add_mount("/live", make_muxer_cfg()).unwrap();
    server.start().unwrap();
    let addr = server.local_addr().unwrap();

    // Open the burst as fast as the kernel will let us — no handshake, just
    // raw connects + a partial line so the session parks holding its slot.
    // Keep the streams alive in `held` so accepted sessions stay open.
    let mut held: Vec<TcpStream> = Vec::with_capacity(BURST);
    for _ in 0..BURST {
        match TcpStream::connect(addr) {
            Ok(mut s) => {
                s.set_read_timeout(Some(Duration::from_millis(200))).ok();
                // Partial request — never terminated, so an accepted session
                // parks in the read loop and holds its slot.
                let _ = s.write_all(b"OPTIONS rtsp://127.0.0.1/live RTSP/1.0\r\n");
                held.push(s);
            }
            Err(_) => { /* kernel backlog full — still counts as refused */ }
        }
    }

    // Observe the live session count over a bounded window. It must NEVER
    // exceed the cap. (Before the fix this spikes well above CAP.)
    let mut max_observed = 0usize;
    let deadline = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < deadline {
        let n = server.stats().active_sessions;
        max_observed = max_observed.max(n);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        max_observed <= CAP,
        "active_sessions reached {max_observed}, exceeding max_sessions={CAP} — the burst bypassed the cap"
    );

    // The excess connections must be REFUSED: the server drops the TCP, so a
    // read on a refused socket returns EOF (0) or a connection-reset error,
    // whereas an ACCEPTED-and-parked session keeps the connection open (its
    // read times out with no bytes). With CAP accepted, the rest are refused.
    // (The load-bearing assertion is the cap above; this corroborates that the
    // overflow connections were actively dropped rather than silently parked.)
    let mut refused = 0usize;
    for s in held.iter_mut() {
        let mut buf = [0u8; 64];
        match s.read(&mut buf) {
            Ok(0) => refused += 1, // clean EOF — server dropped it.
            Ok(_) => {}            // got a response / partial echo — accepted.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                ) =>
            {
                refused += 1; // RST — server dropped it.
            }
            Err(_) => {} // WouldBlock/TimedOut — parked/accepted, not refused.
        }
    }
    assert!(
        refused >= BURST - CAP - 4,
        "expected most of the {BURST} burst connections to be refused (at most {CAP} accepted), only {refused} were"
    );

    // Drop every held connection — accepted sessions close, refused ones are
    // already gone. The counter must drain back to 0 (no leaked slots on the
    // accept-reserve / refuse / normal-close paths).
    drop(held);
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut final_count = usize::MAX;
    while Instant::now() < deadline {
        final_count = server.stats().active_sessions;
        if final_count == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        final_count, 0,
        "active_sessions leaked after all connections closed (stuck at {final_count})"
    );

    server.stop().ok();
}
