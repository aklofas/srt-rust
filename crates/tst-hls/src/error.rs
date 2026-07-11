//! Error types for the `hls` module.

use std::io;

use thiserror::Error;

/// All errors returned by the HLS publisher.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HlsError {
    #[cfg(feature = "serve")]
    #[error("URL parse failed: {0}")]
    Url(#[from] crate::url::HlsUrlError),

    #[error("filesystem I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("HTTP server bind failed: {0}")]
    BindFailed(String),

    #[error("HLS configuration invalid: {0}")]
    InvalidConfig(String),

    #[error("payload must be a multiple of 188 bytes; got {len}")]
    UnalignedPushTs { len: usize },

    #[error("publisher already finished")]
    Finished,

    #[error("TLS feature disabled but `hls+tls` requested")]
    TlsDisabled,

    #[error("TLS setup error: {0}")]
    #[cfg(feature = "tls")]
    Tls(String),

    #[error("internal HTTP server error: {0}")]
    Internal(String),
}

/// Flat error-variant projection for C-ABI mapping (future A5/W3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u32)]
pub enum HlsErrorKind {
    Url = 1,
    Io = 2,
    BindFailed = 3,
    InvalidConfig = 4,
    UnalignedPushTs = 5,
    Finished = 6,
    TlsDisabled = 7,
    Tls = 8,
    Internal = 9,
}

impl HlsError {
    pub fn kind(&self) -> HlsErrorKind {
        match self {
            #[cfg(feature = "serve")]
            Self::Url(_) => HlsErrorKind::Url,
            Self::Io(_) => HlsErrorKind::Io,
            Self::BindFailed(_) => HlsErrorKind::BindFailed,
            Self::InvalidConfig(_) => HlsErrorKind::InvalidConfig,
            Self::UnalignedPushTs { .. } => HlsErrorKind::UnalignedPushTs,
            Self::Finished => HlsErrorKind::Finished,
            Self::TlsDisabled => HlsErrorKind::TlsDisabled,
            #[cfg(feature = "tls")]
            Self::Tls(_) => HlsErrorKind::Tls,
            Self::Internal(_) => HlsErrorKind::Internal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_stable() {
        assert_eq!(HlsErrorKind::Url as u32, 1);
        assert_eq!(HlsErrorKind::Internal as u32, 9);
    }

    #[test]
    fn unaligned_push_carries_len() {
        let e = HlsError::UnalignedPushTs { len: 187 };
        assert_eq!(e.kind(), HlsErrorKind::UnalignedPushTs);
        assert!(e.to_string().contains("187"));
    }
}
