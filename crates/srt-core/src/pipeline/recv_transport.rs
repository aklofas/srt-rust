// crates/srt-core/src/pipeline/recv_transport.rs
//! `RecvTransport` — the receive-side counterpart to `Transport`.
//!
//! Like `Transport`, this is the seam between receive shells and the
//! wire. `SrtTransport` implements both. Future implementors (test
//! mocks, file-replay, in-memory channels) supply both.

use crate::pipeline::transport::TransportError;

/// Receive-side counterpart to [`Transport`][crate::pipeline::transport::Transport].
///
/// Each receive shell (`RawReceiver`, `TsReceiver`, `Receiver`) is generic
/// over a `RecvTransport`. `SrtTransport` implements both `Transport` and
/// `RecvTransport`. Test mocks and file-replay sources implement this trait
/// without needing a real socket.
pub trait RecvTransport: Send {
    /// Receive one message into `buf`. Returns the number of bytes written.
    ///
    /// Returns `Err(TransportError::Closed)` once the transport is closed or
    /// the connection has been broken and no further receive is possible.
    /// Returns `Err(TransportError::Backpressure)` on a recv timeout — the
    /// transport is still alive and the caller may retry.
    fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError>;

    /// Maximum bytes a single `recv_bytes` call may return. For SRT live
    /// mode this is `SRTO_PAYLOADSIZE` (default 1316). Receive shells use
    /// this to size their internal buffers on construction.
    fn max_payload(&self) -> usize;

    /// Advisory liveness check. Returns `false` when the transport is known
    /// dead. `true` does not guarantee the next `recv_bytes` succeeds.
    fn is_alive(&self) -> bool;

    /// Close the transport. Idempotent. After close, `recv_bytes` returns
    /// `TransportError::Closed`.
    ///
    /// Mirrors [`Transport::close`][crate::pipeline::transport::Transport::close].
    /// Defaulted as a no-op so test mocks and channel-backed implementors can
    /// opt in only when they own a tear-down resource.
    fn close(&mut self) {}
}
