//! TLS support for the HLS HTTP server (tokio-rustls 0.26; feature `tls`).

use std::path::Path;
use std::sync::Arc;

use rustls::ServerConfig;

use crate::error::HlsError;

/// Load a server certificate + key from PEM files.
pub(crate) fn load_server_config(cert: &Path, key: &Path) -> Result<Arc<ServerConfig>, HlsError> {
    let certs = {
        let data = std::fs::read(cert).map_err(HlsError::Io)?;
        let mut reader = std::io::BufReader::new(&data[..]);
        rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(HlsError::Io)?
    };
    let key = {
        let data = std::fs::read(key).map_err(HlsError::Io)?;
        let mut reader = std::io::BufReader::new(&data[..]);
        rustls_pemfile::private_key(&mut reader)
            .map_err(HlsError::Io)?
            .ok_or_else(|| HlsError::Tls(format!("no private key in {}", key.display())))?
    };
    let cfg = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| HlsError::Tls(format!("ServerConfig build: {e}")))?;
    Ok(Arc::new(cfg))
}
