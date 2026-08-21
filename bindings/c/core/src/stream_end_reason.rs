//! `TstStreamEndReason` — C mirror of `tst_rtp::StreamEndReason`.
//!
//! Defined here, outside the `rtp` module, so cbindgen emits it
//! **unconditionally** — same reasoning as the sibling `TstReconnectMode`
//! in `config::builders`: the only feature-specific pieces are the two
//! getters that read it (`tst_rtp_receiver_end_reason` /
//! `tst_rtp_demux_receiver_end_reason`, both `TST_HAS_RTP`-gated in
//! `crate::rtp`), not the enum shape itself. Keeping the enum's own
//! definition inside a `#[cfg(feature = "rtp")]`-gated module would make
//! cbindgen wrap every variant in its own `#if defined(TST_HAS_RTP)` —
//! the doubled-guard shape the project deliberately avoids (see
//! `crate::config::builders::TstReconnectMode` for the same pattern and
//! `scripts/check/c/header-conditional-sections.sh`, which enforces
//! module-level-cfg-only gating).
//!
//! Conversion from the real `tst_rtp::StreamEndReason` (which *does*
//! need the `rtp` feature) lives in `crate::rtp::end_reason`.

/// Why an RTP receive session ended. Mirrors `tst_rtp::StreamEndReason`
/// with one addition — `None` (0) — for "hasn't ended yet, or ended
/// through a path this arc doesn't instrument" (the case
/// `StreamEndReasonHandle::get()` reports as `Option::None`, e.g. a plain
/// `rtp://` receiver that was never `_cancel`'d or `_close`'d).
/// Discriminants 1-6 are cross-surface stable — the Python and JVM
/// bindings use the same numbering.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TstStreamEndReason {
    /// The session hasn't ended yet, or ended through a path this arc
    /// doesn't instrument.
    None = 0,
    /// The peer closed the connection in an orderly way, with no
    /// protocol or transport error.
    CleanTeardown = 1,
    /// The server no longer honors the session — a keepalive ping was
    /// answered `454 Session Not Found`.
    SessionExpired = 2,
    /// The keepalive background thread failed to encode or send a ping.
    /// Detail message: see the getter doc.
    KeepaliveFailed = 3,
    /// A hard I/O error on the underlying transport. Detail message:
    /// see the getter doc.
    TransportFailed = 4,
    /// The peer violated the wire protocol. Detail message: see the
    /// getter doc.
    ProtocolError = 5,
    /// The caller explicitly cancelled or closed the transport — not a
    /// wire-level failure.
    Cancelled = 6,
}
