//! Error types for the `tst-udp` crate.

use std::io;

use thiserror::Error;

/// All errors that can be returned from `tst-udp` operations.
///
/// The `kind()` accessor projects to [`UdpErrorKind`] for C-ABI mapping in
/// downstream binding crates.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum UdpError {
    #[error("URL parse failed: {0}")]
    Url(#[from] crate::url::UdpUrlError),

    #[error("host '{host}' must be a literal IPv4/IPv6 address: {detail}")]
    HostNotLiteral { host: String, detail: String },

    #[error("UDP socket I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("multicast iface '{iface}' unsupported: {detail}")]
    IfaceUnsupported { iface: String, detail: String },

    #[error("payload {len} exceeds max {max} bytes per datagram")]
    PayloadTooLarge { len: usize, max: usize },

    #[error("transport closed by caller")]
    Closed,

    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
}

/// Flat error-variant projection. Used by future C ABI / Python bindings
/// (A5 bindings batch) to map errors into a stable numeric code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u32)]
pub enum UdpErrorKind {
    Url = 1,
    HostNotLiteral = 2,
    Io = 3,
    IfaceUnsupported = 4,
    PayloadTooLarge = 5,
    Closed = 6,
    InvalidConfig = 7,
}

impl UdpError {
    /// Project to a flat variant for C-ABI mapping. Stable numeric codes.
    pub fn kind(&self) -> UdpErrorKind {
        match self {
            Self::Url(_) => UdpErrorKind::Url,
            Self::HostNotLiteral { .. } => UdpErrorKind::HostNotLiteral,
            Self::Io(_) => UdpErrorKind::Io,
            Self::IfaceUnsupported { .. } => UdpErrorKind::IfaceUnsupported,
            Self::PayloadTooLarge { .. } => UdpErrorKind::PayloadTooLarge,
            Self::Closed => UdpErrorKind::Closed,
            Self::InvalidConfig(_) => UdpErrorKind::InvalidConfig,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_stable() {
        // If anyone reorders the enum, this catches the numeric drift before
        // the bindings ratchet flags it.
        assert_eq!(UdpErrorKind::Url as u32, 1);
        assert_eq!(UdpErrorKind::Closed as u32, 6);
    }

    #[test]
    fn io_error_wraps() {
        let io_err = io::Error::new(io::ErrorKind::TimedOut, "tick");
        let u: UdpError = io_err.into();
        assert_eq!(u.kind(), UdpErrorKind::Io);
        assert!(u.to_string().contains("UDP socket I/O error"));
    }
}
