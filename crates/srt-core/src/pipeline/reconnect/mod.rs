// crates/srt-core/src/pipeline/reconnect/mod.rs
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
    /// `ManagedReceiveTransport` (receive side) so the backoff math lives in
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

use crate::pipeline::transport::{Transport, TransportError};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;

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
/// shells (`Sender`, `TsSender`, `RawSender`) compose with it
/// transparently:
///
/// ```ignore
/// let factory = || SrtTransport::connect(...);
/// let inner = factory()?;
/// let managed = ManagedTransport::new(inner, factory, ReconnectPolicy::default());
/// let sender = Sender::new(config, managed)?;
/// // sender now silently reconnects on transport breakage
/// ```
pub struct ManagedTransport<T: Transport> {
    inner: Arc<Mutex<Option<T>>>,
    factory: Arc<dyn Fn() -> Result<T, TransportError> + Send + Sync>,
    policy: ReconnectPolicy,
    gap: Arc<Mutex<GapBuffer>>,
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
        }
    }

    /// Try to send via the inner transport. On Broken/Closed, queue bytes
    /// and attempt reconnect.
    ///
    /// Pre-checks `bytes.len() > max_payload` against the inner transport
    /// before any state mutation, so oversized messages never enter the gap
    /// buffer (where they'd block drain forever).
    fn send_managed(&self, bytes: &[u8]) -> Result<(), TransportError> {
        // Pre-check size against inner before queuing — oversized messages
        // would otherwise sit in the gap buffer and fail every drain.
        let max = self
            .inner
            .lock()
            .unwrap()
            .as_ref()
            .map(|t| t.max_payload())
            .unwrap_or(1316);
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
            attempt += 1;
            let Some(wait) = self.policy.next_delay(attempt) else {
                let max = self.policy.max_attempts.unwrap_or(0);
                return Err(TransportError::Broken(format!(
                    "reconnect gave up after {max} attempts"
                )));
            };
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
            .unwrap_or(1316)
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
        if let Some(t) = self.inner.lock().unwrap().as_mut() {
            t.close();
        }
    }
}
