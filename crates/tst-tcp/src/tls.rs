//! TLS support via rustls 0.23 (feature `tls`).
//!
//! Phase 6 stub. Phase 8 fills this with the real rustls 0.23 wrap.

use crate::config::SocketConfig;
use crate::error::TcpError;
use crate::transport::TcpTransport;
use crate::url::TcpUrl;

/// TLS stream stub — Phase 8 fills this in.
pub struct TlsStream;

impl TlsStream {
    pub fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "TLS support landing in Phase 8",
        ))
    }

    pub fn write_all(&mut self, _buf: &[u8]) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "TLS support landing in Phase 8",
        ))
    }
}

pub fn connect_tls(_url: &TcpUrl, _cfg: &SocketConfig) -> Result<TcpTransport, TcpError> {
    Err(TcpError::InvalidConfig(
        "TLS support landing in Phase 8".into(),
    ))
}

#[cfg(feature = "tls")]
pub fn load_server_config(_cert_path: &str, _key_path: &str) -> Result<rustls::ServerConfig, TcpError> {
    Err(TcpError::InvalidConfig(
        "TLS support landing in Phase 8".into(),
    ))
}

#[cfg(feature = "tls")]
pub fn accept_tls(
    _socket: std::net::TcpStream,
    _peer: std::net::SocketAddr,
    _cfg: &SocketConfig,
    _server_config: std::sync::Arc<rustls::ServerConfig>,
) -> Result<TcpTransport, TcpError> {
    Err(TcpError::InvalidConfig(
        "TLS support landing in Phase 8".into(),
    ))
}
