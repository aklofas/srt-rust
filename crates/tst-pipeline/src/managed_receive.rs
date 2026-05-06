// crates/srt-core/src/pipeline/managed_receive.rs
//! `ManagedReceiveTransport<R>` — reconnect on receive break.
//!
//! Sibling to [`ManagedTransport`][crate::reconnect::ManagedTransport]:
//! same factory-closure + [`ReconnectPolicy`] cadence pattern, applied to the
//! receive direction. There is **no gap buffer** — receive-side bytes that
//! never arrived can't be replayed, so reconnect simply restarts the recv
//! loop on a fresh transport and lets the higher-level demux re-align.
//!
//! ## Composition shape
//!
//! `ManagedReceiveTransport` implements [`RecvTransport`], so it slots into
//! any of the receive shells (`RawReceiver`, `Receiver`, `DemuxReceiver`)
//! transparently:
//!
//! ```ignore
//! let factory = || SrtTransport::connect(addr, &cfg);
//! let inner = factory()?;
//! let managed = ManagedReceiveTransport::new(inner, Box::new(factory), ReconnectPolicy::default());
//! let rx = DemuxReceiver::new(managed);
//! ```
//!
//! ## Asymmetry vs. `ManagedTransport`
//!
//! The send-side decorator uses `Arc<dyn Fn() + Send + Sync>` for the
//! factory and `Arc<Mutex<Option<T>>>` for the inner transport — it must
//! support concurrent close-from-any-thread on top of `&mut self` sends.
//! The receive side has no such requirement: `recv_bytes(&mut self, …)` is
//! exclusive-mutable, so a `Box<dyn FnMut + Send>` factory and a plain
//! `Option<R>` inner suffice. This is intentional, not a copy-paste miss.
//!
//! ## Behavior on reconnect
//!
//! When `recv_bytes` returns `Closed` or `Broken`, the inner transport is
//! dropped, `policy.next_delay(attempt)` decides whether to wait-and-retry
//! or give up. On retry the factory rebuilds a fresh inner; on give-up the
//! decorator latches `closed = true` and all subsequent `recv_bytes` return
//! `Closed`.
//!
//! Backpressure (`TransportError::Backpressure`) is propagated unchanged —
//! it indicates a recv-timeout on a still-alive transport, no reconnect.
//! `TransportError::TooLarge` is not produced by `recv_bytes` (a recv-side
//! cap mismatch surfaces elsewhere) but is propagated unchanged for safety.
//!
//! ## Limitations worth knowing about
//!
//! - **Demuxer / sync state outlives reconnect.** This decorator only
//!   replaces the byte source. If the consumer wraps it in `Receiver`,
//!   the syncer's internal buffer carries over from the dead connection.
//!   In practice that costs at most one re-VERIFY pass (a stale packet of
//!   bytes is skipped during HUNT). For a clean restart, callers can use
//!   the `Receiver::reset_sync` helper from a higher-level shell. A
//!   future `ManagedReceiver` may wire this in automatically.
//! - **`max_payload` is assumed stable across reconnects.** The
//!   `RecvTransport` value reported here is the live inner's current
//!   value, but consumers that cache it at construction time (e.g.
//!   `Receiver`'s `recv_buf`) won't re-size if a reconnected peer
//!   advertises a different `SRTO_PAYLOADSIZE`. In practice every libsrt
//!   peer uses the 1316-byte default; a remote changing it across
//!   reconnects is exotic.
//! - **Demuxer flush is not invoked.** Terminal `TransportError::Closed`
//!   from this decorator means the reconnect budget is exhausted; the
//!   higher-level shell (`DemuxReceiver`) is responsible for calling
//!   `Demuxer::flush()` to drain any partial PES at end-of-stream.

use crate::reconnect::ReconnectPolicy;
use std::sync::{Arc, Mutex};
use tst_core::transport::RecvTransport;
use tst_core::transport::TransportError;

/// Receive-side reconnect decorator.
///
/// Wraps any [`RecvTransport`] with a factory closure for rebuilding it
/// on `Closed` / `Broken` failure, gated by a [`ReconnectPolicy`]. See
/// the module docs for the full semantics.
pub struct ManagedReceiveTransport<R: RecvTransport> {
    /// Currently-live inner transport. `None` between a tear-down and a
    /// successful factory rebuild.
    inner: Option<R>,
    /// Builds a fresh inner on demand. `FnMut` (rather than `Fn`) lets the
    /// caller carry mutable state — e.g. round-robin a list of fallback
    /// addrs across reconnects.
    factory: Box<dyn FnMut() -> Result<R, TransportError> + Send>,
    /// Backoff cadence + retry budget.
    policy: ReconnectPolicy,
    /// Local latched-close, set by `close(&mut self)`.
    closed: bool,
    /// Shared latched-close, set by the cancel handle from any thread.
    /// Read at every loop iteration in `recv_bytes`.
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    /// Most-recently-built inner's cancel handle, snapshotted on each
    /// successful build. Held in an Arc<Mutex<>> so the cancel handle
    /// (separate object) can read without owning &mut self.
    inner_cancel: Arc<Mutex<Option<Box<dyn tst_core::transport::TransportCancel>>>>,
}

impl<R: RecvTransport> ManagedReceiveTransport<R> {
    /// Build a new decorator around an already-connected `inner`.
    ///
    /// `factory` is called when `inner` later fails; on the very first
    /// `recv_bytes` it is not consulted because `inner` is provided
    /// up-front. Subsequent reconnects build via `factory` only.
    pub fn new(
        inner: R,
        factory: Box<dyn FnMut() -> Result<R, TransportError> + Send>,
        policy: ReconnectPolicy,
    ) -> Self {
        let inner_cancel: Arc<Mutex<Option<Box<dyn tst_core::transport::TransportCancel>>>> =
            Arc::new(Mutex::new(inner.cancel_handle()));
        Self {
            inner: Some(inner),
            factory,
            policy,
            closed: false,
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            inner_cancel,
        }
    }
}

impl<R: RecvTransport> RecvTransport for ManagedReceiveTransport<R> {
    fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        if self.closed || self.cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(TransportError::Closed);
        }
        let mut attempt: u32 = 0;
        loop {
            if self.cancelled.load(std::sync::atomic::Ordering::Acquire) {
                self.closed = true;
                return Err(TransportError::Closed);
            }
            // Get-or-rebuild inner. The factory may itself fail (e.g. DNS
            // didn't resolve, peer is still down) — treat factory failure
            // exactly like a recv break and back off via the policy.
            if self.inner.is_none() {
                attempt = attempt.saturating_add(1);
                let Some(delay) = self.policy.next_delay(attempt) else {
                    self.closed = true;
                    return Err(TransportError::Closed);
                };
                std::thread::sleep(delay);
                match (self.factory)() {
                    Ok(t) => {
                        *self.inner_cancel.lock().unwrap() = t.cancel_handle();
                        self.inner = Some(t);
                    }
                    Err(_) => continue,
                }
            }

            // Safety: just constructed above if it was None. unwrap is sound.
            let t = self.inner.as_mut().unwrap();
            match t.recv_bytes(buf) {
                Ok(n) => return Ok(n),
                Err(TransportError::Closed) | Err(TransportError::Broken(_)) => {
                    // Transport is dead. Drop it; next loop iteration
                    // reconnects via the factory under the configured backoff.
                    self.inner = None;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn max_payload(&self) -> usize {
        // 1316 is libsrt's universal SRT_DEFAULT_PAYLOADSIZE; used as a
        // safe fallback when the inner is mid-reconnect (None). Receive
        // shells that cache this on construction won't observe the None
        // window since they only call max_payload at construction time.
        self.inner.as_ref().map(|i| i.max_payload()).unwrap_or(1316)
    }

    fn is_alive(&self) -> bool {
        !self.closed
    }

    fn close(&mut self) {
        // Latch closed first so any concurrent reconnect attempt (none
        // possible today since recv_bytes is &mut self, but the latch is
        // cheap and forward-compatible) sees the new state.
        self.closed = true;
        if let Some(t) = self.inner.as_mut() {
            t.close();
        }
    }

    fn cancel_handle(&self) -> Option<Box<dyn tst_core::transport::TransportCancel>> {
        Some(Box::new(ManagedRecvCancel {
            cancelled: self.cancelled.clone(),
            inner_cancel: self.inner_cancel.clone(),
        }))
    }
}

struct ManagedRecvCancel {
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    inner_cancel: Arc<Mutex<Option<Box<dyn tst_core::transport::TransportCancel>>>>,
}

impl tst_core::transport::TransportCancel for ManagedRecvCancel {
    fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        let inner = self.inner_cancel.lock().ok().and_then(|mut g| g.take());
        if let Some(c) = inner {
            c.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconnect::{BackoffStrategy, ReconnectPolicy};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Mock `RecvTransport` that returns `Ok(n)` for the first
    /// `ok_until_calls` `recv_bytes` calls (each writing one byte), then
    /// switches to `Err(Broken)` to drive the reconnect path.
    struct FlakyRecv {
        calls: u32,
        ok_until: u32,
    }

    impl RecvTransport for FlakyRecv {
        fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
            self.calls += 1;
            if self.calls <= self.ok_until {
                buf[0] = self.calls as u8;
                Ok(1)
            } else {
                Err(TransportError::Broken("flaky test transport".into()))
            }
        }

        fn max_payload(&self) -> usize {
            1316
        }

        fn is_alive(&self) -> bool {
            self.calls < self.ok_until
        }
    }

    /// Build a policy with no real wait time so tests don't sleep.
    fn fast_policy(max_attempts: Option<u32>) -> ReconnectPolicy {
        ReconnectPolicy {
            max_attempts,
            backoff: BackoffStrategy::Constant(Duration::from_millis(0)),
            ..Default::default()
        }
    }

    /// On `Broken` from the inner, the decorator rebuilds via the factory
    /// and the next `recv_bytes` succeeds against the fresh transport.
    #[test]
    fn reconnects_on_broken() {
        let factory_calls = Arc::new(Mutex::new(0u32));
        let factory_calls_cl = factory_calls.clone();
        let factory = Box::new(move || {
            *factory_calls_cl.lock().unwrap() += 1;
            // Each rebuilt inner serves 1 byte then breaks.
            Ok(FlakyRecv {
                calls: 0,
                ok_until: 1,
            })
        });

        let initial = FlakyRecv {
            calls: 0,
            ok_until: 1,
        };
        let mut managed = ManagedReceiveTransport::new(initial, factory, fast_policy(Some(5)));

        let mut buf = [0u8; 8];
        // First call: from initial inner, returns Ok(1).
        assert_eq!(managed.recv_bytes(&mut buf).unwrap(), 1);
        // Initial inner is now exhausted; second call triggers reconnect
        // via factory and the new inner returns Ok(1).
        assert_eq!(managed.recv_bytes(&mut buf).unwrap(), 1);
        assert!(*factory_calls.lock().unwrap() >= 1);
    }

    /// When the reconnect budget is exhausted, `recv_bytes` returns
    /// `Closed` and the decorator latches as closed for all future calls.
    #[test]
    fn gives_up_after_max_attempts() {
        // Factory always fails — budget gets exhausted immediately.
        let factory = Box::new(|| -> Result<FlakyRecv, TransportError> {
            Err(TransportError::Broken("factory always fails".into()))
        });

        let initial = FlakyRecv {
            calls: 0,
            ok_until: 0,
        }; // breaks immediately
        let mut managed = ManagedReceiveTransport::new(initial, factory, fast_policy(Some(2)));

        let mut buf = [0u8; 8];
        let err = managed.recv_bytes(&mut buf).unwrap_err();
        assert_eq!(err, TransportError::Closed);
        assert!(!managed.is_alive());

        // Subsequent call short-circuits.
        let err2 = managed.recv_bytes(&mut buf).unwrap_err();
        assert_eq!(err2, TransportError::Closed);
    }

    /// `Backpressure` propagates unchanged — it indicates recv timeout on
    /// a still-alive transport, not a reason to reconnect.
    #[test]
    fn backpressure_propagates_without_reconnect() {
        struct BackpressureRecv;
        impl RecvTransport for BackpressureRecv {
            fn recv_bytes(&mut self, _buf: &mut [u8]) -> Result<usize, TransportError> {
                Err(TransportError::Backpressure("recv timeout".into()))
            }
            fn max_payload(&self) -> usize {
                1316
            }
            fn is_alive(&self) -> bool {
                true
            }
        }

        let factory_calls = Arc::new(Mutex::new(0u32));
        let factory_calls_cl = factory_calls.clone();
        let factory = Box::new(move || {
            *factory_calls_cl.lock().unwrap() += 1;
            Ok(BackpressureRecv)
        });

        let mut managed =
            ManagedReceiveTransport::new(BackpressureRecv, factory, fast_policy(Some(5)));

        let mut buf = [0u8; 8];
        let err = managed.recv_bytes(&mut buf).unwrap_err();
        assert!(matches!(err, TransportError::Backpressure(_)));
        // Factory must NOT have been called — backpressure is not a reason
        // to rebuild the transport.
        assert_eq!(*factory_calls.lock().unwrap(), 0);
    }

    /// `close()` latches the decorator closed and short-circuits future
    /// `recv_bytes` without consulting the factory.
    #[test]
    fn close_short_circuits() {
        let factory_calls = Arc::new(Mutex::new(0u32));
        let factory_calls_cl = factory_calls.clone();
        let factory = Box::new(move || {
            *factory_calls_cl.lock().unwrap() += 1;
            Ok(FlakyRecv {
                calls: 0,
                ok_until: 1,
            })
        });

        let initial = FlakyRecv {
            calls: 0,
            ok_until: 1,
        };
        let mut managed = ManagedReceiveTransport::new(initial, factory, fast_policy(Some(5)));

        managed.close();
        assert!(!managed.is_alive());

        let mut buf = [0u8; 8];
        assert_eq!(
            managed.recv_bytes(&mut buf).unwrap_err(),
            TransportError::Closed
        );
        assert_eq!(*factory_calls.lock().unwrap(), 0);
    }

    /// Stub RecvTransport whose cancel_handle latches a flag.
    struct CancellableRecv {
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    }
    struct CancellableRecvCancel {
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    }
    impl tst_core::transport::TransportCancel for CancellableRecvCancel {
        fn cancel(&self) {
            self.cancelled
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    impl RecvTransport for CancellableRecv {
        fn recv_bytes(&mut self, _: &mut [u8]) -> Result<usize, TransportError> {
            if self.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                Err(TransportError::Broken("cancelled".into()))
            } else {
                Ok(0)
            }
        }
        fn max_payload(&self) -> usize {
            1316
        }
        fn is_alive(&self) -> bool {
            !self.cancelled.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn cancel_handle(&self) -> Option<Box<dyn tst_core::transport::TransportCancel>> {
            Some(Box::new(CancellableRecvCancel {
                cancelled: self.cancelled.clone(),
            }))
        }
    }

    #[test]
    fn managed_recv_cancel_handle_latches_and_cancels_inner() {
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let inner = CancellableRecv {
            cancelled: cancelled.clone(),
        };
        let cancelled_cl = cancelled.clone();
        let factory = Box::new(move || -> Result<CancellableRecv, TransportError> {
            Ok(CancellableRecv {
                cancelled: cancelled_cl.clone(),
            })
        });
        let managed = ManagedReceiveTransport::new(inner, factory, fast_policy(Some(2)));

        let h = managed.cancel_handle().expect("cancellable inner -> Some");
        h.cancel();
        assert!(cancelled.load(std::sync::atomic::Ordering::SeqCst));
    }
}
