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

use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use tracing::{info, warn};
use tst_core::transport::{Transport, TransportError};

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

/// State shared between `ManagedTransport`, its `ManagedStatsHandle`
/// observers, and any spawned background worker (later task in this arc).
#[derive(Debug, Default)]
pub(crate) struct ManagedShared {
    /// True while a background worker owns reconnect+drain.
    /// Transitions happen under the gap lock (invariant 2).
    pub(crate) bg_active: AtomicBool,
    /// Set by a worker that exhausted max_attempts; consumed (swap false)
    /// by the next send_bytes, which reports Broken exactly once.
    pub(crate) gave_up: AtomicBool,
    /// factory() invocations (either mode).
    pub(crate) reconnect_attempts: AtomicU64,
    /// Successful factory() installs (either mode).
    pub(crate) reconnect_successes: AtomicU64,
}

/// Backpressure retry cadence while draining on the worker — there is no
/// caller to propagate to, so the worker absorbs it. Interruptible.
const DRAIN_BACKPRESSURE_RETRY: Duration = Duration::from_millis(20);

/// Everything the worker thread owns. All `Arc`s — the worker never
/// borrows from `ManagedTransport`, so `Drop` can detach it safely.
pub(crate) struct WorkerCtx<T: Transport> {
    pub(crate) inner: Arc<Mutex<Option<T>>>,
    pub(crate) factory: Arc<dyn Fn() -> Result<T, TransportError> + Send + Sync>,
    pub(crate) gap: Arc<Mutex<GapBuffer>>,
    pub(crate) closed: Arc<AtomicBool>,
    pub(crate) shutdown: Arc<Shutdown>,
    pub(crate) shared: Arc<ManagedShared>,
    pub(crate) policy: ReconnectPolicy,
}

enum DrainStep {
    Sent,
    Empty,
    Backpressure,
    Broken,
}

/// Clear `bg_active` under the gap lock (invariant 2) on any exit path
/// that didn't already clear it via the Empty protocol. Poisoned gap:
/// clear anyway — a panic elsewhere is already unwinding the system.
fn exit_clear_active<T: Transport>(ctx: &WorkerCtx<T>) {
    let guard = ctx.gap.lock();
    ctx.shared.bg_active.store(false, Ordering::Release);
    drop(guard);
}

/// One outage's worth of reconnect + drain. Spawned on break, exits when
/// the gap fully drains (Empty protocol), the budget exhausts (give-up),
/// or shutdown is signaled.
pub(crate) fn worker_run<T: Transport>(ctx: WorkerCtx<T>) {
    // The budget covers ONE continuous outage: reset after each
    // successful install. `max_attempts` bounds attempts per outage, not
    // per transport lifetime — matching Blocking, where every
    // `send_bytes` call ran a fresh `reconnect_and_drain` budget.
    let mut attempt: u32 = 0;
    'reconnect: loop {
        if ctx.closed.load(Ordering::Acquire) {
            return exit_clear_active(&ctx);
        }
        attempt += 1;
        let Some(wait) = ctx.policy.next_delay(attempt) else {
            let max = ctx.policy.max_attempts.unwrap_or(0);
            warn!(
                target: "tst_pipeline::reconnect",
                attempts_made = attempt - 1,
                max_attempts = max,
                "background reconnect gave up — next send_bytes reports Broken",
            );
            // gave_up before clearing active: keeps the give-up cycle's
            // report deterministic for the send path's swap-consume.
            ctx.shared.gave_up.store(true, Ordering::Release);
            return exit_clear_active(&ctx);
        };
        info!(
            target: "tst_pipeline::reconnect",
            attempt,
            max_attempts = ctx.policy.max_attempts.unwrap_or(0),
            backoff_ms = wait.as_millis() as u64,
            "background reconnect attempt",
        );
        if ctx.shutdown.wait_timeout(wait) {
            return exit_clear_active(&ctx);
        }
        ctx.shared
            .reconnect_attempts
            .fetch_add(1, Ordering::Relaxed);
        let new_inner = match (ctx.factory)() {
            Ok(t) => t,
            Err(_) => continue 'reconnect,
        };
        {
            let Ok(mut guard) = ctx.inner.lock() else {
                // Inner lock poisoned — unrecoverable from a worker with
                // no caller. Surface as give-up so the next send reports.
                ctx.shared.gave_up.store(true, Ordering::Release);
                return exit_clear_active(&ctx);
            };
            *guard = Some(new_inner);
        }
        ctx.shared
            .reconnect_successes
            .fetch_add(1, Ordering::Relaxed);
        attempt = 0; // fresh budget for any subsequent break

        // ---- drain phase ----
        loop {
            if ctx.closed.load(Ordering::Acquire) {
                return exit_clear_active(&ctx);
            }
            // Per-message lock scope, order inner -> gap (invariant 1).
            // The gap lock is held across this one send on purpose
            // (invariant 4): it pins the front message so a concurrent
            // DropOldest eviction can't pop the message in flight. The
            // pump blocks on the gap lock for at most one inner send.
            let step = {
                let Ok(mut transport_guard) = ctx.inner.lock() else {
                    ctx.shared.gave_up.store(true, Ordering::Release);
                    return exit_clear_active(&ctx);
                };
                let mut gap = ctx
                    .gap
                    .lock()
                    .expect("BUG: gap lock poisoned — gap buffer is invariant-critical");
                if gap.front().is_none() {
                    // Empty protocol: clear active while STILL holding the
                    // gap lock — the send gate checks bg_active under this
                    // same lock, so it can never enqueue into a
                    // worker-less buffer (invariant 2).
                    ctx.shared.bg_active.store(false, Ordering::Release);
                    DrainStep::Empty
                } else if let Some(transport) = transport_guard.as_mut() {
                    let msg = gap.front().expect("checked non-empty above");
                    match transport.send_bytes(msg) {
                        Ok(()) => {
                            gap.pop_front();
                            DrainStep::Sent
                        }
                        Err(TransportError::Backpressure { .. }) => DrainStep::Backpressure,
                        Err(TransportError::TooLarge { len, max }) => {
                            // The rebuilt inner's ceiling shrank below a
                            // queued message. With no caller to bounce it
                            // to, keeping it would wedge the drain forever
                            // — drop it, count it, keep going.
                            if let Some(dropped) = gap.pop_front() {
                                gap.bytes_dropped += dropped.len() as u64;
                                gap.messages_dropped += 1;
                            }
                            warn!(
                                target: "tst_pipeline::reconnect",
                                len,
                                max,
                                "dropping queued message larger than the reconnected transport's max_payload",
                            );
                            DrainStep::Sent
                        }
                        Err(_) => {
                            // Broken / Closed / unknown-future — rebuild.
                            // Front message stays queued for the retry.
                            *transport_guard = None;
                            DrainStep::Broken
                        }
                    }
                } else {
                    // Inner vanished (only the worker clears it — belt and
                    // braces for future refactors): treat as broken.
                    DrainStep::Broken
                }
            };
            match step {
                DrainStep::Sent => continue,
                DrainStep::Empty => return, // active already cleared under the gap lock
                DrainStep::Backpressure => {
                    if ctx.shutdown.wait_timeout(DRAIN_BACKPRESSURE_RETRY) {
                        return exit_clear_active(&ctx);
                    }
                }
                DrainStep::Broken => continue 'reconnect,
            }
        }
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
