//! `SrtCancelHandle` — thread-safe one-shot socket-close primitive.
//!
//! **Stability: Stable** — see the
//! [API stability reference](https://github.com/aklofas/ts-transformer/blob/main/docs/reference/api-stability.md).
//!
//! Wraps a libsrt `SRTSOCKET` (or any other integer handle) plus a
//! caller-supplied closer closure. Calling `cancel()` from any thread
//! atomically swaps the handle to a sentinel and invokes the closer
//! exactly once. Subsequent `cancel()` calls are no-ops.
//!
//! Used by `srt::Socket` and `srt::Listener` so a thread parked in
//! `srt_sendmsg` / `srt_recvmsg` / `srt_accept` can be woken from
//! another thread by closing the underlying SRT handle. Per libsrt's
//! semantics, closing a socket that another thread is parked on causes
//! the parked syscall to return with an error (`SRT_ECONNLOST` or
//! similar) — which surfaces through our error mapping as `Broken` /
//! `ConnectionBroken`.

use alloc::boxed::Box;
use alloc::sync::Arc;
use portable_atomic::{AtomicI64, Ordering};

/// Sentinel stored in the atomic once cancel has run. Picked as `i64::MIN`
/// because libsrt's `SRTSOCKET` (= `c_int`) cannot legally take this value
/// (and even libsrt's own `SRT_INVALID_SOCK = -1` won't collide).
const CANCELLED: i64 = i64::MIN;

/// Type-erased closer the handle invokes on its first `cancel()` call.
type Closer = Box<dyn Fn(i64) + Send + Sync>;

/// Thread-safe one-shot socket-close primitive.
///
/// Construct via `Socket::cancel_handle()` / `Listener::cancel_handle()`
/// (or the test-only `SrtCancelHandle::new`). Clone freely — every clone
/// shares the same atomic state, so calling `cancel()` on any clone
/// fires the closer exactly once.
#[derive(Clone)]
pub struct SrtCancelHandle {
    state: Arc<State>,
}

struct State {
    handle: AtomicI64,
    closer: Closer,
}

impl SrtCancelHandle {
    /// Build a handle. The closer is invoked at most once, with the
    /// handle value passed in here. Public so callers outside the `srt`
    /// module (e.g. test mocks) can construct one; production code uses
    /// `Socket::cancel_handle()` / `Listener::cancel_handle()`.
    pub fn new<F>(handle: i64, closer: F) -> Self
    where
        F: Fn(i64) + Send + Sync + 'static,
    {
        Self {
            state: Arc::new(State {
                handle: AtomicI64::new(handle),
                closer: Box::new(closer),
            }),
        }
    }

    /// Trigger the closer if it hasn't already run.
    ///
    /// Idempotent: extra calls (including from other threads) are no-ops.
    /// The closer always runs to completion on the thread that wins the
    /// atomic swap.
    pub fn cancel(&self) {
        let prev = self.state.handle.swap(CANCELLED, Ordering::AcqRel);
        if prev != CANCELLED {
            (self.state.closer)(prev);
        }
    }

    /// Returns `true` once `cancel()` has been called on this handle (or
    /// any clone of it). Advisory — the underlying socket close may not
    /// have completed yet on another thread.
    pub fn is_cancelled(&self) -> bool {
        self.state.handle.load(Ordering::Acquire) == CANCELLED
    }
}

/// A `SrtCancelHandle` is itself a [`TransportCancel`](crate::transport::TransportCancel):
/// this lets a `Listener`'s handle (the cross-thread wake for a parked
/// `accept()`) be installed wherever a `dyn TransportCancel` is expected —
/// notably a managed receiver's reconnect-factory cancel slot.
impl crate::transport::TransportCancel for SrtCancelHandle {
    fn cancel(&self) {
        SrtCancelHandle::cancel(self);
    }
}

impl core::fmt::Debug for SrtCancelHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SrtCancelHandle")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construct a SrtCancelHandle around an integer "handle" using a stub
    /// closer that records its calls. Verifies idempotence: the closer
    /// runs at most once across any number of cancel() calls (including
    /// concurrent ones).
    #[test]
    fn cancel_runs_closer_once_across_many_calls() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let calls = std::sync::Arc::new(AtomicU32::new(0));
        let calls_cl = calls.clone();
        let h = SrtCancelHandle::new(42, move |handle| {
            assert_eq!(handle, 42);
            calls_cl.fetch_add(1, Ordering::SeqCst);
        });

        h.cancel();
        h.cancel();
        h.cancel();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancel_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SrtCancelHandle>();
    }

    #[test]
    fn cancel_concurrent_runs_closer_once() {
        use std::sync::Barrier;
        use std::sync::atomic::{AtomicU32, Ordering};
        let calls = std::sync::Arc::new(AtomicU32::new(0));
        let calls_cl = calls.clone();
        let h = std::sync::Arc::new(SrtCancelHandle::new(7, move |handle| {
            // Verify the closer receives the original handle value, not the
            // CANCELLED sentinel. Catches a future bug where someone might
            // "fix" cancel() to pass CANCELLED instead of prev.
            assert_eq!(handle, 7);
            calls_cl.fetch_add(1, Ordering::SeqCst);
        }));
        let barrier = std::sync::Arc::new(Barrier::new(16));

        let mut threads = Vec::new();
        for _ in 0..16 {
            let h2 = h.clone();
            let b2 = barrier.clone();
            threads.push(std::thread::spawn(move || {
                b2.wait();
                h2.cancel();
            }));
        }
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn is_cancelled_flips_after_cancel() {
        let h = SrtCancelHandle::new(1, |_| {});
        assert!(!h.is_cancelled());
        h.cancel();
        assert!(h.is_cancelled());
    }
}
