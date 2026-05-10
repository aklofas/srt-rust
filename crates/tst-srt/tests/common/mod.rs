//! Shared scaffolding for tst-srt integration tests.
//! Loopback only; no external network.

#![allow(dead_code)]

use std::net::{SocketAddr, TcpListener};
use std::time::{Duration, Instant};

/// Small wall-clock pause to give a listener thread time to enter accept().
pub fn settle() {
    std::thread::sleep(Duration::from_millis(100));
}

/// Probe whether 127.0.0.1 is bindable. Cheap UDP/TCP open. Tests gate on
/// this so sandbox/restricted CI environments don't fail dozens of tests
/// they can't possibly pass. Set env `SKIP_LOOPBACK=1` to force-skip.
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
/// Use as the first line of every loopback test body.
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
/// Tries to connect to `addr` repeatedly with 5ms backoff until success or
/// 2 seconds elapse. Panics with a clear message on timeout — surfaces
/// CI flakes loudly instead of silent flakes.
///
/// Use this when you need the listener to be ready before connect, but
/// don't want to commit to a wall-clock sleep.
pub fn wait_for_listener(addr: &str) {
    let deadline = Instant::now() + Duration::from_secs(2);
    let parsed: SocketAddr = addr.parse().expect("addr must parse");
    while Instant::now() < deadline {
        if std::net::TcpStream::connect_timeout(&parsed, Duration::from_millis(50)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("listener at {} not ready within 2s", addr);
}
