//! Shared scaffolding for tst-srt integration tests.
//! Loopback only; no external network.

#![allow(dead_code)]

use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Small wall-clock pause to give a listener thread time to enter accept().
pub fn settle() {
    std::thread::sleep(Duration::from_millis(100));
}

/// Probe whether 127.0.0.1 is bindable. Tests gate on this so
/// sandbox/restricted CI environments don't fail dozens of tests
/// they can't possibly pass. Set env `SKIP_LOOPBACK=1` to force-skip.
///
/// Uses TCP-bind: on Linux loopback is governed by the same per-interface
/// policy for TCP and UDP, so TCP-bindability is a faithful proxy for
/// "loopback works." SRT itself is UDP; the probe is layer-agnostic.
///
/// Returns `Ok(())` if loopback is usable; `Err(reason)` otherwise.
pub fn loopback_probe() -> Result<(), &'static str> {
    if std::env::var_os("SKIP_LOOPBACK").is_some() {
        return Err("SKIP_LOOPBACK env set");
    }
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    match TcpListener::bind(addr) {
        Ok(_) => Ok(()),
        Err(_) => Err("127.0.0.1 not bindable"),
    }
}

/// Macro: emit "SKIP" line and return early if loopback is unusable.
/// Use as the first line of every loopback test body. Requires
/// `mod common;` at the top of the test file.
#[macro_export]
macro_rules! require_loopback {
    () => {
        if let Err(why) = $crate::common::loopback_probe() {
            eprintln!("SKIP: loopback unavailable ({})", why);
            return;
        }
    };
}

/// Poll-with-deadline replacement for `thread::sleep(50ms)` listener-settle.
/// Matches the `accept_done` atomic-signal precedent from
/// `cancellation_loopback.rs`: the listener thread stores `true` into a
/// shared `AtomicBool` after `Listener::bind` returns (and before the
/// blocking `accept()` call); the main thread polls until set, then
/// connects.
///
/// Panics if the signal isn't set within 2 seconds — surfaces real
/// listener-thread failures loudly instead of silent flakes.
///
/// Usage shape at each caller:
/// ```ignore
/// let ready = Arc::new(AtomicBool::new(false));
/// let r = ready.clone();
/// thread::spawn(move || {
///     let listener = Listener::bind(addr).unwrap();
///     r.store(true, Ordering::SeqCst);
///     let socket = listener.accept().unwrap();
///     // ...
/// });
/// crate::common::wait_for_ready(&ready);
/// // ... main thread connect ...
/// ```
pub fn wait_for_ready(ready: &AtomicBool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !ready.load(Ordering::SeqCst) {
        if Instant::now() > deadline {
            panic!(
                "wait_for_ready: signal not set within 2s — listener \
                 thread may have panicked before signaling, or never \
                 called ready.store(true, ...)"
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

// ------------------------------------------------------------------------
// SrtLoopback helper: bind + spawn accept + ready-signal in one type.
// See plan: docs/plans/2026-05-15-srt-loopback-test-helper.md
// ------------------------------------------------------------------------

/// Loopback test fixture. Encapsulates the 15-line "bind listener, spawn
/// accept thread, signal ready via AtomicBool, hand socket to closure"
/// boilerplate so each test body shrinks to ~3 lines of setup.
///
/// Usage:
///
/// ```ignore
/// mod common;
/// use std::time::Duration;
/// use tst_srt::SocketBuilder;
///
/// #[test]
/// fn round_trip() {
///     require_loopback!();
///     let lb = crate::common::Loopback::bind();
///     let port = lb.port;
///
///     let accept = lb.spawn_accept(|mut sock| {
///         let mut buf = [0u8; 1500];
///         let n = sock.recv(&mut buf).expect("recv");
///         buf[..n].to_vec()
///     });
///     accept.wait_ready();
///
///     let mut socket = SocketBuilder::new()
///         .recv_timeout(Duration::from_secs(5))
///         .connect(format!("127.0.0.1:{port}"))
///         .expect("connect");
///     socket.send(b"hello").expect("send");
///
///     let received = accept.join();
///     assert_eq!(received, b"hello");
/// }
/// ```
///
/// Design notes:
/// - `Loopback` is consumed by `spawn_accept` because the underlying
///   `Listener` is moved into the accept thread. Cache `port` off the
///   listener BEFORE spawn for use in `SocketBuilder::connect`.
/// - The closure `f: FnOnce(Socket) -> R` receives the ACCEPTED socket
///   (the per-connection socket, not the listener). `R` is whatever the
///   test wants returned from the accept thread (typically a `Vec<u8>`
///   of received bytes, a count, or `()` for accept-then-drop).
/// - `wait_ready()` blocks until the listener thread has signaled (just
///   before entering `accept()`); the panic-with-deadline shape matches
///   `wait_for_ready` so listener-thread crashes surface loudly.
/// - `AcceptHandle::join` panics if the accept thread panicked — same
///   semantics as `JoinHandle::join().expect(...)`.
pub struct Loopback {
    pub listener: tst_srt::Listener,
    pub port: u16,
}

impl Loopback {
    /// Bind a listener to `127.0.0.1:0` with 5-second recv/send timeouts.
    /// Panics if bind fails — the test should have called `require_loopback!()`
    /// first.
    pub fn bind() -> Self {
        let listener = tst_srt::ListenerBuilder::new()
            .recv_timeout(Duration::from_secs(5))
            .send_timeout(Duration::from_secs(5))
            .bind("127.0.0.1:0")
            .expect("bind 127.0.0.1:0");
        let port = listener.local_addr().expect("local_addr").port();
        Self { listener, port }
    }

    /// Bind with a caller-supplied `ListenerBuilder` — for tests that need
    /// non-default options (encryption, custom timeouts, stream-id ACL).
    /// The builder MUST not already have called `.bind(...)`; this method
    /// supplies the bind address.
    pub fn bind_with(builder: tst_srt::ListenerBuilder) -> Self {
        let listener = builder.bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
        let port = listener.local_addr().expect("local_addr").port();
        Self { listener, port }
    }

    /// Spawn a thread that signals ready, accepts one connection, hands
    /// the resulting `Socket` to `f`, and returns `f`'s value via the
    /// `AcceptHandle`.
    pub fn spawn_accept<F, R>(self, f: F) -> AcceptHandle<R>
    where
        F: FnOnce(tst_srt::Socket) -> R + Send + 'static,
        R: Send + 'static,
    {
        let ready = Arc::new(AtomicBool::new(false));
        let r = ready.clone();
        let mut listener = self.listener;
        let handle = std::thread::spawn(move || {
            r.store(true, Ordering::SeqCst);
            let (sock, _peer) = listener.accept().expect("accept");
            f(sock)
        });
        AcceptHandle { handle, ready }
    }
}

/// Handle to the accept thread spawned by [`Loopback::spawn_accept`].
pub struct AcceptHandle<R> {
    handle: std::thread::JoinHandle<R>,
    ready: Arc<AtomicBool>,
}

impl<R: Send + 'static> AcceptHandle<R> {
    /// Block until the listener thread has signaled ready (just before
    /// `accept()`). Same semantics as [`wait_for_ready`] — panics after
    /// 2 seconds if the signal hasn't appeared.
    pub fn wait_ready(&self) {
        wait_for_ready(&self.ready);
    }

    /// Consume the handle and return the closure's value. Panics if the
    /// listener thread panicked.
    pub fn join(self) -> R {
        self.handle.join().expect("listener thread panicked")
    }
}
