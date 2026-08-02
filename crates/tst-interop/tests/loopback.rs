//! Network-loopback round-trip tests for `send`/`recv`'s URL transport
//! dispatch: udp cell (byte-transparent loopback) + srt cell (reliable
//! caller/listener loopback).
//!
//! This binary's name (`loopback`) puts it in the `network` nextest
//! test-group (`.config/nextest.toml`), which caps each test at a hard
//! 20s kill (`slow-timeout = { period = "10s", terminate-after = 2 }`).
//! Every timing constant below is sized to keep the happy path (which
//! is what actually runs almost every time — connection setup on
//! loopback is sub-millisecond) at a few seconds, with generous but
//! still-bounded margin for a loaded CI box.
//!
//! Deterministic-test policy (no fixed sleeps as synchronization):
//! - Ports are ephemeral, discovered via a throwaway UDP bind (see
//!   `free_port`) — never hardcoded.
//! - The udp cell binds its recv transport (`transport::make_recv`) on
//!   THIS thread before spawning either peer — UDP has no handshake, so
//!   datagrams sent before the socket is bound are silently dropped;
//!   binding first, synchronously, makes the ordering race-free without
//!   a sleep.
//! - The srt cell can't do the same trick (`make_recv`'s listener path
//!   blocks on `accept()`, which must run on its own thread), so the
//!   sender instead retries its connect attempt on a short bounded
//!   budget (`send_with_retry`) — deterministic and fast in the common
//!   case, and safe from partial sends either way (a failed SRT connect
//!   attempt sends no data at all; see `send_with_retry`'s doc comment).
//! - All thread joins are timeout-bounded (`join_with_timeout`), never
//!   a bare `.join()`.

use std::net::UdpSocket;
use std::thread;
use std::time::{Duration, Instant};

use tst_interop::{profiles, recv, send, transport};

/// Shared by both cells — long enough to clear the 70%-of-nominal count
/// floors (`verify::NOMINAL_COUNT_SLACK`) with margin, short enough to
/// keep each test comfortably under the network test-group's 20s kill.
const SECONDS: f64 = 3.0;

/// Ask the OS for an unused port via a throwaway UDP bind. Used for both
/// cells — SRT runs over UDP too, so probing the UDP namespace matches
/// where an SRT socket will actually be allocated from. Small TOCTOU
/// race between this probe's drop and the real bind that follows; the
/// same accepted trade-off `examples/sending/encrypted_send_recv.rs`
/// documents for its own analogous port pick.
fn free_port() -> u16 {
    let probe = UdpSocket::bind("127.0.0.1:0").expect("bind probe port");
    probe.local_addr().expect("read probe local_addr").port()
}

/// Poll `handle.is_finished()` until it's done or `timeout` elapses.
/// The bounded-wait counterpart to a bare `.join()`, which this test
/// suite never uses (a hung receive loop must fail the test fast, not
/// hang the whole nextest run).
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

#[test]
fn udp_baseline_loopback_round_trips_and_matches() {
    let profile = profiles::by_name("baseline").expect("baseline profile must exist");
    let url = format!("udp://127.0.0.1:{}", free_port());

    // Bind the receiver here, on the test's own thread, before any peer
    // sends — see the module doc's synchronization note.
    let recv_transport = transport::make_recv(&url).expect("bind udp recv");
    let recv_handle = {
        let seconds = SECONDS;
        thread::spawn(move || recv::recv_over_transport(recv_transport, profile, seconds))
    };

    let send_metrics = send::run(profile, &url, SECONDS, None).expect("udp send must succeed");

    let recv_report = join_with_timeout(recv_handle, Duration::from_secs(10))
        .expect("recv_over_transport must succeed");

    assert!(
        recv_report.pass,
        "recv failures: {:?}",
        recv_report.failures
    );
    assert_eq!(
        send_metrics.klv_set_sha256, recv_report.metrics.klv_set_sha256,
        "sent and received KLV record sets must match"
    );
    assert_eq!(
        send_metrics.stream_sha256, recv_report.metrics.stream_sha256,
        "UDP loopback must be byte-transparent"
    );
    assert_eq!(
        send_metrics.bytes, recv_report.metrics.bytes,
        "sent and received byte counts must match"
    );
}

/// Retry `send::run` on a bounded budget. A failed SRT `connect()`
/// attempt (e.g. because the listener hasn't bound yet — see the
/// module doc's synchronization note) fails before any data is pushed,
/// so retrying the whole call never risks a double/partial send: either
/// the whole `seconds`-long session succeeds once connected, or nothing
/// was sent at all.
fn send_with_retry(
    profile: &profiles::Profile,
    url: &str,
    seconds: f64,
    budget: Duration,
) -> tst_interop::report_types::CellMetrics {
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

/// A listener-mode `srt://` recv against a port nobody ever connects to
/// must fail cleanly within a bound, not hang forever. `Listener::
/// accept()` (libsrt's plain accept call) has no timeout at all — see
/// `transport::srt_socket`'s doc comment on why it uses `accept_timeout`
/// instead — so this proves that fix actually bounds the wait.
///
/// Overrides the URL's `conntimeo`/`connect_timeout` overlay (which
/// `transport::srt_socket` reuses as its accept-timeout bound in
/// listener mode — see `SRT_ACCEPT_TIMEOUT`'s doc comment) down to 2s so
/// this test stays fast, rather than waiting out the 15s production
/// default and eating most of the network test-group's 20s per-test
/// kill.
#[test]
fn srt_listener_accept_times_out_when_nobody_connects() {
    let port = free_port();
    let url = format!("srt://127.0.0.1:{port}?mode=listener&conntimeo=2000");

    let started = Instant::now();
    let result = transport::make_recv(&url);
    let elapsed = started.elapsed();

    assert!(
        result.is_err(),
        "accept against a port nobody connects to must fail, not silently return"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "accept must return within its bounded timeout (~2s), not hang; took {elapsed:?}"
    );
}

#[test]
fn srt_baseline_loopback_round_trips_and_matches() {
    let profile = profiles::by_name("baseline").expect("baseline profile must exist");
    let port = free_port();
    let recv_url = format!("srt://127.0.0.1:{port}?mode=listener");
    let send_url = format!("srt://127.0.0.1:{port}"); // default mode=caller

    // The listener's bind()+listen() is fast, but recv::run's accept()
    // call blocks until a peer connects, so it must run on its own
    // thread — unlike the udp cell, there's no way to pre-bind and hand
    // off an already-accepted transport without also blocking this
    // thread. The sender's bounded retry (see `send_with_retry`) covers
    // the resulting race instead.
    let recv_handle = {
        let recv_url = recv_url.clone();
        thread::spawn(move || recv::run(&recv_url, profile, SECONDS, None))
    };

    let send_metrics = send_with_retry(profile, &send_url, SECONDS, Duration::from_secs(5));

    let recv_report =
        join_with_timeout(recv_handle, Duration::from_secs(15)).expect("recv::run must succeed");

    assert!(
        recv_report.pass,
        "recv failures: {:?}",
        recv_report.failures
    );
    assert_eq!(
        send_metrics.klv_set_sha256, recv_report.metrics.klv_set_sha256,
        "sent and received KLV record sets must match"
    );
    assert_eq!(
        send_metrics.stream_sha256, recv_report.metrics.stream_sha256,
        "SRT is a reliable in-order transport, so a loopback capture should be byte-transparent too"
    );
}
