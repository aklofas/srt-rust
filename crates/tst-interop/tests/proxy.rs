//! Integration tests for `proxy::run`: transparent relay (byte
//! transparency at loss=0), scheduled outage windows, and an
//! end-to-end SRT-through-proxy cell (SRT's own retransmission
//! recovering from a lossy/jittered link).
//!
//! # nextest group placement
//!
//! This binary's name (`proxy`) doesn't match any of
//! `.config/nextest.toml`'s `binary(...)` filter entries, so no test
//! here lands in the capped `network` test-group by binary name alone.
//! Following `tests/serve.rs`'s and `tests/loopback.rs`'s own convention
//! (deliberately naming a timing-sensitive network test so its NAME
//! matches one of the `test(...)` filter entries, opting it into the
//! serialized group), only
//! [`srt_round_trip_through_lossy_proxy_recovers_via_retransmission`]
//! does that (via `round_trip`) — it's the one cell in this file where
//! CPU/port contention from unrelated parallel tests could plausibly
//! perturb a real SRT handshake + TSBPD delivery under injected loss.
//! It therefore gets the `network` group's 10s-period/2-terminate (20s
//! hard kill; 40s on Windows via the platform override) instead of the
//! default profile's 30s/2 (60s) global safety net.
//!
//! The other two tests ([`transparent_relay_preserves_order_and_bytes`],
//! [`outage_windows_produce_periodic_gaps`]) are left unmatched
//! (default group, full parallelism): both use ephemeral OS-assigned
//! ports (no cross-test port contention) and are written with generous
//! tolerances rather than tight wall-clock assertions, so ordinary CPU
//! contention from sibling tests shouldn't flake them. `outage_windows_
//! produce_periodic_gaps` is the one with any real wall-clock-shape
//! assertion (a gap between arrivals) — if it's ever observed to flake
//! under load, promoting it into the `network` group (rename to include
//! e.g. `round_trip`, or add a `.config/nextest.toml` override) is the
//! fix; nothing about that decision belongs in `proxy.rs` itself.
//!
//! Deterministic-test policy (mirrors `tests/loopback.rs`/`tests/
//! serve.rs`): ports are ephemeral (OS-assigned via a throwaway bind);
//! `proxy::run`'s `on_bound` callback (not stdout-JSON parsing) is how
//! these tests learn the proxy's actual listen port; every thread join
//! is timeout-bounded, never a bare `.join()`.

use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use tst_interop::impair::ImpairConfig;
use tst_interop::report_types::CellMetrics;
use tst_interop::{profiles, proxy, recv, send};

/// Ask the OS for an unused port via a throwaway UDP bind — mirrors
/// `tests/loopback.rs::free_port` (this crate's convention: small
/// per-test-file duplicated helpers rather than a shared test-utils
/// module).
fn free_port() -> u16 {
    let probe = UdpSocket::bind("127.0.0.1:0").expect("bind probe port");
    probe.local_addr().expect("read probe local_addr").port()
}

/// Poll `handle.is_finished()` until it's done or `timeout` elapses —
/// mirrors `tests/loopback.rs::join_with_timeout`.
fn join_with_timeout<T>(handle: thread::JoinHandle<T>, timeout: Duration) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if handle.is_finished() {
            return handle.join().expect("spawned thread panicked");
        }
        assert!(
            Instant::now() < deadline,
            "thread did not finish within {timeout:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// Retry `send::run` on a bounded budget — mirrors `tests/loopback.rs::
/// send_with_retry` (a failed SRT `connect()` attempt sends no data at
/// all, so retrying the whole call never risks a double/partial send).
fn send_with_retry(
    profile: &profiles::Profile,
    url: &str,
    seconds: f64,
    budget: Duration,
) -> CellMetrics {
    let deadline = Instant::now() + budget;
    loop {
        match send::run(profile, url, seconds, None) {
            Ok(metrics) => return metrics,
            Err(e) => {
                assert!(
                    Instant::now() < deadline,
                    "send::run kept failing until the retry budget ({budget:?}) ran out: {e}"
                );
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Spawn `proxy::run` on its own thread and block until it reports its
/// bound listen address via `on_bound` — the in-process bound-address
/// discovery these tests use instead of parsing the CLI's stdout JSON
/// line.
fn spawn_proxy(
    listen: SocketAddr,
    forward: SocketAddr,
    cfg: ImpairConfig,
    stats_json: Option<PathBuf>,
    run_seconds: u64,
) -> (
    SocketAddr,
    thread::JoinHandle<Result<proxy::ProxyStats, String>>,
) {
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        proxy::run(
            listen,
            forward,
            cfg,
            stats_json,
            Some(run_seconds),
            Some(Box::new(move |addr| {
                let _ = tx.send(addr);
            })),
        )
    });
    let bound = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("proxy must report its bound address via on_bound");
    (bound, handle)
}

/// Transparent mode (`ImpairConfig::default()`, i.e. loss/dup/reorder/
/// jitter all off) is the byte-transparent tier's foundation: 500
/// distinct datagrams sent through the proxy must arrive at the real
/// destination in the exact order they were sent, byte-identical.
#[test]
fn transparent_relay_preserves_order_and_bytes() {
    let dest_sock = UdpSocket::bind("127.0.0.1:0").expect("bind destination socket");
    dest_sock
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("set_read_timeout");
    let dest_addr = dest_sock.local_addr().expect("destination local_addr");

    let (proxy_addr, proxy_handle) = spawn_proxy(
        "127.0.0.1:0".parse().unwrap(),
        dest_addr,
        ImpairConfig::default(),
        None,
        3,
    );

    let payloads: Vec<Vec<u8>> = (0..500u32)
        .map(|i| format!("packet-{i:04}").into_bytes())
        .collect();
    let expected_count = payloads.len();

    // Read on its own thread, CONCURRENTLY with the send loop below,
    // rather than sending all 500 first and only then reading: the
    // latter would let all 500 datagrams pile up in the destination
    // socket's kernel receive buffer before this test ever drains it,
    // risking an overflow-driven loss under a burst this size (see the
    // outage-window test's own note for the closely related "arrival
    // timing gets destroyed by batch-draining" failure mode this same
    // pattern avoids).
    let receiver = thread::spawn(move || {
        let mut received = Vec::with_capacity(expected_count);
        let mut buf = [0u8; 256];
        let deadline = Instant::now() + Duration::from_secs(3);
        while received.len() < expected_count && Instant::now() < deadline {
            match dest_sock.recv(&mut buf) {
                Ok(n) => received.push(buf[..n].to_vec()),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => panic!("recv error: {e}"),
            }
        }
        received
    });

    // A small per-send pace (500us -> ~250ms total for 500 packets)
    // rather than firing all 500 in a zero-delay tight loop: this
    // single-threaded proxy's recv/decide/send loop has real per-packet
    // syscall + scheduling overhead, and an instantaneous burst this
    // size can outrun it fast enough to overflow the LISTEN socket's own
    // kernel receive buffer before the proxy ever gets scheduled to
    // drain it — genuine UDP behavior, not a proxy bug, but not what
    // this test means to exercise either (it's checking transparent-mode
    // byte/order fidelity, not this crate's raw burst throughput
    // ceiling).
    let sender = UdpSocket::bind("127.0.0.1:0").expect("bind sender socket");
    for p in &payloads {
        sender
            .send_to(p, proxy_addr)
            .expect("send datagram to proxy");
        thread::sleep(Duration::from_micros(500));
    }

    let received = join_with_timeout(receiver, Duration::from_secs(10));

    let stats =
        join_with_timeout(proxy_handle, Duration::from_secs(10)).expect("proxy::run must succeed");

    assert_eq!(
        received, payloads,
        "a transparent proxy must preserve order and content exactly"
    );

    assert_eq!(stats.forwarded, payloads.len() as u64);
    assert_eq!(stats.dropped, 0);
    assert_eq!(stats.duped, 0);
}

/// `outage_period_s=2, outage_dur_s=1` at 10Hz for ~4s: the receiver
/// must observe periodic ~1s gaps where the outage windows suppressed
/// traffic, and the stats JSON written at exit must agree with `proxy::
/// run`'s own returned counters.
#[test]
fn outage_windows_produce_periodic_gaps() {
    let dest_sock = UdpSocket::bind("127.0.0.1:0").expect("bind destination socket");
    dest_sock
        .set_read_timeout(Some(Duration::from_millis(200)))
        .expect("set_read_timeout");
    let dest_addr = dest_sock.local_addr().expect("destination local_addr");

    let stats_path = std::env::temp_dir().join(format!(
        "tst-interop-proxy-outage-stats-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before UNIX_EPOCH")
            .as_nanos(),
    ));

    let cfg = ImpairConfig {
        outage_period_s: Some(2),
        outage_dur_s: 1,
        ..ImpairConfig::default()
    };
    let (proxy_addr, proxy_handle) = spawn_proxy(
        "127.0.0.1:0".parse().unwrap(),
        dest_addr,
        cfg,
        Some(stats_path.clone()),
        6,
    );

    const HZ: u32 = 10;
    const TOTAL: u32 = HZ * 4; // ~4s at 10Hz
    // Send window is ~4s; give the collector a further 2s margin past
    // that for the last few packets' proxy-relay + wire latency to
    // land. Runs on its own thread, CONCURRENTLY with the paced send
    // loop below — reading only AFTER the send loop finished would just
    // batch-drain whatever piled up in the kernel receive buffer over
    // the whole 4s window, and every arrival would then read back nearly
    // simultaneously: the real wall-clock gaps this test's assertions
    // depend on only exist if arrivals are timestamped as they actually
    // land, not after the fact.
    let collect_for = Duration::from_secs(4) + Duration::from_secs(2);
    let receiver = thread::spawn(move || {
        let deadline = Instant::now() + collect_for;
        let mut arrivals: Vec<Instant> = Vec::new();
        let mut buf = [0u8; 16];
        while Instant::now() < deadline {
            match dest_sock.recv(&mut buf) {
                Ok(_) => arrivals.push(Instant::now()),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => panic!("recv error: {e}"),
            }
        }
        arrivals
    });

    let sender = UdpSocket::bind("127.0.0.1:0").expect("bind sender socket");
    let send_start = Instant::now();
    for i in 0..TOTAL {
        sender
            .send_to(&i.to_le_bytes(), proxy_addr)
            .expect("send datagram to proxy");
        let target = Duration::from_millis((i as u64 + 1) * 1000 / HZ as u64);
        let elapsed = send_start.elapsed();
        if target > elapsed {
            thread::sleep(target - elapsed);
        }
    }

    let arrivals = join_with_timeout(receiver, Duration::from_secs(10));

    assert!(
        !arrivals.is_empty(),
        "expected at least some packets to survive the up windows"
    );
    assert!(
        arrivals.len() < TOTAL as usize,
        "expected some packets to be dropped by the outage windows, got all {}/{TOTAL}",
        arrivals.len()
    );

    // At least one gap between consecutive arrivals should be
    // consistent with a ~1s outage window (nominal ~100ms between
    // packets while a window is up) — generous threshold (500ms, half
    // the configured 1s outage) so ordinary scheduling jitter can't
    // flake this.
    let max_gap = arrivals
        .windows(2)
        .map(|w| w[1].duration_since(w[0]))
        .max()
        .unwrap_or(Duration::ZERO);
    assert!(
        max_gap >= Duration::from_millis(500),
        "expected an outage-sized gap (>=500ms) between arrivals, largest observed was {max_gap:?}"
    );

    let stats =
        join_with_timeout(proxy_handle, Duration::from_secs(10)).expect("proxy::run must succeed");
    assert!(
        stats.dropped > 0,
        "expected some packets to be dropped by the outage windows"
    );
    assert!(
        stats.forwarded > 0,
        "expected some packets to survive the up windows"
    );
    // Roughly half of a 4s send window falls inside outage windows
    // (period=2s, dur=1s) — generous 30%/70% band around that 50%
    // nominal split to tolerate where the 4s window happens to land
    // relative to the proxy's own outage-window phase.
    let total = stats.dropped + stats.forwarded;
    let dropped_frac = stats.dropped as f64 / total as f64;
    assert!(
        (0.3..=0.7).contains(&dropped_frac),
        "dropped fraction {dropped_frac:.2} out of the expected ~50% band (dropped={}, forwarded={})",
        stats.dropped,
        stats.forwarded
    );

    let json = std::fs::read_to_string(&stats_path).expect("read stats json written at exit");
    let parsed: proxy::ProxyStats = serde_json::from_str(&json).expect("stats json must parse");
    assert_eq!(
        parsed.dropped, stats.dropped,
        "stats file must match proxy::run's own returned counters"
    );
    assert_eq!(parsed.forwarded, stats.forwarded);
    assert_eq!(parsed.seed, cfg.seed);
    assert_eq!(parsed.config.outage_period_s, cfg.outage_period_s);
    assert_eq!(parsed.config.outage_dur_s, cfg.outage_dur_s);

    let _ = std::fs::remove_file(&stats_path);
}

/// SRT sender -> impaired proxy (2% loss, 10ms jitter, fixed seed) ->
/// real SRT listener, full round trip: SRT's own retransmission over
/// UDP must recover the receiver's invariant checks (`recv report pass
/// == true`) despite the injected loss/jitter — that recovery is the
/// entire point of this cell (see `tst_interop::impair`'s module doc).
///
/// Named with `round_trip` deliberately — see this file's module doc's
/// "nextest group placement" section for why.
///
/// The `GracefulSrtClose` send-side close wait (300ms, reasoned against
/// unimpaired loopback — see `transport.rs`) gets its first real
/// pressure here: if this test is ever observed to flake with a
/// tail-truncation shape (missing trailing KLV/video records, or a
/// `send`-side error during close), that is real evidence of the
/// watched margin being too tight under impairment, not something to
/// paper over with a looser assertion.
#[test]
fn srt_round_trip_through_lossy_proxy_recovers_via_retransmission() {
    let profile = profiles::by_name("baseline").expect("baseline profile must exist");
    const SECONDS: f64 = 5.0;

    let listener_port = free_port();
    let recv_url = format!("srt://127.0.0.1:{listener_port}?mode=listener");
    let recv_handle = {
        let recv_url = recv_url.clone();
        thread::spawn(move || recv::run(&recv_url, profile, SECONDS, None))
    };

    let cfg = ImpairConfig {
        loss_pct: 2.0,
        jitter_ms_max: 10,
        seed: 42,
        ..ImpairConfig::default()
    };
    let forward_addr: SocketAddr = format!("127.0.0.1:{listener_port}").parse().unwrap();
    let (proxy_addr, proxy_handle) =
        spawn_proxy("127.0.0.1:0".parse().unwrap(), forward_addr, cfg, None, 15);

    let send_url = format!("srt://127.0.0.1:{}", proxy_addr.port());
    let send_metrics = send_with_retry(profile, &send_url, SECONDS, Duration::from_secs(10));

    let recv_report =
        join_with_timeout(recv_handle, Duration::from_secs(20)).expect("recv::run must succeed");

    assert!(
        recv_report.pass,
        "recv failures over the lossy/jittered proxy: {:?}",
        recv_report.failures
    );
    assert_eq!(
        send_metrics.klv_set_sha256, recv_report.metrics.klv_set_sha256,
        "SRT must recover the full KLV record set despite injected loss"
    );

    // Don't wait out the proxy's own run_seconds budget on the happy
    // path (mirrors tests/serve.rs's convention for its lingering
    // producer threads) — only surface its result if it already
    // finished (a fast failure). nextest reaps this test process's
    // threads at process exit either way.
    if proxy_handle.is_finished() {
        proxy_handle
            .join()
            .expect("proxy thread panicked")
            .expect("proxy::run must succeed");
    }
}
