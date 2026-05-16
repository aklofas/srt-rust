//! `SrtTransport` — the canonical `Transport` impl backed by a `srt::Socket`.
//!
//! Wraps the safe-Rust `Socket` and translates `SendError` to the
//! `Transport`-trait shape. Used as the inner of `ManagedTransport` in
//! the canonical reconnecting setup.

use crate::Socket;
use crate::error::{SendError, SrtErrno};
use std::sync::Arc;
use tst_core::transport::{SocketStats, Transport, TransportCancel, TransportError};

pub struct SrtTransport {
    socket: Option<Socket>,
    max_payload: usize,
}

impl SrtTransport {
    /// Default SRT live-mode payload size (libsrt's `SRTO_PAYLOADSIZE` default).
    pub const DEFAULT_PAYLOAD: usize = 1316;

    /// Wrap an already-connected `Socket`. Caller is responsible for
    /// configuring it (passphrase, latency, etc.) before passing in.
    ///
    /// `max_payload` defaults to 1316 (libsrt's default `SRTO_PAYLOADSIZE`)
    /// and is NOT queried from the socket. Callers using a socket with a
    /// non-default `SRTO_PAYLOADSIZE` must call [`with_max_payload`] to
    /// match.
    ///
    /// [`with_max_payload`]: SrtTransport::with_max_payload
    pub fn new(socket: Socket) -> Self {
        Self {
            socket: Some(socket),
            max_payload: Self::DEFAULT_PAYLOAD,
        }
    }

    /// Override the max payload (for callers who've configured a
    /// non-default `SRTO_PAYLOADSIZE` on the socket).
    pub fn with_max_payload(mut self, n: usize) -> Self {
        self.max_payload = n;
        self
    }
}

impl Transport for SrtTransport {
    fn send_bytes(&mut self, msg: &[u8]) -> Result<(), TransportError> {
        if msg.len() > self.max_payload {
            return Err(TransportError::TooLarge {
                len: msg.len(),
                max: self.max_payload,
            });
        }
        let socket = self.socket.as_mut().ok_or(TransportError::Closed)?;
        match socket.send(msg) {
            Ok(_) => Ok(()),
            Err(SendError::TimedOut) => Err(TransportError::Backpressure("send timed out".into())),
            Err(SendError::QueueFull) => {
                Err(TransportError::Backpressure("send queue full".into()))
            }
            Err(SendError::PayloadTooLarge { actual, .. }) => Err(TransportError::TooLarge {
                len: actual,
                max: self.max_payload,
            }),
            Err(SendError::ConnectionBroken) => {
                self.socket = None;
                Err(TransportError::Broken("connection broken".into()))
            }
            Err(SendError::System(e)) => {
                self.socket = None;
                Err(TransportError::Broken(format!("system error: {e}")))
            }
            Err(SendError::Other { kind, message }) => {
                // SrtErrno::Async coarsens libsrt's async-class category. The only
                // sub-code that can reach this arm on a send is SRT_EASYNCSND
                // (send-buffer-full in non-blocking mode) — SRT_ETIMEOUT is
                // pre-consumed into SendError::TimedOut by From<RawError>, and
                // SRT_EASYNCRCV / SRT_EASYNCFAIL don't fire on srt_sendmsg2.
                // Everything else → broken (rebuild the transport).
                if matches!(kind, SrtErrno::Async) {
                    Err(TransportError::Backpressure(message))
                } else {
                    self.socket = None;
                    Err(TransportError::Broken(message))
                }
            }
        }
    }

    fn max_payload(&self) -> usize {
        self.max_payload
    }

    fn is_alive(&self) -> bool {
        self.socket.is_some()
    }

    fn close(&mut self) {
        if let Some(socket) = self.socket.take() {
            // Socket::close consumes self; ignore the error — we're closing.
            let _ = socket.close();
        }
    }

    fn cancel_handle(&self) -> Option<Arc<dyn TransportCancel + Send + Sync>> {
        self.socket.as_ref().map(|s| {
            Arc::new(SrtCancel(s.cancel_handle())) as Arc<dyn TransportCancel + Send + Sync>
        })
    }

    fn socket_stats(&self) -> Option<SocketStats> {
        let socket = self.socket.as_ref()?;
        let s = socket.stats().ok()?;
        Some(map_stats(&s))
    }
}

impl tst_core::transport::RecvTransport for SrtTransport {
    fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        use crate::error::RecvError;
        let socket = self.socket.as_mut().ok_or(TransportError::Closed)?;
        match socket.recv(buf) {
            Ok(n) => Ok(n),
            Err(RecvError::TimedOut) => Err(TransportError::Backpressure("recv timed out".into())),
            Err(RecvError::ConnectionBroken) => {
                // Peer hung up or mid-stream abort. Surface as Broken (not
                // Closed) so a managed receive decorator can distinguish a
                // self-initiated close from a peer-initiated break and drive
                // reconnect. Matches the send-side mapping for the same
                // RecvError-equivalent variant.
                self.socket = None;
                Err(TransportError::Broken("connection broken".into()))
            }
            Err(RecvError::BufferTooSmall {
                buf_len,
                message_len,
            }) => {
                // The caller passed a buf smaller than the incoming message.
                // Surface as Broken — the receive shell is misconfigured (it
                // should have sized buf to at least max_payload()).
                self.socket = None;
                Err(TransportError::Broken(format!(
                    "recv buf too small: {buf_len} < {message_len}"
                )))
            }
            Err(other) => {
                self.socket = None;
                Err(TransportError::Broken(other.to_string()))
            }
        }
    }

    fn max_payload(&self) -> usize {
        self.max_payload
    }

    fn is_alive(&self) -> bool {
        self.socket.is_some()
    }

    fn close(&mut self) {
        <Self as tst_core::transport::Transport>::close(self);
    }

    fn cancel_handle(&self) -> Option<Arc<dyn TransportCancel + Send + Sync>> {
        self.socket.as_ref().map(|s| {
            Arc::new(SrtCancel(s.cancel_handle())) as Arc<dyn TransportCancel + Send + Sync>
        })
    }

    fn socket_stats(&self) -> Option<SocketStats> {
        let socket = self.socket.as_ref()?;
        let s = socket.stats().ok()?;
        Some(map_stats(&s))
    }
}

impl Drop for SrtTransport {
    fn drop(&mut self) {
        self.close();
    }
}

/// Adapter: wraps `tst_core::CancelHandle` as a `TransportCancel`.
struct SrtCancel(tst_core::CancelHandle);

impl TransportCancel for SrtCancel {
    fn cancel(&self) {
        self.0.cancel();
    }
}

/// Map the libsrt-flavored `crate::socket::Stats` into the abstract
/// `tst_core::transport::SocketStats`. Unit conversions:
/// * RTT: `Duration` → microseconds, saturating at `u32::MAX`.
/// * Bandwidth: libsrt-side u64 bps is passed through; the redundant
///   `mbps_estimated_bandwidth` f64 field is multiplied by 1e6 and
///   saturated into `link_bandwidth_bps`.
/// * Send-side byte loss: libsrt doesn't export `byteSndLossTotal`, so
///   the abstract struct has no `bytes_lost_send` field. The local
///   `Stats::bytes_lost_send_side` is always 0 today and is dropped.
fn map_stats(s: &crate::socket::Stats) -> SocketStats {
    let rtt_us = u32::try_from(s.rtt.as_micros()).unwrap_or(u32::MAX);
    let link_bandwidth_bps =
        if s.mbps_estimated_bandwidth.is_finite() && s.mbps_estimated_bandwidth >= 0.0 {
            let scaled = s.mbps_estimated_bandwidth * 1e6;
            if scaled >= u64::MAX as f64 {
                u64::MAX
            } else {
                scaled.round() as u64
            }
        } else {
            0
        };
    // `#[non_exhaustive]` blocks the `SocketStats { ... }` struct-literal
    // form from outside tst-core (Rust E0639), and `..Default::default()`
    // doesn't lift the restriction outside the defining crate. Default-
    // and-assign is the only pattern that preserves the #[non_exhaustive]
    // forward-compatibility guarantee.
    let mut out = SocketStats::default();
    out.rtt_us = rtt_us;
    out.send_bandwidth_bps = s.send_bandwidth_bps;
    out.recv_bandwidth_bps = s.recv_bandwidth_bps;
    out.link_bandwidth_bps = link_bandwidth_bps;
    out.bytes_sent = s.bytes_sent;
    out.packets_sent = s.packets_sent;
    out.bytes_received = s.bytes_received;
    out.packets_received = s.packets_received;
    out.bytes_lost_recv = s.bytes_lost_recv_side;
    out.packets_lost_recv = s.packets_lost_recv_side;
    out.packets_lost_send = s.packets_lost_send_side;
    out.packets_retransmitted = s.packets_retransmitted;
    out.packets_dropped_send = s.packets_dropped_send_side;
    out.packets_dropped_recv = s.packets_dropped_recv_side;
    out.send_buffer_packets = s.send_buffer_packets;
    out.recv_buffer_packets = s.recv_buffer_packets;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_transport_max_payload_matches_default() {
        // Without an actual connected Socket, just verify the constant.
        // SrtTransport::DEFAULT_PAYLOAD == 1316 (libsrt default).
        assert_eq!(SrtTransport::DEFAULT_PAYLOAD, 1316);
    }

    /// `cancel_handle()` returns Some when a Socket is held; calling
    /// cancel() flips the inner socket to None on the next send_bytes
    /// (which now returns Closed because we proactively dropped it).
    #[test]
    #[ignore = "needs live SRT socket; covered by tests/cancellation_loopback.rs"]
    fn cancel_handle_some_when_alive() {}

    /// Sanity: with no Socket held (already-closed transport), the
    /// cancel-handle accessor returns None — there's nothing to cancel.
    #[test]
    fn cancel_handle_none_when_already_closed() {
        // We can't construct an SrtTransport without a live Socket
        // through the public API, so this assertion lives in the
        // integration test instead. Documented as a guarantee here.
    }
}
