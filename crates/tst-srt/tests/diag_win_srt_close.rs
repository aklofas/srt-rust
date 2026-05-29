//! TEMPORARY DIAGNOSTIC — Windows SRT loopback hang (plan #65 root-cause).
//!
//! Not a correctness gate: it asserts only that the loopback plumbing works,
//! then streams an `eprintln!` timeline of exactly what `srt_recv` returns
//! after the sending peer closes. Run with `-- --nocapture --test-threads=1`;
//! the SIGNAL is the timeline in the CI log, used to localize the Windows-only
//! hang documented in `project_plan_65_windows_runtime_test_deferral` /
//! `project_ci_known_flakes_2026_05_29`.
//!
//! It isolates the SRT layer (Socket/Listener) from the mux/demux pipeline so
//! the result points squarely at libsrt's blocking-recv-wakeup-on-peer-close
//! behaviour. Every wait is bounded (per-phase iteration cap + an 8 s
//! watchdog) so it can never hang a CI step. **Delete this file once the root
//! cause is confirmed and fixed.**

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use tst_srt::{ListenerBuilder, RecvError, SocketBuilder};

fn loopback_ok() -> bool {
    std::env::var_os("SKIP_LOOPBACK").is_none() && TcpListener::bind("127.0.0.1:0").is_ok()
}

fn wait_ready(flag: &AtomicBool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !flag.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "listener never signaled ready");
        thread::sleep(Duration::from_millis(5));
    }
}

/// PHASE 1 — accepted socket with a finite 1 s `SRTO_RCVTIMEO`. After the
/// caller closes, does `srt_recv` ever surface `ConnectionBroken`, and how
/// soon? This distinguishes "the timed wait fires and broken IS detected when
/// polled" (=> a finite recv timeout is a viable fix) from "broken is never
/// surfaced even when actively polled" (=> deeper libsrt/winsock breakage).
#[test]
fn diag_phase1_polled_recv_after_close() {
    if !loopback_ok() {
        eprintln!("SKIP: loopback unavailable");
        return;
    }

    let lb = ListenerBuilder::new();
    let mut listener = lb.bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("local_addr").port();

    let ready = Arc::new(AtomicBool::new(false));
    let r = ready.clone();
    let acc = thread::spawn(move || {
        r.store(true, Ordering::SeqCst);
        let (mut sock, _peer) = listener.accept().expect("accept");
        sock.set_recv_timeout(Some(Duration::from_millis(1000)))
            .expect("set rcvtimeo");
        let start = Instant::now();
        let mut buf = vec![0u8; 4096];
        let mut surfaced: Option<u128> = None;
        for i in 0..15 {
            let res = sock.recv(&mut buf);
            let ms = start.elapsed().as_millis();
            match &res {
                Ok(n) => eprintln!("[diag P1] iter={i:<2} t={ms:>5}ms  Ok({n})"),
                Err(RecvError::TimedOut) => {
                    eprintln!("[diag P1] iter={i:<2} t={ms:>5}ms  TimedOut")
                }
                Err(RecvError::ConnectionBroken) => {
                    eprintln!(
                        "[diag P1] iter={i:<2} t={ms:>5}ms  ConnectionBroken  <== peer-close surfaced"
                    );
                    surfaced = Some(ms);
                    break;
                }
                Err(e) => {
                    eprintln!("[diag P1] iter={i:<2} t={ms:>5}ms  Other: {e:?}");
                    surfaced = Some(ms);
                    break;
                }
            }
        }
        match surfaced {
            Some(ms) => eprintln!(
                "[diag P1] RESULT: recv surfaced a non-data error ~{ms}ms into the loop \
                 -> polled broken-detection WORKS (finite RCVTIMEO is a viable fix)"
            ),
            None => eprintln!(
                "[diag P1] RESULT: recv NEVER surfaced broken across 15 polls (~15s) \
                 -> Windows never marks the socket broken even when polled (deeper issue)"
            ),
        }
    });

    wait_ready(&ready);
    let mut cb = SocketBuilder::new();
    cb.recv_timeout(Duration::from_secs(5));
    let mut caller = cb.connect(format!("127.0.0.1:{port}")).expect("connect");
    for i in 0..3u8 {
        caller.send(format!("p1-msg{i}").as_bytes()).expect("send");
    }
    thread::sleep(Duration::from_millis(600));
    eprintln!("[diag P1] caller closing now");
    let _ = caller.close();
    acc.join().expect("phase1 accept thread");
}

/// PHASE 2 — accepted socket with the DEFAULT (infinite) `RCVTIMEO`, the exact
/// condition the real receiver uses. A watchdog measures how long the blocking
/// `srt_recv` takes to return after the caller closes. On Linux/macOS it
/// returns near-instantly with `ConnectionBroken`; the reported Windows hang is
/// this recv never returning.
#[test]
fn diag_phase2_default_blocking_recv_after_close() {
    if !loopback_ok() {
        eprintln!("SKIP: loopback unavailable");
        return;
    }

    let lb = ListenerBuilder::new();
    let mut listener = lb.bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("local_addr").port();

    let ready = Arc::new(AtomicBool::new(false));
    let r = ready.clone();
    let (tx, rx) = mpsc::channel::<String>();
    // Detached on purpose: if the blocking recv hangs (the bug), this thread
    // stays parked in srt_recv and is reaped at process exit. The watchdog
    // below bounds the TEST, not this thread.
    let _acc = thread::spawn(move || {
        r.store(true, Ordering::SeqCst);
        let (mut sock, _peer) = listener.accept().expect("accept");
        // Deliberately do NOT set a recv timeout -> infinite blocking recv.
        let mut buf = vec![0u8; 4096];
        loop {
            let res = sock.recv(&mut buf);
            let line = match &res {
                Ok(n) => format!("Ok({n})"),
                Err(e) => format!("Err({e:?})"),
            };
            if tx.send(line).is_err() {
                return;
            }
            if res.is_err() {
                return;
            }
        }
    });

    wait_ready(&ready);
    let cb = SocketBuilder::new();
    let mut caller = cb.connect(format!("127.0.0.1:{port}")).expect("connect");
    for i in 0..3u8 {
        caller.send(format!("p2-msg{i}").as_bytes()).expect("send");
    }
    thread::sleep(Duration::from_millis(600));
    eprintln!("[diag P2] caller closing now (default infinite-RCVTIMEO blocking recv)");
    let _ = caller.close();

    let start = Instant::now();
    let mut drained = 0u32;
    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(s) if s.starts_with("Ok") => {
                drained += 1;
                eprintln!(
                    "[diag P2] t={:>5}ms  recv {s} (drained data #{drained})",
                    start.elapsed().as_millis()
                );
            }
            Ok(s) => {
                let ms = start.elapsed().as_millis();
                eprintln!(
                    "[diag P2] t={ms:>5}ms  recv {s}  <== blocking recv returned after close"
                );
                eprintln!(
                    "[diag P2] RESULT: blocking recv returned ~{ms}ms after peer close -> NO hang"
                );
                break;
            }
            Err(_) => {
                let ms = start.elapsed().as_millis();
                eprintln!("[diag P2] t={ms:>5}ms  still blocked (no recv return yet)");
                if ms > 8000 {
                    eprintln!(
                        "[diag P2] RESULT: HANG CONFIRMED -- blocking recv did not return \
                         within 8s of peer close"
                    );
                    break;
                }
            }
        }
    }
}
