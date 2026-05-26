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
    /// [`crate::RtpRecvTransport::recv_bytes`] then yields
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

    /// Caller invoked [`crate::RtspCancelHandle::cancel`] mid-request.
    /// The TCP write/read returned early; no server state was
    /// necessarily mutated, so caller should treat the session as
    /// indeterminate.
    #[error("RTSP request canceled by caller")]
    LocalCancel,

    /// `DESCRIBE` returned an SDP that contains no `m=` line with PT=33
    /// (MP2T, RFC 3551 §6). Only emitted by
    /// [`crate::RtspClient::setup_mp2t_auto`]; explicit
    /// `setup(&media)` does not consult MP2T-ness.
    #[error("no MPEG-TS m-line in SDP (no payload type 33)")]
    NoMp2tMedia,

    /// `DESCRIBE` returned an SDP with multiple `m=` lines containing
    /// PT=33. Caller should fall back to explicit
    /// [`crate::RtspClient::setup`] with a chosen media line.
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
