// crates/srt-core/src/pipeline/raw_sender.rs
//! `RawSender<T: Transport>` — one-shot byte-blind sender.
//!
//! Each `send` call sends exactly one outbound message of the given
//! length. No buffering, no framing, no accumulation. Caller is
//! responsible for sizing each message to the transport's
//! `max_payload()` (typically 1316 bytes for SRT live mode).
//!
//! Wrap with [`crate::pipeline::ManagedTransport`] for reconnection.

use crate::pipeline::transport::{Transport, TransportError};

/// Construction-time knobs for [`RawSender`].
///
/// Currently empty — no behavior knobs are needed today. Reserved as a
/// distinct type so future additions are non-breaking.
#[derive(Debug, Clone, Default)]
pub struct RawSenderConfig {
    // Reserved for future use. Currently empty.
    _private: (),
}

pub struct RawSender<T: Transport> {
    transport: T,
    _config: RawSenderConfig,
}

impl<T: Transport> RawSender<T> {
    pub fn new(transport: T, config: RawSenderConfig) -> Self {
        Self {
            transport,
            _config: config,
        }
    }

    /// Send one outbound message. Validates `bytes.len() ≤ transport.max_payload()`
    /// before delegating; the transport may add its own validation on top.
    pub fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        let max = self.transport.max_payload();
        if bytes.len() > max {
            return Err(TransportError::TooLarge {
                len: bytes.len(),
                max,
            });
        }
        self.transport.send_bytes(bytes)
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
}
