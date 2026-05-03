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

/// Aggregate receive stats for [`RawReceiver`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RawReceiverStats {
    /// Total bytes received from the transport.
    pub bytes_received: u64,
    /// Count of successful `recv_one()` calls.
    pub packets_received: u64,
}

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
    stats: RawReceiverStats,
}

impl<R: RecvTransport> RawReceiver<R> {
    /// Wrap a transport. Allocates an internal buffer sized to
    /// `transport.max_payload()`.
    pub fn new(transport: R) -> Self {
        let cap = transport.max_payload();
        Self {
            transport,
            buf: vec![0u8; cap],
            stats: RawReceiverStats::default(),
        }
    }

    /// Block until one message arrives. Returns a copy of the received bytes.
    ///
    /// Returns `Err(TransportError::Closed)` when the connection has ended.
    /// Returns `Err(TransportError::Backpressure)` on a recv timeout — the
    /// transport is still alive; the caller may call `recv_one` again.
    pub fn recv_one(&mut self) -> Result<Vec<u8>, TransportError> {
        let n = self.transport.recv_bytes(&mut self.buf)?;
        self.stats.bytes_received += n as u64;
        self.stats.packets_received += 1;
        Ok(self.buf[..n].to_vec())
    }

    /// Return a snapshot of aggregate receive stats.
    pub fn stats(&self) -> RawReceiverStats {
        self.stats
    }

    /// Zero all counters. Does not affect the underlying transport.
    pub fn reset_stats(&mut self) {
        self.stats = RawReceiverStats::default();
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
    use crate::pipeline::recv_transport::RecvTransport;
    use crate::pipeline::transport::TransportError;

    struct MemRecv {
        queue: std::collections::VecDeque<Vec<u8>>,
        alive: bool,
    }
    impl RecvTransport for MemRecv {
        fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
            match self.queue.pop_front() {
                Some(v) => {
                    let n = v.len().min(buf.len());
                    buf[..n].copy_from_slice(&v[..n]);
                    Ok(n)
                }
                None => Err(TransportError::Closed),
            }
        }
        fn max_payload(&self) -> usize {
            1316
        }
        fn is_alive(&self) -> bool {
            self.alive
        }
    }

    #[test]
    fn stats_starts_zero() {
        let r = RawReceiver::new(MemRecv {
            queue: Default::default(),
            alive: true,
        });
        let st = r.stats();
        assert_eq!(st.bytes_received, 0);
        assert_eq!(st.packets_received, 0);
    }

    #[test]
    fn stats_increment_on_recv() {
        let mut q = std::collections::VecDeque::new();
        q.push_back(vec![1u8; 100]);
        q.push_back(vec![2u8; 50]);
        let mut r = RawReceiver::new(MemRecv {
            queue: q,
            alive: true,
        });
        let _ = r.recv_one();
        let _ = r.recv_one();
        let st = r.stats();
        assert_eq!(st.bytes_received, 150);
        assert_eq!(st.packets_received, 2);
    }

    #[test]
    fn reset_zeros_counters() {
        let mut q = std::collections::VecDeque::new();
        q.push_back(vec![1u8; 100]);
        let mut r = RawReceiver::new(MemRecv {
            queue: q,
            alive: true,
        });
        let _ = r.recv_one();
        r.reset_stats();
        let st = r.stats();
        assert_eq!(st.bytes_received, 0);
        assert_eq!(st.packets_received, 0);
    }

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
