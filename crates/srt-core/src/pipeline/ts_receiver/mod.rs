// crates/srt-core/src/pipeline/ts_receiver/mod.rs
//! `TsReceiver<R>` — pull bytes from a `RecvTransport`, run TS sync recovery,
//! and emit 188-byte aligned packets.
//!
//! `TsReceiver` is the receive-side counterpart to `TsSender`: where
//! `TsSender` wraps raw bytes into TS packets, `TsReceiver` receives a byte
//! stream and recovers packet alignment via the [`sync::Syncer`] state machine.
//!
//! Feed the muxed TS stream to the underlying `RecvTransport`; call
//! [`TsReceiver::next_packet`] repeatedly to drain one 188-byte packet at a
//! time. On a network gap or resync, the syncer silently re-hunts and
//! returns the next aligned packet.

pub mod sync;

use crate::pipeline::recv_transport::RecvTransport;
use crate::pipeline::transport::TransportError;
use sync::Syncer;

/// Receive shell that emits one 188-byte TS packet per call, with automatic
/// sync recovery.
///
/// `R` is any [`RecvTransport`] — typically `SrtTransport` for live
/// connections, or a test mock. The underlying transport framing (SRT live
/// mode) guarantees that each `recv_bytes` call returns one complete SRT
/// message; `TsReceiver` then feeds those bytes through the TS syncer to
/// extract correctly-aligned 188-byte packets.
pub struct TsReceiver<R: RecvTransport> {
    transport: R,
    syncer: Syncer,
    /// Reusable scratch buffer sized to `transport.max_payload()` on
    /// construction. Avoids a per-call heap allocation for the recv itself.
    recv_buf: Vec<u8>,
}

impl<R: RecvTransport> TsReceiver<R> {
    /// Wrap a transport. Allocates an internal receive buffer sized to
    /// `transport.max_payload()`.
    pub fn new(transport: R) -> Self {
        let cap = transport.max_payload();
        Self {
            transport,
            syncer: Syncer::new(),
            recv_buf: vec![0u8; cap],
        }
    }

    /// Block until at least one 188-byte TS packet is ready and return it.
    ///
    /// Internally:
    /// 1. Check whether the syncer already has a packet buffered (fast path,
    ///    avoids a transport call).
    /// 2. If not, call `recv_bytes` once to pull more data from the transport,
    ///    feed the bytes to the syncer, then retry.
    /// 3. Repeat until a packet is available or the transport closes.
    ///
    /// Returns `Err(TransportError::Closed)` when the transport has closed and
    /// the syncer's buffer is exhausted.
    /// Returns `Err(TransportError::Backpressure)` on a recv timeout — the
    /// transport is still alive; the caller may call `next_packet` again.
    pub fn next_packet(&mut self) -> Result<[u8; 188], TransportError> {
        loop {
            if let Some(pkt) = self.syncer.next_packet() {
                // The syncer always emits exactly 188 bytes when locked, so
                // the conversion is infallible. A Vec-to-array conversion via
                // try_into panics only if len != 188, which cannot happen here.
                let arr: [u8; 188] = pkt.try_into().unwrap();
                return Ok(arr);
            }
            let n = self.transport.recv_bytes(&mut self.recv_buf)?;
            // The RecvTransport contract says closed/broken transports return
            // Err(Closed), not Ok(0). Defensively treating Ok(0) as closed
            // guards against implementors that follow the io::Read convention
            // instead, and makes the loop terminate rather than spin.
            if n == 0 {
                return Err(TransportError::Closed);
            }
            self.syncer.push(&self.recv_buf[..n]);
        }
    }

    /// Advisory liveness check. Delegates to the underlying transport.
    pub fn is_alive(&self) -> bool {
        self.transport.is_alive()
    }

    /// Drop any buffered bytes and force the syncer back to HUNT.
    ///
    /// Intended for reconnect scenarios at a higher composition layer: when
    /// the underlying transport has been re-established, bytes left over
    /// from the dead connection must not seed the new alignment search.
    /// Note that `ManagedReceiveTransport` itself does **not** own the
    /// `TsReceiver` (it lives one layer up, inside `Receiver`); this method
    /// exists for a future `ManagedReceiver` shell to call.
    pub fn reset_sync(&mut self) {
        self.syncer.reset();
    }

    /// Close the underlying transport. Idempotent. After close, `next_packet`
    /// will return `TransportError::Closed` once the syncer's internal buffer
    /// is exhausted. Mirrors `RawReceiver::close`.
    pub fn close(&mut self) {
        self.transport.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::transport::TransportError;

    /// Minimal `RecvTransport` mock that plays back a fixed sequence of byte
    /// messages then signals closed. Each `recv_bytes` call returns one message
    /// in its entirety — mirroring SRT live-mode framing.
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

    /// Build a syntactically valid 188-byte TS packet with the given PID.
    fn ts_packet(pid: u16) -> [u8; 188] {
        let mut buf = [0xFFu8; 188];
        buf[0] = 0x47;
        buf[1] = 0x40 | ((pid >> 8) as u8 & 0x1F);
        buf[2] = (pid & 0xFF) as u8;
        buf[3] = 0x10;
        buf
    }

    /// Happy path: a well-formed TS stream arrives as a single large message.
    /// TsReceiver locks, emits all packets, then returns Closed.
    #[test]
    fn emits_packets_from_single_message() {
        let mut stream = Vec::new();
        for i in 0..6u16 {
            stream.extend_from_slice(&ts_packet(i));
        }
        let mut rx = TsReceiver::new(MockRecv::new(vec![stream]));

        let mut got = 0;
        loop {
            match rx.next_packet() {
                Ok(_) => got += 1,
                Err(TransportError::Closed) => break,
                Err(e) => panic!("unexpected error: {e:?}"),
            }
        }
        assert_eq!(got, 6);
    }

    /// Packets split across multiple transport messages (multi-call recv path).
    #[test]
    fn emits_packets_across_transport_messages() {
        // Each transport message is exactly one TS packet.
        let messages: Vec<Vec<u8>> = (0..5u16).map(|i| ts_packet(i).to_vec()).collect();
        let mut rx = TsReceiver::new(MockRecv::new(messages));

        let mut got = 0;
        loop {
            match rx.next_packet() {
                Ok(_) => got += 1,
                Err(TransportError::Closed) => break,
                Err(e) => panic!("unexpected error: {e:?}"),
            }
        }
        assert_eq!(got, 5);
    }

    /// Closed transport with no prior data returns Closed immediately.
    #[test]
    fn closed_transport_returns_closed() {
        let mut rx = TsReceiver::new(MockRecv::new(vec![]));
        assert_eq!(rx.next_packet().unwrap_err(), TransportError::Closed,);
    }
}
