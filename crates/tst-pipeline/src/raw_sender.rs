//! `RawSender<T: Transport>` — one-shot byte-blind sender.
//!
//! Each `send` call sends exactly one outbound message of the given
//! length. No buffering, no framing, no accumulation. Caller is
//! responsible for sizing each message to the transport's
//! `max_payload()` (typically 1316 bytes for SRT live mode).
//!
//! Wrap with [`crate::ManagedTransport`] for reconnection.

use std::sync::Arc;
use tracing::{info_span, Span};
use tst_core::transport::{Transport, TransportError};

/// Construction-time knobs for [`RawSender`].
///
/// Currently empty — no behavior knobs are needed today. Reserved as a
/// distinct type so future additions are non-breaking.
#[derive(Debug, Clone, Default)]
pub struct RawSenderConfig {
    // Reserved for future use. Currently empty.
    _private: (),
}

/// Stats for [`RawSender`]. Aggregate-only — there are no streams at
/// this layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RawSenderStats {
    /// Bytes that succeeded through the transport.
    pub bytes_sent: u64,
    /// Count of successful `send()` calls.
    pub packets_sent: u64,
}

pub struct RawSender<T: Transport> {
    transport: T,
    _config: RawSenderConfig,
    stats: RawSenderStats,
    /// Lifetime [`tracing::Span`] opened in [`Self::new`] and entered
    /// from [`Drop`] to bracket open/close events. Private — must NOT
    /// be exposed publicly (see CI public-API ratchet).
    ///
    /// Wrapped in [`std::panic::AssertUnwindSafe`] because `Span`
    /// internally holds a `Mutex` which would otherwise flip this shell
    /// from `UnwindSafe`/`RefUnwindSafe` to `!UnwindSafe`/`!RefUnwindSafe`.
    /// `Span` is only entered in `new()` and `Drop`, never on hot paths,
    /// so asserting unwind safety is correct here.
    _span: std::panic::AssertUnwindSafe<Span>,
}

impl<T: Transport> RawSender<T> {
    pub fn new(transport: T, config: RawSenderConfig) -> Self {
        let span = info_span!(
            target: "tst_pipeline::raw_sender",
            "raw_sender",
            transport_kind = std::any::type_name::<T>(),
        );
        let _enter = span.enter();
        tracing::info!("RawSender opened");
        drop(_enter);
        Self {
            transport,
            _config: config,
            stats: RawSenderStats::default(),
            _span: std::panic::AssertUnwindSafe(span),
        }
    }

    /// Send one outbound message. Validates `bytes.len() ≤ transport.max_payload()`
    /// before delegating; the transport may add its own validation on top.
    ///
    /// Use this when the caller has its own muxer; for muxer integration
    /// use [`crate::Sender`] (pre-muxed TS bytes) or [`crate::MuxSender`]
    /// (encoded video / KLV / audio / subtitle in, TS out).
    ///
    /// # Errors
    /// - [`TransportError::TooLarge`] when `bytes.len()` exceeds
    ///   `transport.max_payload()`.
    /// - Bubbles up any other [`TransportError`] from the underlying
    ///   transport (e.g. `Closed`, `Broken`).
    ///
    /// # Example
    /// ```
    /// use tst_pipeline::{RawSender, RawSenderConfig};
    /// use tst_core::transport::{Transport, TransportError};
    ///
    /// // In-memory sink; real callers plug in `tst_srt::SrtTransport`.
    /// struct Sink(Vec<u8>);
    /// impl Transport for Sink {
    ///     fn send_bytes(&mut self, b: &[u8]) -> Result<(), TransportError> {
    ///         self.0.extend_from_slice(b);
    ///         Ok(())
    ///     }
    ///     fn max_payload(&self) -> usize { 1316 }
    ///     fn close(&mut self) {}
    ///     fn is_alive(&self) -> bool { true }
    /// }
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut sender = RawSender::new(Sink(Vec::new()), RawSenderConfig::default());
    /// sender.send(&[0u8; 1316])?;
    /// assert_eq!(sender.stats().packets_sent, 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        let max = self.transport.max_payload();
        if bytes.len() > max {
            return Err(TransportError::TooLarge {
                len: bytes.len(),
                max,
            });
        }
        self.transport.send_bytes(bytes)?;
        self.stats.bytes_sent += bytes.len() as u64;
        self.stats.packets_sent += 1;
        Ok(())
    }

    pub fn close(&mut self) {
        self.transport.close();
    }

    pub fn is_alive(&self) -> bool {
        self.transport.is_alive()
    }

    /// Borrow the inner transport (e.g., for stats accessors specific to
    /// the transport type).
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Snapshot of the underlying transport's cancel handle.
    pub fn cancel_handle(&self) -> Option<Arc<dyn tst_core::transport::TransportCancel + Send + Sync>> {
        self.transport.cancel_handle()
    }

    /// Snapshot stats counters.
    pub fn stats(&self) -> RawSenderStats {
        self.stats
    }

    /// Zero all stats counters. Stats-only — does not affect transport,
    /// pending data, or any other state.
    pub fn reset_stats(&mut self) {
        self.stats = RawSenderStats::default();
    }
}

impl<T: Transport> Drop for RawSender<T> {
    fn drop(&mut self) {
        let _enter = self._span.0.enter();
        tracing::info!("RawSender closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tst_core::transport::{Transport, TransportError};

    struct MemTransport {
        max: usize,
        alive: bool,
        accept: bool,
    }
    impl Transport for MemTransport {
        fn send_bytes(&mut self, _bytes: &[u8]) -> Result<(), TransportError> {
            if self.accept {
                Ok(())
            } else {
                Err(TransportError::Broken("test".into()))
            }
        }
        fn max_payload(&self) -> usize {
            self.max
        }
        fn close(&mut self) {
            self.alive = false;
        }
        fn is_alive(&self) -> bool {
            self.alive
        }
    }

    #[test]
    fn stats_starts_zero() {
        let s = RawSender::new(
            MemTransport {
                max: 1316,
                alive: true,
                accept: true,
            },
            RawSenderConfig::default(),
        );
        let st = s.stats();
        assert_eq!(st.bytes_sent, 0);
        assert_eq!(st.packets_sent, 0);
    }

    #[test]
    fn stats_increment_on_successful_send() {
        let mut s = RawSender::new(
            MemTransport {
                max: 1316,
                alive: true,
                accept: true,
            },
            RawSenderConfig::default(),
        );
        s.send(&[0u8; 100]).unwrap();
        s.send(&[0u8; 200]).unwrap();
        let st = s.stats();
        assert_eq!(st.bytes_sent, 300);
        assert_eq!(st.packets_sent, 2);
    }

    #[test]
    fn stats_unchanged_on_too_large() {
        let mut s = RawSender::new(
            MemTransport {
                max: 100,
                alive: true,
                accept: true,
            },
            RawSenderConfig::default(),
        );
        let _ = s.send(&[0u8; 200]); // exceeds max
        let st = s.stats();
        assert_eq!(st.bytes_sent, 0);
        assert_eq!(st.packets_sent, 0);
    }

    #[test]
    fn stats_unchanged_on_transport_error() {
        let mut s = RawSender::new(
            MemTransport {
                max: 1316,
                alive: true,
                accept: false,
            },
            RawSenderConfig::default(),
        );
        let _ = s.send(&[0u8; 100]);
        let st = s.stats();
        assert_eq!(st.bytes_sent, 0);
        assert_eq!(st.packets_sent, 0);
    }

    #[test]
    fn reset_zeros_counters() {
        let mut s = RawSender::new(
            MemTransport {
                max: 1316,
                alive: true,
                accept: true,
            },
            RawSenderConfig::default(),
        );
        s.send(&[0u8; 100]).unwrap();
        s.reset_stats();
        let st = s.stats();
        assert_eq!(st.bytes_sent, 0);
        assert_eq!(st.packets_sent, 0);
    }

    #[test]
    fn mem_transport_default_cancel_handle_is_none() {
        let t = MemTransport {
            max: 1316,
            alive: true,
            accept: true,
        };
        assert!(t.cancel_handle().is_none());
    }
}
