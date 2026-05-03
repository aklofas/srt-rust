// crates/srt-core/src/pipeline/transport.rs
//! `Transport` — the seam between sender shells and the wire.
//!
//! Each sender shell (`Sender`, `TsSender`, `RawSender`) is generic over a
//! `Transport`. The bare implementation (`SrtTransport`) calls `srt::Socket`
//! directly; the `ManagedTransport` decorator (in `pipeline::reconnect`)
//! wraps any inner `Transport` with reconnect + gap buffering. This is the
//! single seam where retry, gap-buffering, and (eventually) async-send
//! variants insert.

use thiserror::Error;

/// Failure shape for `Transport::send_bytes`.
///
/// Two semantic categories:
///
/// - **Recoverable** — the transport is alive but momentarily refused
///   (libsrt's `SRT_EASYNCSND` from a full send buffer, e.g.). Caller may
///   retry without rebuilding the transport.
/// - **Broken** — the transport is dead and must be re-established. Plain
///   senders surface this to the caller; `ManagedTransport` triggers
///   reconnect.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransportError {
    /// Transport was alive but couldn't accept the bytes right now (full
    /// send buffer, transient backpressure). Retrying the same bytes later
    /// is reasonable.
    #[error("transport temporarily unavailable: {0}")]
    Backpressure(String),

    /// Transport is dead. Caller must rebuild it (or rely on a managed
    /// wrapper to do so).
    #[error("transport broken: {0}")]
    Broken(String),

    /// Transport was already closed.
    #[error("transport closed")]
    Closed,

    /// `len > maximum payload size`. For SRT live mode, the cap is
    /// `SRTO_PAYLOADSIZE` (default 1316). Caller is responsible for
    /// chunking on their own framing semantics.
    #[error("message too large: {len} bytes exceeds payload-size cap of {max} bytes")]
    TooLarge { len: usize, max: usize },
}

/// One-shot byte transport. Each `send_bytes` call sends exactly one
/// outbound message.
///
/// Implementors are expected to be `Send` so a single transport can move
/// between threads (e.g., a builder thread hands off to a sender thread).
/// They are NOT expected to be `Sync` — concurrent sends through one
/// transport are the caller's responsibility (the sender shells handle
/// this internally where their thread-safety contract requires it).
pub trait Transport: Send {
    /// Send one message. Returns success once libsrt (or the equivalent)
    /// has accepted the bytes — not when they reach the peer.
    ///
    /// On `Err(TransportError::Backpressure)`, the bytes have NOT been
    /// partially consumed; callers may retry the identical slice. On any
    /// other error the transport state is undefined and retrying the same
    /// bytes is not safe (rebuild the transport, or rely on a managed
    /// wrapper).
    fn send_bytes(&mut self, msg: &[u8]) -> Result<(), TransportError>;

    /// Maximum payload size for this transport. For SRT live mode this is
    /// `SRTO_PAYLOADSIZE` (default 1316). Senders use this to size
    /// per-send buffers and to validate raw-sender input.
    fn max_payload(&self) -> usize;

    /// Advisory liveness check. Returns `false` when the transport is
    /// known dead (closed or previously broken). `true` does not
    /// guarantee that the next `send_bytes` succeeds — the transport
    /// may die between this call and the send. Callers must handle
    /// `TransportError::Broken` regardless of what `is_alive` returns.
    fn is_alive(&self) -> bool;

    /// Close the transport. Idempotent — calling close twice is fine.
    /// After close, `send_bytes` returns `TransportError::Closed`.
    ///
    /// The `&mut self` requirement means callers (or the sender shells
    /// wrapping the transport) are responsible for serializing close
    /// with concurrent sends. The "close from any thread is always safe"
    /// contract in the wider design lives at the sender-shell layer,
    /// not on `Transport` itself.
    fn close(&mut self);

    /// Optional cancellation accessor. Implementors that own a wakeable
    /// blocking primitive (a real socket, an MPSC channel sender, etc.)
    /// return `Some(handle)`; pure in-memory test mocks return `None`.
    ///
    /// The returned handle is `Send + Sync` and can be moved/cloned to
    /// any thread; calling `cancel()` while another thread is parked in
    /// [`Self::send_bytes`] makes that parked call return
    /// `TransportError::Broken`.
    fn cancel_handle(&self) -> Option<Box<dyn TransportCancel>> {
        None
    }
}

/// Type-erased cancel-handle accessor returned by
/// [`Transport::cancel_handle`] and
/// [`crate::pipeline::recv_transport::RecvTransport::cancel_handle`].
///
/// `cancel()` from any thread interrupts the transport's current blocking
/// send or receive — the parked call returns `Broken` (or whatever the
/// inner mapping produces). Idempotent.
///
/// `Send + Sync` is required so consumers can stash one in an
/// `Arc<dyn TransportCancel>` and share it across worker threads.
pub trait TransportCancel: Send + Sync {
    fn cancel(&self);
}
