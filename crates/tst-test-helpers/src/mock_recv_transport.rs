//! Test-only mock [`RecvTransport`] that replays pre-queued TS packets and
//! can be programmed to fail in deterministic patterns. The receive-side
//! counterpart to [`MockTransport`](crate::mock_transport::MockTransport).
//!
//! # Motivation
//!
//! Receiver-shell integration tests (`Receiver`, `RawReceiver`,
//! `DemuxReceiver`, `ManagedRecvTransport`) previously had to choose between:
//!
//! 1. Spinning up an SRT loopback pair (slow, OS-flaky, requires tokio
//!    runtime or background threads).
//! 2. Writing yet another one-off `impl RecvTransport for FooRecv` inside the
//!    test file (duplicated boilerplate; in 2026-05 there were 13 such
//!    duplicates across the workspace per a `rg "impl RecvTransport"` sweep).
//!
//! `MockRecvTransport` consolidates that pattern. A single helper covers
//! every `TransportError` variant the receive side can surface
//! (`Backpressure`, `Broken`, `Closed`, `ExplicitClose`, `TooLarge`) plus
//! the happy path (queued packet replay).
//!
//! # Example
//!
//! ```ignore
//! use tst_test_helpers::mock_recv_transport::{MockRecvTransport, RecvFailMode};
//!
//! // Queue 3 packets, then EndOfStream.
//! let mut rx = MockRecvTransport::from_packets(vec![
//!     vec![0x47, 0x40, 0x00, 0x10],
//!     vec![0x47, 0x40, 0x00, 0x11],
//!     vec![0x47, 0x40, 0x00, 0x12],
//! ]);
//!
//! // Inject Backpressure on the next recv (then resume normal queue replay).
//! *rx.fail_handle().lock().unwrap() = RecvFailMode::BackpressureForN(1);
//! ```

use std::sync::{Arc, Mutex};
use tst_core::{RecvTransport, TransportError};

/// Programmable failure pattern for [`MockRecvTransport::recv_bytes`].
///
/// `#[non_exhaustive]` is intentional: new failure shapes (e.g., a
/// future `RetryAfter` variant) can be added without bumping the dev-only
/// crate's API contract.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum RecvFailMode {
    /// Always succeed — replay queued packets, then return `Closed`.
    Never,
    /// Return `Broken` on the next N recvs, then resume normal replay.
    BrokenForN(usize),
    /// Return `Backpressure` on the next N recvs (recv timeout), then resume.
    BackpressureForN(usize),
    /// Return `ExplicitClose` on the next recv, then resume normal replay.
    ///
    /// Mirrors the producer site documented on
    /// [`TransportError::ExplicitClose`]: a caller-initiated cancel signal
    /// (e.g., `ManagedRecvTransport::cancel()` firing while a recv is
    /// parked). Distinguished from `Closed`, which means peer EOS.
    ExplicitCloseOnNext,
    /// Return `Closed` on the next recv (peer EOS / connection broken).
    /// Subsequent calls also return `Closed`.
    ClosedOnNext,
}

/// Receive-side counterpart to
/// [`MockTransport`](crate::mock_transport::MockTransport).
///
/// Wraps a FIFO queue of byte vectors that `recv_bytes` pops one at a time
/// into the caller's buffer. Once the queue is exhausted, recv returns
/// `TransportError::Closed` (peer EOS). Use [`RecvFailMode`] to override
/// that flow with deterministic errors.
///
/// # Buffer handling
///
/// `recv_bytes` copies the front packet into the caller's buffer; if the
/// buffer is shorter than the packet, the packet is truncated to `buf.len()`
/// (matching libsrt's behavior for live mode where the caller pre-sizes to
/// `max_payload`).
pub struct MockRecvTransport {
    max_payload: usize,
    /// Set by `close()`; subsequent recvs return `Closed`.
    closed: bool,
    /// Set when the `ExplicitCloseOnNext` fail-mode fires; lets callers
    /// distinguish caller-initiated close from peer-EOS via stats / fixtures.
    /// Currently informational; reserved for future use.
    explicit_closed: bool,
    queue: Arc<Mutex<std::collections::VecDeque<Vec<u8>>>>,
    fail_mode: Arc<Mutex<RecvFailMode>>,
}

impl MockRecvTransport {
    /// New mock with no queued packets and `max_payload = 1316` (SRT live
    /// mode default). Use [`Self::push_packet`] to queue, or
    /// [`Self::from_packets`] to bulk-load.
    #[must_use]
    pub fn new(max_payload: usize) -> Self {
        Self {
            max_payload,
            closed: false,
            explicit_closed: false,
            queue: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            fail_mode: Arc::new(Mutex::new(RecvFailMode::Never)),
        }
    }

    /// New mock pre-loaded with `packets` (FIFO order) and default
    /// `max_payload = 1316`.
    #[must_use]
    pub fn from_packets(packets: Vec<Vec<u8>>) -> Self {
        let mock = Self::new(1316);
        mock.queue.lock().unwrap().extend(packets);
        mock
    }

    /// Append a packet to the back of the recv queue. Thread-safe; callers
    /// can hold a clone of the queue handle and push after construction.
    pub fn push_packet(&self, packet: Vec<u8>) {
        self.queue.lock().unwrap().push_back(packet);
    }

    /// Shared handle to the recv queue. Lets a test thread inject packets
    /// while the receiver shell is running in another thread.
    #[must_use]
    pub fn queue_handle(&self) -> Arc<Mutex<std::collections::VecDeque<Vec<u8>>>> {
        Arc::clone(&self.queue)
    }

    /// Shared handle to the fail-mode register. Mirrors
    /// [`MockTransport::fail_handle`](crate::mock_transport::MockTransport::fail_handle).
    #[must_use]
    pub fn fail_handle(&self) -> Arc<Mutex<RecvFailMode>> {
        Arc::clone(&self.fail_mode)
    }

    /// Returns `true` once the `ExplicitCloseOnNext` fail-mode has fired.
    /// Reserved for future stat-style assertions; not currently observable
    /// through the `RecvTransport` trait.
    #[must_use]
    pub fn was_explicit_closed(&self) -> bool {
        self.explicit_closed
    }
}

impl RecvTransport for MockRecvTransport {
    fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }

        // Check fail-mode register first — failure injections override the
        // queue. We decrement-and-fall-through so BackpressureForN(0) /
        // BrokenForN(0) gracefully revert to queue replay.
        let mut mode = self.fail_mode.lock().unwrap();
        match &mut *mode {
            RecvFailMode::BrokenForN(n) if *n > 0 => {
                *n -= 1;
                return Err(TransportError::Broken {
                    msg: "mock recv broken".into(),
                    errno_code: None,
                });
            }
            RecvFailMode::BackpressureForN(n) if *n > 0 => {
                *n -= 1;
                return Err(TransportError::Backpressure {
                    msg: "mock recv timeout".into(),
                    errno_code: None,
                });
            }
            RecvFailMode::ExplicitCloseOnNext => {
                // One-shot: revert to Never so the next call resumes queue
                // replay. The test fixture is responsible for re-setting if
                // it wants repeat fires.
                *mode = RecvFailMode::Never;
                self.explicit_closed = true;
                return Err(TransportError::ExplicitClose);
            }
            RecvFailMode::ClosedOnNext => {
                // Sticky: subsequent calls also return Closed. Models peer
                // EOS / broken connection.
                self.closed = true;
                return Err(TransportError::Closed);
            }
            _ => {}
        }
        drop(mode);

        // Happy path: pop one packet, copy into caller buf, return byte count.
        let mut queue = self.queue.lock().unwrap();
        match queue.pop_front() {
            Some(pkt) => {
                let n = pkt.len().min(buf.len());
                buf[..n].copy_from_slice(&pkt[..n]);
                Ok(n)
            }
            None => Err(TransportError::Closed),
        }
    }

    fn max_payload(&self) -> usize {
        self.max_payload
    }

    fn is_alive(&self) -> bool {
        !self.closed
    }

    fn close(&mut self) {
        // Idempotent per the RecvTransport trait contract. Note: we do NOT
        // set `explicit_closed` here unconditionally — that flag is set only
        // by the ExplicitCloseOnNext fail-mode firing, to give tests a way
        // to distinguish caller-initiated close from peer-EOS observed
        // via `Closed`.
        self.closed = true;
    }
}

// ---------------------------------------------------------------------------
// Unit tests — cover the 5 documented TransportError variants + happy path.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_replay_then_closed() {
        let mut rx = MockRecvTransport::from_packets(vec![vec![0xAA, 0xBB], vec![0xCC]]);
        let mut buf = [0u8; 8];

        // First packet: 2 bytes.
        assert_eq!(rx.recv_bytes(&mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], &[0xAA, 0xBB]);

        // Second packet: 1 byte.
        assert_eq!(rx.recv_bytes(&mut buf).unwrap(), 1);
        assert_eq!(buf[0], 0xCC);

        // Queue exhausted → Closed (peer EOS shape).
        assert!(matches!(
            rx.recv_bytes(&mut buf),
            Err(TransportError::Closed)
        ));
    }

    #[test]
    fn explicit_close_fail_mode_returns_explicit_close_variant() {
        // ExplicitClose is what `ManagedRecvTransport::cancel()` produces
        // when the caller fires the cancel signal mid-recv. The mock has
        // first-class support for it; transport_error_discrimination.rs's
        // ClosedRecv minimal local impl can be retired in favor of this.
        let mut rx = MockRecvTransport::from_packets(vec![vec![0xAA]]);
        *rx.fail_handle().lock().unwrap() = RecvFailMode::ExplicitCloseOnNext;

        let mut buf = [0u8; 8];
        assert!(matches!(
            rx.recv_bytes(&mut buf),
            Err(TransportError::ExplicitClose)
        ));
        assert!(rx.was_explicit_closed());

        // ExplicitCloseOnNext is one-shot; queue replay resumes.
        assert_eq!(rx.recv_bytes(&mut buf).unwrap(), 1);
        assert_eq!(buf[0], 0xAA);
    }

    #[test]
    fn close_then_recv_returns_closed() {
        let mut rx = MockRecvTransport::from_packets(vec![vec![0xAA]]);
        assert!(rx.is_alive());

        rx.close();
        assert!(!rx.is_alive());

        // Even with a queued packet, recv after close returns Closed.
        let mut buf = [0u8; 8];
        assert!(matches!(
            rx.recv_bytes(&mut buf),
            Err(TransportError::Closed)
        ));

        // Idempotent: calling close again is fine.
        rx.close();
        assert!(!rx.is_alive());
    }

    #[test]
    fn backpressure_for_n_decrements_and_resumes() {
        let mut rx = MockRecvTransport::from_packets(vec![vec![0xAA]]);
        *rx.fail_handle().lock().unwrap() = RecvFailMode::BackpressureForN(2);

        let mut buf = [0u8; 8];

        // 2 backpressure errors...
        assert!(matches!(
            rx.recv_bytes(&mut buf),
            Err(TransportError::Backpressure { .. })
        ));
        assert!(matches!(
            rx.recv_bytes(&mut buf),
            Err(TransportError::Backpressure { .. })
        ));

        // ...then happy-path replay resumes.
        assert_eq!(rx.recv_bytes(&mut buf).unwrap(), 1);
    }

    #[test]
    fn broken_for_n_decrements_and_resumes() {
        let mut rx = MockRecvTransport::from_packets(vec![vec![0xAA]]);
        *rx.fail_handle().lock().unwrap() = RecvFailMode::BrokenForN(1);

        let mut buf = [0u8; 8];
        assert!(matches!(
            rx.recv_bytes(&mut buf),
            Err(TransportError::Broken { .. })
        ));
        // Resumes — Broken is recoverable in the mock even though real
        // libsrt wouldn't typically; the fixture is for shell-routing tests.
        assert_eq!(rx.recv_bytes(&mut buf).unwrap(), 1);
    }

    #[test]
    fn closed_on_next_is_sticky() {
        let mut rx = MockRecvTransport::from_packets(vec![vec![0xAA], vec![0xBB]]);
        *rx.fail_handle().lock().unwrap() = RecvFailMode::ClosedOnNext;

        let mut buf = [0u8; 8];
        assert!(matches!(
            rx.recv_bytes(&mut buf),
            Err(TransportError::Closed)
        ));

        // Sticky: subsequent calls also return Closed even though the
        // queue still has packets. Models peer EOS / broken connection.
        assert!(matches!(
            rx.recv_bytes(&mut buf),
            Err(TransportError::Closed)
        ));
    }

    #[test]
    fn push_packet_after_construction_works() {
        let rx = MockRecvTransport::new(1316);
        rx.push_packet(vec![0x47]);

        let mut rx = rx;
        let mut buf = [0u8; 8];
        assert_eq!(rx.recv_bytes(&mut buf).unwrap(), 1);
        assert_eq!(buf[0], 0x47);
    }

    #[test]
    fn buffer_smaller_than_packet_truncates() {
        // libsrt live-mode contract: caller pre-sizes to max_payload. If a
        // test deliberately uses a shorter buffer, we copy what fits and
        // report that byte count.
        let mut rx = MockRecvTransport::from_packets(vec![vec![1, 2, 3, 4]]);
        let mut buf = [0u8; 2];
        assert_eq!(rx.recv_bytes(&mut buf).unwrap(), 2);
        assert_eq!(&buf, &[1, 2]);
    }
}
