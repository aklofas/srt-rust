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
    #[error("IPv6 not supported")]
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
// KLV errors
// ============================================================================

#[derive(Debug, Error)]
pub enum KlvDecodeError {
    #[error("buffer truncated at offset {offset}: needed {needed} bytes, have {have}")]
    Truncated {
        offset: usize,
        needed: usize,
        have: usize,
    },

    #[error("malformed BER length at offset {offset}")]
    MalformedLength { offset: usize },

    #[error("BER length {value} exceeds maximum supported size")]
    LengthOverflow { value: u64 },

    #[error("malformed BER-OID tag at offset {offset}")]
    MalformedTag { offset: usize },

    #[error("unexpected universal label: expected {expected}, got {found}")]
    UnexpectedUniversalLabel {
        expected: crate::klv::UniversalLabel,
        found: crate::klv::UniversalLabel,
    },

    #[error("checksum mismatch: declared {expected:#06x}, computed {found:#06x}")]
    ChecksumMismatch { expected: u16, found: u16 },

    #[error("duplicate tag {tag} at offset {offset}")]
    DuplicateTag { tag: u32, offset: usize },

    #[error("trailing bytes after declared length: {len} extra")]
    TrailingBytes { len: usize },

    #[error("Precision Time Stamp Pack body must be 9 bytes, got {got}")]
    BadTimeStampPackLength { got: usize },

    /// Not produced by `klv::st0605::decode` (which is permissive about
    /// reserved bits per its doc); call `time_status.reserved_bits_valid()`
    /// on the decoded pack and raise this if a stricter caller wants it.
    #[error("Time Status reserved bits 4-0 must be 0b11111, got {got:#04x}")]
    ReservedBitsInvalid { got: u8 },

    #[error("Tag 2 (timestamp) must be the first element per ST 0601.8-09")]
    Tag2NotFirst,

    #[error("Tag 1 (checksum) must be the last element per ST 0601.8-11")]
    Tag1NotLast,

    #[error("Tag 65 (UAS LS Version) is required per ST 0601.8-12")]
    MissingTag65,
}

#[derive(Debug, Error)]
pub enum KlvEncodeError {
    #[error("output buffer too small: needed {needed} bytes, got {got}")]
    BufferTooSmall { needed: usize, got: usize },

    #[error("record exceeds maximum BER-encodable length")]
    RecordTooLarge,

    #[error("value out of range for tag {tag}: {value} not in [{min}, {max}]")]
    OutOfRange {
        tag: u32,
        value: f64,
        min: f64,
        max: f64,
    },

    #[error("string field for tag {tag} exceeds {max} bytes")]
    StringTooLong { tag: u32, max: usize },
}

#[derive(Debug, Clone, Error, PartialEq)]
pub enum KlvFieldError {
    #[error("tag {tag}: value {value} out of declared range [{min}, {max}]")]
    OutOfRange {
        tag: u32,
        value: f64,
        min: f64,
        max: f64,
    },

    #[error("tag {tag}: invalid UTF-8 in string field")]
    InvalidUtf8 { tag: u32 },

    #[error("tag {tag}: expected {expected} value bytes, got {got}")]
    InvalidLength {
        tag: u32,
        expected: usize,
        got: usize,
    },

    #[error("tag {tag}: value reserved as INVALID by spec")]
    InvalidSentinel { tag: u32 },
}

// ============================================================================
// MPEG-TS mux errors
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MuxError {
    #[error("muxer configuration is invalid: {0}")]
    InvalidConfig(&'static str),

    #[error("video input is not Annex-B framed (no start code prefix)")]
    InvalidNal,

    #[error("muxer packet buffer is full ({capacity_packets} packets); drain via pull and retry")]
    BufferFull { capacity_packets: usize },

    /// KLV blob exceeds the 16-bit `PES_packet_length` ceiling.
    ///
    /// PES_packet_length is at most 65535 and must cover flags1, flags2,
    /// header_data_length, the PTS field (if present), and the ES payload —
    /// so the KLV payload itself is bounded to 65532 bytes (no PTS) or
    /// 65527 bytes (with PTS). MISB ST 0601 packs are typically <2 KB so
    /// this is a sanity check, not a regular failure mode.
    #[error("KLV blob is {size} bytes, exceeds PES_packet_length ceiling of {max} bytes")]
    KlvTooLarge { size: usize, max: usize },

    /// Caller passed a `VideoStreamHandle` / `KlvStreamHandle` that doesn't
    /// match a configured stream on this `Muxer`. Handles are obtained from
    /// `Muxer::video_handles()` / `klv_handles()` and are tied to the
    /// muxer that produced them — passing one from a different muxer is
    /// also rejected here.
    #[error("invalid {kind} stream handle (index {index}) — not a configured stream")]
    InvalidStreamHandle {
        kind: &'static str, // "video" or "klv"
        index: usize,
    },

    /// Caller invoked the no-suffix `push_video` / `push_klv` (or the
    /// `Sender::send_video` / `send_klv` wrappers) on a muxer that has more
    /// than one stream of that kind. The single-target API can only resolve
    /// to a single handle when exactly one stream of that kind is configured.
    #[error(
        "ambiguous push: {count} {kind} streams configured — call push_{kind}_to(handle, ...) instead"
    )]
    AmbiguousTarget {
        kind: &'static str, // "video" or "klv"
        count: usize,
    },

    /// `Config::validate` rejects more than 16 video streams.
    /// Trivially lifted if a consumer asks; 16 is well above realistic
    /// gimbaled-platform topologies (EO + IR + maybe IR-narrow + a depth
    /// channel = 4 in the wild today).
    #[error("too many video streams: {count} configured, cap is {cap}")]
    TooManyVideoStreams { count: usize, cap: usize },

    /// `Config::validate` rejects more than 16 KLV streams.
    #[error("too many klv streams: {count} configured, cap is {cap}")]
    TooManyKlvStreams { count: usize, cap: usize },
}

// ============================================================================
// MPEG-TS demux errors
// ============================================================================

/// Errors emitted by `mpegts::demux`.
///
/// Lenient-mode demuxing typically does NOT return errors — non-conformance
/// surfaces as `DemuxEvent::NonConformant { issue }` so the receive loop
/// keeps running. The error variants below fire when something is genuinely
/// fatal (the byte stream is unrecoverable, or strict mode converts a
/// `NonConformantIssue` into a hard failure).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DemuxError {
    /// Byte stream is unrecoverable: too few bytes after a long sync-search
    /// window to make progress, or repeated PSI checksum failures.
    #[error("demuxer cannot recover sync after {after_bytes} bytes")]
    Unrecoverable { after_bytes: usize },

    /// Strict mode rejected a `NonConformantIssue`. Lenient mode would have
    /// emitted a `NonConformant` event instead and continued.
    #[error("strict-mode rejection: {0}")]
    StrictRejection(String),

    /// PSI section claimed a length that doesn't fit a valid PAT/PMT.
    /// Distinct from a checksum mismatch (which is `NonConformant` in
    /// lenient mode); this is structurally impossible.
    #[error("malformed PSI section at PID 0x{pid:04X}: {reason}")]
    MalformedPsi { pid: u16, reason: &'static str },

    /// PES header at PID 0x{pid:04X} declared a length that's too short to
    /// contain its own claimed flags. Unlike PSI checksum failures (which
    /// surface as `NonConformant` in lenient mode), this prevents the
    /// reassembler from making any forward progress.
    #[error("malformed PES header at PID 0x{pid:04X}: {reason}")]
    MalformedPes { pid: u16, reason: &'static str },
}

// ============================================================================
// Pipeline transport errors (re-exported for convenience; defined in
// pipeline::transport because they describe a behavioral contract that's
// part of the pipeline module's public surface)
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
    #[error(transparent)]
    KlvDecode(#[from] KlvDecodeError),
    #[error(transparent)]
    KlvEncode(#[from] KlvEncodeError),
    #[error(transparent)]
    KlvField(#[from] KlvFieldError),
    #[error(transparent)]
    Mux(#[from] MuxError),
    #[error(transparent)]
    Demux(#[from] DemuxError),
    #[error(transparent)]
    Transport(#[from] crate::pipeline::transport::TransportError),
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

    #[test]
    fn klv_decode_displays() {
        let e = KlvDecodeError::ChecksumMismatch {
            expected: 0xDEAD,
            found: 0xBEEF,
        };
        assert_eq!(
            e.to_string(),
            "checksum mismatch: declared 0xdead, computed 0xbeef"
        );
    }

    #[test]
    fn klv_encode_displays() {
        let e = KlvEncodeError::BufferTooSmall {
            needed: 256,
            got: 100,
        };
        assert_eq!(
            e.to_string(),
            "output buffer too small: needed 256 bytes, got 100"
        );
    }

    #[test]
    fn klv_field_clone_and_eq() {
        let a = KlvFieldError::InvalidUtf8 { tag: 50 };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn umbrella_from_klv_decode() {
        let e: Error = KlvDecodeError::Truncated {
            offset: 42,
            needed: 8,
            have: 3,
        }
        .into();
        match e {
            Error::KlvDecode(KlvDecodeError::Truncated { offset: 42, .. }) => {}
            _ => panic!("expected KlvDecode(Truncated{{offset:42, ...}})"),
        }
    }

    #[test]
    fn umbrella_from_klv_field() {
        let e: Error = KlvFieldError::InvalidUtf8 { tag: 50 }.into();
        matches!(e, Error::KlvField(KlvFieldError::InvalidUtf8 { tag: 50 }));
    }

    #[test]
    fn mux_error_displays() {
        let e = MuxError::BufferFull {
            capacity_packets: 10000,
        };
        assert_eq!(
            e.to_string(),
            "muxer packet buffer is full (10000 packets); drain via pull and retry"
        );
    }

    #[test]
    fn mux_invalid_config_displays() {
        let e = MuxError::InvalidConfig("video_pid must differ from klv_pid");
        assert_eq!(
            e.to_string(),
            "muxer configuration is invalid: video_pid must differ from klv_pid"
        );
    }

    #[test]
    fn umbrella_from_mux() {
        let e: Error = MuxError::InvalidNal.into();
        match e {
            Error::Mux(MuxError::InvalidNal) => {}
            _ => panic!("expected Mux(InvalidNal)"),
        }
    }

    #[test]
    fn mux_error_invalid_stream_handle_displays_kind_and_index() {
        let e = MuxError::InvalidStreamHandle {
            kind: "video",
            index: 7,
        };
        assert_eq!(
            e.to_string(),
            "invalid video stream handle (index 7) — not a configured stream",
        );
    }

    #[test]
    fn mux_error_ambiguous_target_displays_kind_and_count() {
        let e = MuxError::AmbiguousTarget { kind: "klv", count: 3 };
        assert_eq!(
            e.to_string(),
            "ambiguous push: 3 klv streams configured — call push_klv_to(handle, ...) instead",
        );
    }

    #[test]
    fn mux_error_too_many_video_streams_reports_cap() {
        let e = MuxError::TooManyVideoStreams { count: 17, cap: 16 };
        assert_eq!(
            e.to_string(),
            "too many video streams: 17 configured, cap is 16",
        );
    }

    #[test]
    fn mux_error_too_many_klv_streams_reports_cap() {
        let e = MuxError::TooManyKlvStreams { count: 20, cap: 16 };
        assert_eq!(
            e.to_string(),
            "too many klv streams: 20 configured, cap is 16",
        );
    }
}
