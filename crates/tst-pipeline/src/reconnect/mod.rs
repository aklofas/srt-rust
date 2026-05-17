//! `ManagedTransport<T>` — Transport decorator with reconnect + gap buffer.
//!
//! Wraps any inner Transport (most commonly `SrtTransport`); on send
//! failure with `Broken` semantics, queues the bytes in a fixed-size
//! gap buffer and attempts to re-establish the inner transport with
//! configurable backoff. On reconnect success, drains the gap buffer
//! before resuming new sends.

mod gap_buffer;

pub use gap_buffer::{GapBuffer, OverflowPolicy};

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackoffStrategy {
    /// Fixed wait between attempts.
    Constant(Duration),
    /// Exponential: wait = base * 2^(attempt-1), capped at max.
    Exponential { base: Duration, max: Duration },
}

impl Default for BackoffStrategy {
    fn default() -> Self {
        BackoffStrategy::Exponential {
            base: Duration::from_millis(100),
            max: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReconnectPolicy {
    /// Maximum reconnect attempts before giving up. None = retry forever.
    /// Default: `Some(10)`.
    pub max_attempts: Option<u32>,

    /// Backoff strategy between attempts. Default: exponential 100ms..=10s.
    pub backoff: BackoffStrategy,

    /// Gap-buffer capacity in messages. Default 256.
    pub gap_buffer_capacity: usize,

    /// What to do when gap buffer is full and a new message arrives.
    /// Default: drop oldest message.
    pub overflow_policy: OverflowPolicy,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            max_attempts: Some(10),
            backoff: BackoffStrategy::default(),
            gap_buffer_capacity: 256,
            overflow_policy: OverflowPolicy::DropOldest,
        }
    }
}

impl ReconnectPolicy {
    /// Compute the wait before the next reconnect attempt, or `None` if the
    /// budget is exhausted.
    ///
    /// `attempt` is the 1-based index of the attempt about to be made (i.e.
    /// the very first reconnect after a transport break is `attempt = 1`).
    /// When `max_attempts == Some(n)`, returns `None` once `attempt > n`.
    /// When `max_attempts == None`, retries forever (always returns `Some`).
    ///
    /// Used by both `ManagedTransport` (send side) and
    /// `ManagedRecvTransport` (receive side) so the backoff math lives in
    /// one place.
    pub fn next_delay(&self, attempt: u32) -> Option<Duration> {
        if let Some(max) = self.max_attempts {
            if attempt > max {
                return None;
            }
        }
        let wait = match &self.backoff {
            BackoffStrategy::Constant(d) => *d,
            BackoffStrategy::Exponential { base, max } => {
                let exp = (*base).saturating_mul(1 << attempt.saturating_sub(1).min(20));
                if exp > *max { *max } else { exp }
            }
        };
        Some(wait)
    }
}

use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use tracing::{debug, info, warn};
use tst_core::mpegts::common::SRT_TS_BUNDLE_BYTES;
use tst_core::transport::{Transport, TransportCancel, TransportError};

/// Decorator that wraps an inner `Transport` with reconnect + gap-buffer
/// behavior.
///
/// On `send_bytes` returning `TransportError::Broken`, the bytes go into
/// the gap buffer (subject to the configured overflow policy) and the
/// inner transport is rebuilt via the user-supplied factory closure.
/// Reconnect attempts run synchronously on the caller's thread with the
/// configured backoff. After the inner transport reconnects, the gap
/// buffer is drained before resuming new sends.
///
/// `ManagedTransport` itself implements `Transport`, so all three sender
/// shells (`MuxSender`, `Sender`, `RawSender`) compose with it
/// transparently:
///
/// ```ignore
/// let factory = || SrtTransport::connect(...);
/// let inner = factory()?;
/// let managed = ManagedTransport::new(inner, factory, ReconnectPolicy::default());
/// let sender = MuxSender::new(managed, config)?;
/// // sender now silently reconnects on transport breakage
/// ```
///
/// # Panics
///
/// All [`Transport`] methods (`send_bytes`, `max_payload`, `is_alive`,
/// `close`) and [`Self::cancel_handle`] acquire internal [`Mutex`]es and
/// panic if a lock has been poisoned by a previous panic in another
/// thread inside the same `ManagedTransport`. This is the standard Rust
/// `Mutex` behavior; a poisoned lock signals that the inner-transport
/// or gap-buffer state may be inconsistent and the wrapper should be
/// discarded.
pub struct ManagedTransport<T: Transport> {
    inner: Arc<Mutex<Option<T>>>,
    factory: Arc<dyn Fn() -> Result<T, TransportError> + Send + Sync>,
    policy: ReconnectPolicy,
    gap: Arc<Mutex<GapBuffer>>,
    /// Latched true by `cancel_handle().cancel()` or `close()`. The
    /// reconnect loop checks this each iteration so a cancel mid-retry
    /// breaks out instead of waiting through the full backoff budget.
    closed: Arc<std::sync::atomic::AtomicBool>,
}

impl<T: Transport + 'static> ManagedTransport<T> {
    pub fn new<F>(inner: T, factory: F, policy: ReconnectPolicy) -> Self
    where
        F: Fn() -> Result<T, TransportError> + Send + Sync + 'static,
    {
        let gap = GapBuffer::new(policy.gap_buffer_capacity, policy.overflow_policy);
        Self {
            inner: Arc::new(Mutex::new(Some(inner))),
            factory: Arc::new(factory),
            policy,
            gap: Arc::new(Mutex::new(gap)),
            closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Try to send via the inner transport. On Broken/Closed, queue bytes
    /// and attempt reconnect.
    ///
    /// Pre-checks `bytes.len() > max_payload` against the inner transport
    /// before any state mutation, so oversized messages never enter the gap
    /// buffer (where they'd block drain forever).
    fn send_managed(&self, bytes: &[u8]) -> Result<(), TransportError> {
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(TransportError::Closed);
        }
        // Pre-check size against inner before queuing — oversized messages
        // would otherwise sit in the gap buffer and fail every drain.
        let max = self
            .inner
            .lock()
            .unwrap()
            .as_ref()
            .map(|t| t.max_payload())
            .unwrap_or(SRT_TS_BUNDLE_BYTES);
        if bytes.len() > max {
            return Err(TransportError::TooLarge {
                len: bytes.len(),
                max,
            });
        }

        // Drain any queued bytes first. If drain breaks the transport
        // mid-flight (Broken), the caller's `bytes` would be lost without
        // queuing. Capture that case and fall through to enqueue+reconnect.
        match self.drain_gap_if_alive() {
            Ok(()) => {}
            Err(TransportError::Broken(_)) | Err(TransportError::Closed) => {
                // Fall through to enqueue + reconnect — the new bytes get
                // queued alongside whatever's still in the gap buffer.
            }
            Err(e) => return Err(e),
        }

        // Try the new bytes if the transport is still alive after drain.
        if let Some(transport) = self.inner.lock().unwrap().as_mut() {
            match transport.send_bytes(bytes) {
                Ok(()) => return Ok(()),
                Err(TransportError::Backpressure(_)) => {
                    // Backpressure is recoverable without reconnect — propagate.
                    // Caller may retry the same bytes.
                    return Err(TransportError::Backpressure("inner backpressure".into()));
                }
                Err(TransportError::TooLarge { len, max }) => {
                    return Err(TransportError::TooLarge { len, max });
                }
                Err(TransportError::Broken(_)) | Err(TransportError::Closed) => {
                    // Fall through to reconnect path.
                }
                Err(_) => {
                    // Phase 1: Unknown future variant — treat as broken and reconnect.
                    // Fall through to reconnect path.
                }
            }
        }

        // Inner is broken/closed. Queue this message and attempt reconnect.
        {
            let mut gap = self.gap.lock().unwrap();
            let _ = gap.enqueue(bytes.to_vec()); // overflow policy applies
        }
        self.reconnect_and_drain()
    }

    /// Drain the gap buffer if the inner transport is alive.
    fn drain_gap_if_alive(&self) -> Result<(), TransportError> {
        let mut transport_guard = self.inner.lock().unwrap();
        let Some(transport) = transport_guard.as_mut() else {
            return Ok(()); // can't drain without a transport
        };
        let mut gap = self.gap.lock().unwrap();
        while let Some(msg) = gap.front() {
            match transport.send_bytes(msg) {
                Ok(()) => {
                    gap.pop_front();
                }
                Err(TransportError::Backpressure(_)) => {
                    return Err(TransportError::Backpressure("drain backpressure".into()));
                }
                Err(TransportError::Broken(_)) | Err(TransportError::Closed) => {
                    *transport_guard = None;
                    return Err(TransportError::Broken(
                        "transport broken during drain".into(),
                    ));
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn reconnect_and_drain(&self) -> Result<(), TransportError> {
        let mut attempt: u32 = 0;
        loop {
            if self.closed.load(std::sync::atomic::Ordering::Acquire) {
                return Err(TransportError::Closed);
            }
            attempt += 1;
            let Some(wait) = self.policy.next_delay(attempt) else {
                let max = self.policy.max_attempts.unwrap_or(0);
                // attempts_made = attempt - 1 because this iteration never
                // got past the budget check (no factory call happened on
                // this turn).
                warn!(
                    target: "tst_pipeline::reconnect",
                    attempts_made = attempt - 1,
                    max_attempts = max,
                    "reconnect gave up — propagating final error to caller",
                );
                return Err(TransportError::Broken(format!(
                    "reconnect gave up after {max} attempts"
                )));
            };
            info!(
                target: "tst_pipeline::reconnect",
                attempt,
                max_attempts = self.policy.max_attempts.unwrap_or(0),
                backoff_ms = wait.as_millis() as u64,
                "reconnect attempt",
            );
            if !wait.is_zero() {
                debug!(
                    target: "tst_pipeline::reconnect",
                    backoff_ms = wait.as_millis() as u64,
                    "backoff before next attempt",
                );
            }
            thread::sleep(wait);
            match (self.factory)() {
                Ok(new_inner) => {
                    *self.inner.lock().unwrap() = Some(new_inner);
                    // Drain gap buffer.
                    return self.drain_gap_if_alive();
                }
                Err(_) => {
                    continue; // try again
                }
            }
        }
    }
}

impl<T: Transport + 'static> Transport for ManagedTransport<T> {
    fn send_bytes(&mut self, msg: &[u8]) -> Result<(), TransportError> {
        self.send_managed(msg)
    }

    fn max_payload(&self) -> usize {
        self.inner
            .lock()
            .unwrap()
            .as_ref()
            .map(|t| t.max_payload())
            .unwrap_or(SRT_TS_BUNDLE_BYTES)
    }

    fn is_alive(&self) -> bool {
        self.inner
            .lock()
            .unwrap()
            .as_ref()
            .map(|t| t.is_alive())
            .unwrap_or(false)
    }

    fn close(&mut self) {
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(t) = self.inner.lock().unwrap().as_mut() {
            t.close();
        }
    }

    fn cancel_handle(&self) -> Option<Arc<dyn TransportCancel + Send + Sync>> {
        let inner = self.inner.clone();
        let closed = self.closed.clone();
        Some(Arc::new(ManagedCancel { inner, closed }))
    }

    fn socket_stats(&self) -> Option<tst_core::transport::SocketStats> {
        // Mirror max_payload() / is_alive() shape: when inner is None
        // (mid-reconnect or after close), there's no socket to query.
        // Returns None rather than fake-zero — the C ABI maps that to
        // TST_E_NOT_AVAILABLE so callers can distinguish "no socket"
        // from "socket exists with zero counters".
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(|t| t.socket_stats()))
    }
}

struct ManagedCancel<T: Transport + 'static> {
    inner: Arc<Mutex<Option<T>>>,
    closed: Arc<std::sync::atomic::AtomicBool>,
}

impl<T: Transport + 'static> TransportCancel for ManagedCancel<T> {
    fn cancel(&self) {
        // Latch closed first so the reconnect loop exits next iteration.
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        // Then cancel the current inner if any. We re-acquire the inner
        // mutex briefly to grab a cancel-handle from it, then release;
        // we do NOT hold the inner mutex while invoking cancel (which
        // could call srt_close — sub-millisecond, but still better off
        // the lock).
        let inner_cancel = {
            let guard = self.inner.lock().ok();
            guard.and_then(|g| g.as_ref().and_then(|t| t.cancel_handle()))
        };
        if let Some(c) = inner_cancel {
            c.cancel();
        }
    }
}

#[cfg(test)]
mod cancel_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use tst_core::transport::{Transport, TransportCancel, TransportError};

    /// Stub Transport whose cancel_handle records cancel() calls and
    /// makes is_alive() return false after cancel.
    struct CancellableMock {
        cancelled: Arc<std::sync::atomic::AtomicBool>,
        cancel_calls: Arc<AtomicU32>,
    }
    struct CancellableMockCancel {
        cancelled: Arc<std::sync::atomic::AtomicBool>,
        calls: Arc<AtomicU32>,
    }
    impl TransportCancel for CancellableMockCancel {
        fn cancel(&self) {
            self.cancelled.store(true, Ordering::SeqCst);
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl Transport for CancellableMock {
        fn send_bytes(&mut self, _: &[u8]) -> Result<(), TransportError> {
            if self.cancelled.load(Ordering::SeqCst) {
                Err(TransportError::Broken("cancelled".into()))
            } else {
                Ok(())
            }
        }
        fn max_payload(&self) -> usize {
            1316
        }
        fn is_alive(&self) -> bool {
            !self.cancelled.load(Ordering::SeqCst)
        }
        fn close(&mut self) {
            self.cancelled.store(true, Ordering::SeqCst);
        }
        fn cancel_handle(&self) -> Option<Arc<dyn TransportCancel + Send + Sync>> {
            Some(Arc::new(CancellableMockCancel {
                cancelled: self.cancelled.clone(),
                calls: self.cancel_calls.clone(),
            }))
        }
    }

    #[test]
    fn managed_cancel_handle_cancels_current_inner() {
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let calls = Arc::new(AtomicU32::new(0));
        let inner = CancellableMock {
            cancelled: cancelled.clone(),
            cancel_calls: calls.clone(),
        };
        let factory = move || -> Result<CancellableMock, TransportError> {
            Err(TransportError::Broken("test factory always fails".into()))
        };
        let managed = ManagedTransport::new(inner, factory, ReconnectPolicy::default());

        let handle = managed.cancel_handle().expect("cancellable inner -> Some");
        handle.cancel();
        assert!(cancelled.load(Ordering::SeqCst));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn managed_cancel_latches_closed_so_reconnect_loop_exits() {
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let calls = Arc::new(AtomicU32::new(0));
        let inner = CancellableMock {
            cancelled: cancelled.clone(),
            cancel_calls: calls.clone(),
        };
        let factory_calls = Arc::new(AtomicU32::new(0));
        let factory_calls_cl = factory_calls.clone();
        let cancelled_cl = cancelled.clone();
        let calls_cl2 = calls.clone();
        let factory = move || -> Result<CancellableMock, TransportError> {
            factory_calls_cl.fetch_add(1, Ordering::SeqCst);
            Ok(CancellableMock {
                cancelled: cancelled_cl.clone(),
                cancel_calls: calls_cl2.clone(),
            })
        };
        let policy = ReconnectPolicy {
            max_attempts: Some(100),
            backoff: BackoffStrategy::Constant(std::time::Duration::from_millis(0)),
            ..Default::default()
        };
        let mut managed = ManagedTransport::new(inner, factory, policy);

        // Trigger cancel before any send.
        let h = managed.cancel_handle().unwrap();
        h.cancel();

        // After cancel, send_bytes should return Broken without burning
        // through reconnect attempts (the closed flag short-circuits the
        // reconnect loop).
        let err = managed.send_bytes(b"x").unwrap_err();
        assert!(matches!(
            err,
            TransportError::Broken(_) | TransportError::Closed
        ));
        // The factory should NOT have been called repeatedly trying to
        // reconnect after cancel.
        assert!(factory_calls.load(Ordering::SeqCst) <= 1);
    }
}
