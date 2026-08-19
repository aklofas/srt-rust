//! Background-reconnect machinery for `ManagedTransport`:
//! the interruptible-wait primitive, the shared worker flags/counters,
//! and the worker's reconnect + drain state machine.
//!
//! Locking invariants (shared with `send_managed` in mod.rs):
//! 1. Lock order where both are held: `inner` -> `gap`. Never the reverse.
//! 2. `bg_active` transitions and the send-path enqueue decision happen
//!    under the `gap` lock, so worker exit and pump enqueue linearize.
//! 3. `spawn_worker` is never called while holding the `gap` lock.
//! 4. The worker holds `gap` across one inner send during drain — this
//!    pins the front message so a concurrent `DropOldest` eviction can't
//!    pop the message in flight (clone-then-pop would desync the queue).

#[allow(unused_imports)] // removed in a later task of this arc
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[allow(unused_imports)] // removed in a later task of this arc
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[allow(unused_imports)] // removed in a later task of this arc
use tracing::{info, warn};
#[allow(unused_imports)] // removed in a later task of this arc
use tst_core::transport::{Transport, TransportError};

#[allow(unused_imports)] // removed in a later task of this arc
use super::{GapBuffer, ReconnectPolicy};

/// Interruptible sleep. `wait_timeout(dur)` parks up to `dur`, returning
/// early (`true`) if `signal()` fired. A poisoned mutex reads as signaled
/// — conservative shutdown. Used by BOTH reconnect modes so that
/// `close()` / `cancel()` / `Drop` interrupt a backoff wait immediately
/// instead of waiting out a full `thread::sleep`.
pub(crate) struct Shutdown {
    flagged: Mutex<bool>,
    cv: Condvar,
}

impl Shutdown {
    pub(crate) fn new() -> Self {
        Self {
            flagged: Mutex::new(false),
            cv: Condvar::new(),
        }
    }

    pub(crate) fn signal(&self) {
        if let Ok(mut f) = self.flagged.lock() {
            *f = true;
        }
        // On poison: waiters treat poison as signaled, so notify is enough.
        self.cv.notify_all();
    }

    /// Returns true if shutdown was signaled (or the lock is poisoned);
    /// false if the full duration elapsed without a signal.
    pub(crate) fn wait_timeout(&self, dur: Duration) -> bool {
        // checked_add: Duration::MAX must not panic (same class as the
        // v0.5.0 checked-timeout-arithmetic fix). None => wait unbounded.
        let deadline = Instant::now().checked_add(dur);
        let Ok(mut flagged) = self.flagged.lock() else {
            return true;
        };
        while !*flagged {
            match deadline {
                Some(d) => {
                    let Some(remaining) = d.checked_duration_since(Instant::now()) else {
                        return false;
                    };
                    match self.cv.wait_timeout(flagged, remaining) {
                        Ok((guard, _)) => flagged = guard,
                        Err(_) => return true,
                    }
                }
                None => match self.cv.wait(flagged) {
                    Ok(guard) => flagged = guard,
                    Err(_) => return true,
                },
            }
        }
        true
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;

    #[test]
    fn wait_timeout_elapses_without_signal() {
        let s = Shutdown::new();
        let t0 = Instant::now();
        assert!(!s.wait_timeout(Duration::from_millis(50)));
        assert!(t0.elapsed() >= Duration::from_millis(50));
    }

    #[test]
    fn signal_interrupts_wait_promptly() {
        let s = Arc::new(Shutdown::new());
        let s2 = Arc::clone(&s);
        let h = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            s2.signal();
        });
        let t0 = Instant::now();
        assert!(
            s.wait_timeout(Duration::from_secs(30)),
            "must report signaled"
        );
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "must not wait out the 30s"
        );
        h.join().unwrap();
    }

    #[test]
    fn signal_before_wait_returns_immediately() {
        let s = Shutdown::new();
        s.signal();
        assert!(s.wait_timeout(Duration::from_secs(30)));
    }
}
