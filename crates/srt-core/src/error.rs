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
    Rejected { reason: RejectReason, detail: String },
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
    PeerRejected { reason: RejectReason, detail: String },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_error_displays() {
        let e = PassphraseError::InvalidLength(5);
        assert_eq!(e.to_string(), "passphrase length must be 10-79 chars (got 5)");
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
}
