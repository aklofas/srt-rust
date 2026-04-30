//! Error types for srt-core.
//!
//! Per-call-category enums (`ConnectError`, `BindError`, etc.) are the
//! Rust-idiomatic surface — exhaustive `match` is meaningful at every call
//! site. The umbrella `Error` exists for callers who want one type to
//! propagate across categories.

use std::io;
use thiserror::Error;

// ============================================================================
// Validation errors (newtype constructors)
// ============================================================================

#[derive(Debug, Error)]
pub enum PassphraseError {
    #[error("passphrase length must be 10-79 chars (got {0})")]
    InvalidLength(usize),
    #[error("passphrase contains non-printable ASCII")]
    InvalidCharset,
    #[error("environment variable '{0}' is unset or empty")]
    EnvUnset(String),
    #[error("error reading passphrase file: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Error)]
pub enum StreamIdError {
    #[error("stream ID exceeds 512 chars (got {0})")]
    TooLong(usize),
    #[error("stream ID contains non-ASCII characters")]
    NonAscii,
}

#[derive(Debug, Error)]
pub enum PacketFilterError {
    #[error("filter spec exceeds 512 chars")]
    TooLong,
    #[error("filter spec contains invalid characters")]
    InvalidCharset,
}

#[derive(Debug, Error)]
pub enum AddrError {
    #[error("could not resolve address: {0}")]
    Resolve(String),
    #[error("IPv6 not supported in v0")]
    Ipv6Unsupported,
    #[error("IO error reading address: {0}")]
    Io(#[from] io::Error),
}

// ============================================================================
// Supporting public types
// ============================================================================

/// libsrt's MJ_* major error category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SrtErrno {
    Setup,
    Connection,
    SystemRes,
    FileSystem,
    Notsup,
    Async,
    PeerError,
    Timeout,
    Bad,
    Unknown(i32),
}

/// Listener-side reject codes (SRT_REJ_*) sent during handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RejectReason {
    BadSecret,
    Unsecure,
    ValueLearn,
    UnknownStreamId,
    Resource,
    Rogue,
    Backlog,
    Ipe,
    Close,
    Version,
    RdvCookie,
    BadRequest,
    Forbidden,
    NotFound,
    Other(i32),
}

// ============================================================================
// Cross-cutting error enums
// ============================================================================

#[derive(Debug, Error)]
pub enum OptionError {
    #[error("option not settable in current socket state")]
    InvalidState,
    #[error("option value out of range: {0}")]
    OutOfRange(String),
    #[error("invalid passphrase: {0}")]
    InvalidPassphrase(#[from] PassphraseError),
    #[error("invalid stream id: {0}")]
    InvalidStreamId(#[from] StreamIdError),
    #[error("invalid packet filter: {0}")]
    InvalidPacketFilter(#[from] PacketFilterError),
    #[error("libsrt error: {kind:?} - {message}")]
    Other { kind: SrtErrno, message: String },
}

#[derive(Debug, Error)]
pub enum IoError {
    #[error("socket has been closed")]
    SocketClosed,
    #[error("system error: {0}")]
    System(#[from] io::Error),
    #[error("libsrt error: {kind:?} - {message}")]
    Other { kind: SrtErrno, message: String },
}

// ============================================================================
// Per-call-site error enums
// ============================================================================

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("invalid address: {0}")]
    InvalidAddress(#[from] AddrError),
    #[error("encryption configuration rejected: {detail}")]
    BadEncryption { detail: String },
    #[error("peer rejected connection: {reason:?} - {detail}")]
    Rejected {
        reason: RejectReason,
        detail: String,
    },
    #[error("connection timed out")]
    TimedOut,
    #[error("connection refused")]
    Refused,
    #[error("config validation failed: {0}")]
    InvalidOption(#[from] OptionError),
    #[error("system error: {0}")]
    System(#[from] io::Error),
    #[error("libsrt error: {kind:?} - {message}")]
    Other { kind: SrtErrno, message: String },
}

#[derive(Debug, Error)]
pub enum BindError {
    #[error("invalid address: {0}")]
    InvalidAddress(#[from] AddrError),
    #[error("address already in use")]
    AddressInUse,
    #[error("permission denied")]
    PermissionDenied,
    #[error("config validation failed: {0}")]
    InvalidOption(#[from] OptionError),
    #[error("system error: {0}")]
    System(#[from] io::Error),
    #[error("libsrt error: {kind:?} - {message}")]
    Other { kind: SrtErrno, message: String },
}

#[derive(Debug, Error)]
pub enum AcceptError {
    #[error("accept timed out")]
    TimedOut,
    #[error("listener was closed")]
    ListenerClosed,
    #[error("peer rejected during handshake: {reason:?} - {detail}")]
    PeerRejected {
        reason: RejectReason,
        detail: String,
    },
    #[error("system error: {0}")]
    System(#[from] io::Error),
    #[error("libsrt error: {kind:?} - {message}")]
    Other { kind: SrtErrno, message: String },
}

#[derive(Debug, Error)]
pub enum SendError {
    #[error("send timed out")]
    TimedOut,
    #[error("connection broken")]
    ConnectionBroken,
    #[error("payload exceeds payload_size limit ({actual} > {limit})")]
    PayloadTooLarge { actual: usize, limit: usize },
    #[error("send queue full")]
    QueueFull,
    #[error("system error: {0}")]
    System(#[from] io::Error),
    #[error("libsrt error: {kind:?} - {message}")]
    Other { kind: SrtErrno, message: String },
}

#[derive(Debug, Error)]
pub enum RecvError {
    #[error("recv timed out")]
    TimedOut,
    #[error("connection broken")]
    ConnectionBroken,
    #[error("buffer too small for incoming message ({buf_len} < {message_len})")]
    BufferTooSmall { buf_len: usize, message_len: usize },
    #[error("system error: {0}")]
    System(#[from] io::Error),
    #[error("libsrt error: {kind:?} - {message}")]
    Other { kind: SrtErrno, message: String },
}

// ============================================================================
// Umbrella `Error` + `Result<T>` alias
// ============================================================================

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Connect(#[from] ConnectError),
    #[error(transparent)]
    Bind(#[from] BindError),
    #[error(transparent)]
    Accept(#[from] AcceptError),
    #[error(transparent)]
    Send(#[from] SendError),
    #[error(transparent)]
    Recv(#[from] RecvError),
    #[error(transparent)]
    Option(#[from] OptionError),
    #[error(transparent)]
    Io(#[from] IoError),
}

pub type Result<T> = std::result::Result<T, Error>;

// ============================================================================
// libsrt-error → typed-enum mapping
// ============================================================================

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct RawError {
    pub kind: SrtErrno,
    pub message: String,
}

/// Read libsrt's last-error state. Call immediately after a libsrt FFI call
/// returned an error indicator.
#[allow(dead_code)]
pub(crate) fn last_error() -> RawError {
    let kind = unsafe { srt_sys::srt_getlasterror(std::ptr::null_mut()) };
    let msg_ptr = unsafe { srt_sys::srt_getlasterror_str() };
    let message = if msg_ptr.is_null() {
        "<no error string>".to_string()
    } else {
        unsafe { std::ffi::CStr::from_ptr(msg_ptr) }
            .to_string_lossy()
            .into_owned()
    };
    RawError {
        kind: SrtErrno::from_raw(kind),
        message,
    }
}

/// Read the typed reject code (only meaningful after a connection-rejected error).
#[allow(dead_code)]
pub(crate) fn last_reject() -> RejectReason {
    let raw = unsafe { srt_sys::srt_getrejectreason(0) };
    RejectReason::from_raw(raw)
}

impl SrtErrno {
    /// Map the raw `srt_getlasterror` int to a typed variant.
    /// libsrt encodes `MJ_*` in upper bits; we drop sub-codes here.
    pub(crate) fn from_raw(raw: i32) -> Self {
        // libsrt's MJ_* values (see vendor/srt/srtcore/srt.h SRT_ERRNO enum).
        // Major category is encoded as (major * 1000 + minor) in the SRT_ERRNO enum.
        let major = raw / 1000;
        match major {
            1 => SrtErrno::Setup,
            2 => SrtErrno::Connection,
            3 => SrtErrno::SystemRes,
            4 => SrtErrno::FileSystem,
            5 => SrtErrno::Notsup,
            6 => SrtErrno::Async,
            7 => SrtErrno::PeerError,
            _ => SrtErrno::Unknown(raw),
        }
    }
}

impl RejectReason {
    /// Map the raw reject reason code to a typed variant.
    /// See vendor/srt/srtcore/access_control.h and srt.h SRT_REJECT_REASON.
    pub(crate) fn from_raw(raw: i32) -> Self {
        match raw {
            1001 => RejectReason::BadSecret,
            1002 => RejectReason::Unsecure,
            1003 => RejectReason::ValueLearn,
            1004 => RejectReason::UnknownStreamId,
            1005 => RejectReason::Resource,
            1006 => RejectReason::Rogue,
            1007 => RejectReason::Backlog,
            1008 => RejectReason::Ipe,
            1009 => RejectReason::Close,
            1010 => RejectReason::Version,
            1011 => RejectReason::RdvCookie,
            1012 => RejectReason::BadRequest,
            1013 => RejectReason::Forbidden,
            1014 => RejectReason::NotFound,
            other => RejectReason::Other(other),
        }
    }
}

/// Decide whether a `RawError` indicates a timeout.
#[allow(dead_code)]
pub(crate) fn is_timeout(raw: &RawError) -> bool {
    matches!(raw.kind, SrtErrno::Async)
        && (raw.message.contains("Timeout")
            || raw.message.contains("timeout")
            || raw.message.contains("timed out"))
}

/// Decide whether a `RawError` indicates the connection has been broken.
#[allow(dead_code)]
pub(crate) fn is_broken(raw: &RawError) -> bool {
    matches!(raw.kind, SrtErrno::Connection)
        && (raw.message.contains("broken") || raw.message.contains("Broken"))
}

// ============================================================================
// From<RawError> impls — translate raw libsrt errors into typed variants.
// Each per-call enum has an `Other { kind, message }` catch-all populated here.
// Specific cases (timeout, broken, refused, rejected, address-in-use,
// permission-denied) are detected by classifier helpers and string matching.
// ============================================================================

impl From<RawError> for ConnectError {
    fn from(raw: RawError) -> Self {
        if matches!(raw.kind, SrtErrno::Connection) {
            // Could be a typed reject. Check.
            let reason = last_reject();
            // RejectReason::Other(0) is the libsrt sentinel for "no reject info".
            if reason != RejectReason::Other(0) {
                return ConnectError::Rejected {
                    reason,
                    detail: raw.message,
                };
            }
            if raw.message.contains("refused") || raw.message.contains("Refused") {
                return ConnectError::Refused;
            }
        }
        if is_timeout(&raw) {
            return ConnectError::TimedOut;
        }
        ConnectError::Other {
            kind: raw.kind,
            message: raw.message,
        }
    }
}

impl From<RawError> for BindError {
    fn from(raw: RawError) -> Self {
        if raw.message.contains("in use") || raw.message.contains("already") {
            return BindError::AddressInUse;
        }
        if raw.message.contains("permission") || raw.message.contains("Permission") {
            return BindError::PermissionDenied;
        }
        BindError::Other {
            kind: raw.kind,
            message: raw.message,
        }
    }
}

impl From<RawError> for AcceptError {
    fn from(raw: RawError) -> Self {
        if is_timeout(&raw) {
            return AcceptError::TimedOut;
        }
        if matches!(raw.kind, SrtErrno::Setup) || raw.message.contains("closed") {
            return AcceptError::ListenerClosed;
        }
        AcceptError::Other {
            kind: raw.kind,
            message: raw.message,
        }
    }
}

impl From<RawError> for SendError {
    fn from(raw: RawError) -> Self {
        if is_timeout(&raw) {
            return SendError::TimedOut;
        }
        if is_broken(&raw) {
            return SendError::ConnectionBroken;
        }
        SendError::Other {
            kind: raw.kind,
            message: raw.message,
        }
    }
}

impl From<RawError> for RecvError {
    fn from(raw: RawError) -> Self {
        if is_timeout(&raw) {
            return RecvError::TimedOut;
        }
        if is_broken(&raw) {
            return RecvError::ConnectionBroken;
        }
        RecvError::Other {
            kind: raw.kind,
            message: raw.message,
        }
    }
}

impl From<RawError> for OptionError {
    fn from(raw: RawError) -> Self {
        OptionError::Other {
            kind: raw.kind,
            message: raw.message,
        }
    }
}

impl From<RawError> for IoError {
    fn from(raw: RawError) -> Self {
        IoError::Other {
            kind: raw.kind,
            message: raw.message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_error_displays() {
        let e = PassphraseError::InvalidLength(5);
        assert_eq!(
            e.to_string(),
            "passphrase length must be 10-79 chars (got 5)"
        );
    }

    #[test]
    fn umbrella_from_per_call() {
        let e: Error = ConnectError::TimedOut.into();
        match e {
            Error::Connect(ConnectError::TimedOut) => {}
            _ => panic!("expected Connect(TimedOut)"),
        }
    }

    #[test]
    fn option_error_from_validation() {
        let e: OptionError = StreamIdError::TooLong(600).into();
        match e {
            OptionError::InvalidStreamId(StreamIdError::TooLong(600)) => {}
            _ => panic!("expected InvalidStreamId(TooLong(600))"),
        }
    }

    #[test]
    fn srt_errno_mapping() {
        assert_eq!(SrtErrno::from_raw(1000), SrtErrno::Setup);
        assert_eq!(SrtErrno::from_raw(1003), SrtErrno::Setup); // sub-codes folded into major
        assert_eq!(SrtErrno::from_raw(2000), SrtErrno::Connection);
        assert_eq!(SrtErrno::from_raw(3001), SrtErrno::SystemRes);
        assert_eq!(SrtErrno::from_raw(4000), SrtErrno::FileSystem);
        assert_eq!(SrtErrno::from_raw(5000), SrtErrno::Notsup);
        assert_eq!(SrtErrno::from_raw(6000), SrtErrno::Async);
        assert_eq!(SrtErrno::from_raw(7000), SrtErrno::PeerError);
        assert_eq!(SrtErrno::from_raw(99999), SrtErrno::Unknown(99999));
    }

    #[test]
    fn reject_reason_mapping() {
        assert_eq!(RejectReason::from_raw(1001), RejectReason::BadSecret);
        assert_eq!(RejectReason::from_raw(1002), RejectReason::Unsecure);
        assert_eq!(RejectReason::from_raw(1010), RejectReason::Version);
        assert_eq!(RejectReason::from_raw(1014), RejectReason::NotFound);
        assert_eq!(RejectReason::from_raw(9999), RejectReason::Other(9999));
    }

    #[test]
    fn timeout_classifier() {
        let r = RawError {
            kind: SrtErrno::Async,
            message: "Operation timed out".into(),
        };
        assert!(is_timeout(&r));
    }

    #[test]
    fn broken_classifier() {
        let r = RawError {
            kind: SrtErrno::Connection,
            message: "Connection broken".into(),
        };
        assert!(is_broken(&r));
    }
}
