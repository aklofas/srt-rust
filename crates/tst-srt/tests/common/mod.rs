//! Shared scaffolding for tst-srt integration tests.
//! Loopback only; no external network.

#![allow(dead_code)]

use std::net::{SocketAddr, TcpListener};
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
/// common::wait_for_ready(&ready);
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
