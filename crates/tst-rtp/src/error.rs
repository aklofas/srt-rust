//! RTSP-control-plane errors.
//!
//! RTSP failures do not fit the [`tst_core::transport::TransportError`]
//! semantics — RTSP is a separate state machine before the
//! [`crate::RtpRecvTransport`] is constructed. This type carries all
//! RTSP-side failures up to the caller; post-SETUP failures (TCP RST
//! mid-PLAY, UDP recv errors) bubble through the transport's normal
//! `TransportError::Broken` path.
//!
//! Total variants: 15 (Phase 2 master-spec 12 + Url + NoMp2tMedia +
//! MultipleMp2tMedia).

use std::io;

use crate::url::UrlError;

/// Failure shape for the RTSP client state machine.
///
/// Constructed at every point where the client may fail before the
/// pipeline is wired up. Once [`crate::RtpRecvTransport`] is in hand,
/// subsequent failures bubble through `TransportError` instead.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RtspError {
    /// Socket-level I/O on the control channel (connect refused,
    /// connection reset, etc.). Mirrors Phase 1's
    /// [`crate::ConnectError::Io`] but scoped to the RTSP TCP connection.
    #[error("RTSP I/O error: {0:?}")]
    Io(io::ErrorKind),

    /// rustls handshake / certificate-verification failure (only emitted
    /// when the `tls` feature is enabled).
    #[error("RTSP TLS error: {0}")]
    Tls(String),

    /// 4xx or 5xx server response that doesn't match a more specific
    /// variant. `code` is the numeric status (e.g., 404 Not Found),
    /// `reason` is the human-readable reason phrase ("Stream Not
    /// Found").
    #[error("RTSP server returned {code} {reason}")]
    Protocol { code: u16, reason: String },

    /// 401 Unauthorized after credential retry. Either credentials were
    /// missing/wrong, or the server returned a fresh nonce that didn't
    /// help on the second attempt.
    #[error("RTSP authentication failed")]
    AuthFailed,

    /// Server demanded an auth scheme we don't implement (e.g., NTLM,
    /// Bearer). `scheme` is the lowercase scheme name from the
    /// `WWW-Authenticate` header.
    #[error("RTSP auth scheme not supported: {scheme}")]
    AuthUnsupported { scheme: String },

    /// Server response couldn't be parsed by `rtsp-types`. `detail` is a
    /// short reason ("missing CSeq header", "truncated body").
    #[error("malformed RTSP response: {detail}")]
    BadResponse { detail: &'static str },

    /// SDP from `DESCRIBE` couldn't be parsed by `sdp-types`. `detail`
    /// is the parse-error rendering.
    #[error("malformed SDP from server: {detail}")]
    BadSdp { detail: String },

    /// Server returned 461 Unsupported Transport on both UDP and TCP
    /// attempts, or `?transport=` URL query forced a transport the
    /// server refused.
    #[error("RTSP server does not support a transport we accept")]
    UnsupportedTransport,

    /// Malformed `$<channel:u8><length:u16><payload>` interleaved frame
    /// from the server (length field doesn't match payload size,
    /// channel out of allocated range, etc.). The control channel
    /// surfaces this immediately; downstream
    /// `RtpRecvTransport::recv_bytes` then yields
    /// `TransportError::Broken`.
    #[error("malformed interleaved frame: {detail}")]
    InterleavedFraming { detail: &'static str },

    /// Server closed our session (sent RTSP/1.0 454 Session Not Found
    /// on a keepalive ping, or the underlying TCP went RST). After
    /// this, no further requests succeed; caller must construct a fresh
    /// `RtspClient`.
    #[error("RTSP session expired or was closed by server")]
    SessionExpired,

    /// Server's `Session: ...;timeout=N` interval elapsed without a
    /// successful keepalive ping. Distinct from
    /// [`RtspError::SessionExpired`] in that the timeout fired locally
    /// (we never got a response) rather than receiving an explicit
    /// session-not-found status.
    #[error("RTSP session timed out (no response to keepalive)")]
    Timeout,

    /// Caller invoked `RtspCancelHandle::cancel` (lands Wave B) mid-request.
    /// The TCP write/read returned early; no server state was
    /// necessarily mutated, so caller should treat the session as
    /// indeterminate.
    #[error("RTSP request canceled by caller")]
    LocalCancel,

    /// `DESCRIBE` returned an SDP that contains no `m=` line with PT=33
    /// (MP2T, RFC 3551 §6). Only emitted by
    /// `RtspClient::setup_mp2t_auto` (lands Wave B); explicit
    /// `setup(&media)` does not consult MP2T-ness.
    #[error("no MPEG-TS m-line in SDP (no payload type 33)")]
    NoMp2tMedia,

    /// `DESCRIBE` returned an SDP with multiple `m=` lines containing
    /// PT=33. Caller should fall back to explicit
    /// `RtspClient::setup` (lands Wave B) with a chosen media line.
    #[error("multiple MPEG-TS m-lines in SDP ({count} found)")]
    MultipleMp2tMedia { count: usize },

    /// URL parsing failed before any RTSP exchange. Wraps the
    /// underlying [`crate::RtpUrlError`] from Phase 1.
    #[error("RTSP URL parse error: {0}")]
    Url(#[from] UrlError),
}

impl From<io::Error> for RtspError {
    fn from(e: io::Error) -> Self {
        RtspError::Io(e.kind())
    }
}

/// Failure shape for `RtspServer` (introduced in Phase 3 Task 7)
/// lifecycle and configuration. Per-session errors (one client
/// misbehaving) do NOT surface here — they are logged via
/// `tracing::warn!` and the session closes; the server keeps running.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RtspServerError {
    /// Socket-level I/O on the listener.
    #[error("RTSP server I/O error: {0:?}")]
    Io(io::ErrorKind),

    /// rustls server-side handshake / cert-loading failure (only emitted
    /// when the `tls` feature is enabled).
    #[error("RTSP server TLS error: {0}")]
    Tls(String),

    /// Bind URL parsing failed before any server lifecycle.
    #[error("RTSP server bind URL parse error: {0}")]
    UrlParse(#[from] UrlError),

    /// The bind URL's host:port pair could not be claimed (another process
    /// holds it, or insufficient privileges for the port).
    #[error("bind address in use")]
    BindAddrInUse,

    /// `add_mount("/path", ...)` rejected the path — empty, doesn't start
    /// with `/`, contains URL-reserved characters, etc.
    #[error("invalid mount path: {detail}")]
    InvalidMountPath { detail: String },

    /// `add_multicast_mount(...)` rejected the group address — not in the
    /// 224.0.0.0/4 or ff00::/8 ranges, or the URL is malformed.
    #[error("invalid multicast group '{addr}': {detail}")]
    InvalidMulticastGroup { addr: String, detail: String },

    /// `add_mount("/path", ...)` called twice with the same path.
    #[error("duplicate mount path '{path}'")]
    DuplicateMount { path: String },

    /// `MuxerConfig` failed validation (no programs declared, etc.) or
    /// some other configuration-time invariant was violated.
    #[error("invalid mount config: {detail}")]
    InvalidConfig { detail: String },

    /// `start()` called twice without an intervening `stop()`.
    #[error("RTSP server already started")]
    AlreadyStarted,

    /// `stop()`, `add_mount()`, or similar called before `start()`.
    #[error("RTSP server not started")]
    NotStarted,

    /// Public method invoked after `stop()` completed (or after `cancel()`).
    #[error("RTSP server has been shut down")]
    Shutdown,
}

/// Failure shape for `MountHandle` (introduced in Phase 3 Wave C) push methods.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MountError {
    /// The inner muxer surfaced an error during push or drain.
    #[error("muxer error: {0}")]
    Mux(#[from] tst_core::error::MuxError),

    /// The mount's parent server has been shut down (or the mount was
    /// explicitly removed in a future API).
    #[error("mount closed")]
    Closed,

    /// At least one peer's broadcast subscriber lagged past capacity; the
    /// `dropped_frames` count is informational (the push itself
    /// succeeded). Callers wanting end-to-end no-drop semantics can react
    /// by slowing their push rate.
    #[error("peer backpressure: {dropped_frames} frames dropped on slow peers")]
    PeerBackpressure { dropped_frames: u64 },
}
