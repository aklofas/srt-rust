//! Error types for the `tst-tcp` crate.

use std::io;

use thiserror::Error;

/// All errors that can be returned from `tst-tcp` operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TcpError {
    #[error("URL parse failed: {0}")]
    Url(#[from] crate::url::TcpUrlError),

    #[error("TCP I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("payload {len} exceeds max {max} bytes per send call")]
    PayloadTooLarge { len: usize, max: usize },

    #[error("transport closed by caller")]
    Closed,

    #[error("connection timed out after {seconds}s")]
    ConnectTimeout { seconds: u64 },

    #[error("invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("TLS error: {0}")]
    #[cfg(feature = "tls")]
    Tls(String),

    #[error("TLS feature disabled but URL uses 'tcps://'")]
    TlsDisabled,
}

/// Flat error-variant projection for C-ABI mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u32)]
pub enum TcpErrorKind {
    Url = 1,
    Io = 2,
    PayloadTooLarge = 3,
    Closed = 4,
    ConnectTimeout = 5,
    InvalidConfig = 6,
    Tls = 7,
    TlsDisabled = 8,
}

impl TcpError {
    pub fn kind(&self) -> TcpErrorKind {
        match self {
            Self::Url(_) => TcpErrorKind::Url,
            Self::Io(_) => TcpErrorKind::Io,
            Self::PayloadTooLarge { .. } => TcpErrorKind::PayloadTooLarge,
            Self::Closed => TcpErrorKind::Closed,
            Self::ConnectTimeout { .. } => TcpErrorKind::ConnectTimeout,
            Self::InvalidConfig(_) => TcpErrorKind::InvalidConfig,
            #[cfg(feature = "tls")]
            Self::Tls(_) => TcpErrorKind::Tls,
            Self::TlsDisabled => TcpErrorKind::TlsDisabled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_stable() {
        assert_eq!(TcpErrorKind::Url as u32, 1);
        assert_eq!(TcpErrorKind::TlsDisabled as u32, 8);
    }

    #[test]
    fn io_error_wraps() {
        let io_err = io::Error::new(io::ErrorKind::ConnectionRefused, "no");
        let t: TcpError = io_err.into();
        assert_eq!(t.kind(), TcpErrorKind::Io);
        assert!(t.to_string().contains("TCP I/O error"));
    }
}
