// crates/srt-core/src/pipeline/raw_receiver.rs
//! `RawReceiver<R>` — return one owned byte vec per recv, no TS framing.
//!
//! This is the simplest receive shell: one `recv_one` call blocks until a
//! single SRT message arrives, then returns the bytes verbatim. There is no
//! MPEG-TS sync recovery or stream demuxing — that's `TsReceiver`'s job.
//!
//! Use `RawReceiver` when:
//! - The sender uses `RawSender` (raw byte blobs, no TS wrapping).
//! - You want to handle framing yourself.
//! - You're writing a test that needs a bare receive loop.

use crate::pipeline::recv_transport::RecvTransport;
use crate::pipeline::transport::TransportError;

/// Receive shell that emits one raw byte vec per transport message.
///
/// `R` is any [`RecvTransport`] — typically `SrtTransport` for live
/// connections, or a test mock for unit tests.
pub struct RawReceiver<R: RecvTransport> {
    transport: R,
    /// Reusable scratch buffer sized to `transport.max_payload()` on
    /// construction. Avoids a per-call allocation for the recv itself;
    /// `recv_one` still allocates a `Vec` for the returned slice.
    buf: Vec<u8>,
}

impl<R: RecvTransport> RawReceiver<R> {
    /// Wrap a transport. Allocates an internal buffer sized to
    /// `transport.max_payload()`.
    pub fn new(transport: R) -> Self {
        let cap = transport.max_payload();
        Self {
            transport,
            buf: vec![0u8; cap],
        }
    }

    /// Block until one message arrives. Returns a copy of the received bytes.
    ///
    /// Returns `Err(TransportError::Closed)` when the connection has ended.
    /// Returns `Err(TransportError::Backpressure)` on a recv timeout — the
    /// transport is still alive; the caller may call `recv_one` again.
    pub fn recv_one(&mut self) -> Result<Vec<u8>, TransportError> {
        let n = self.transport.recv_bytes(&mut self.buf)?;
        Ok(self.buf[..n].to_vec())
    }

    /// Advisory liveness check. Delegates to the underlying transport.
    pub fn is_alive(&self) -> bool {
        self.transport.is_alive()
    }

    /// Close the underlying transport. Idempotent. After close, `recv_one`
    /// returns `TransportError::Closed`. Mirrors `RawSender::close`.
    pub fn close(&mut self) {
        self.transport.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::transport::TransportError;

    /// Minimal `RecvTransport` mock that plays back a fixed sequence of
    /// messages then signals closed.
    struct MockRecv {
        messages: Vec<Vec<u8>>,
        pos: usize,
        max: usize,
    }

    impl MockRecv {
        fn new(messages: Vec<Vec<u8>>) -> Self {
            let max = messages.iter().map(|m| m.len()).max().unwrap_or(1316);
            Self {
                messages,
                pos: 0,
                max,
            }
        }
    }

    impl RecvTransport for MockRecv {
        fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
            if self.pos >= self.messages.len() {
                return Err(TransportError::Closed);
            }
            let msg = &self.messages[self.pos];
            let n = msg.len();
            buf[..n].copy_from_slice(msg);
            self.pos += 1;
            Ok(n)
        }

        fn max_payload(&self) -> usize {
            self.max
        }

        fn is_alive(&self) -> bool {
            self.pos < self.messages.len()
        }
    }

    #[test]
    fn raw_receiver_delivers_messages() {
        let msgs: Vec<Vec<u8>> = vec![b"hello".to_vec(), b"world".to_vec()];
        let mut rx = RawReceiver::new(MockRecv::new(msgs.clone()));

        assert_eq!(rx.recv_one().unwrap(), b"hello");
        assert_eq!(rx.recv_one().unwrap(), b"world");
        assert_eq!(rx.recv_one().unwrap_err(), TransportError::Closed);
    }

    #[test]
    fn raw_receiver_is_alive_tracks_transport() {
        let mut rx = RawReceiver::new(MockRecv::new(vec![b"x".to_vec()]));
        assert!(rx.is_alive());
        let _ = rx.recv_one(); // consume the one message
        assert!(!rx.is_alive());
    }
}
