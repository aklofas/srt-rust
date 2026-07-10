//! [`TcpListener`] — sync TCP listener that accepts new `TcpTransport` connections.

use std::io;
use std::net::TcpListener as StdTcpListener;
use std::net::{IpAddr, SocketAddr};
#[cfg(feature = "tls")]
use std::sync::Arc;

use crate::config::SocketConfig;
use crate::error::TcpError;
use crate::transport::TcpTransport;

/// Sync TCP listener.
///
/// Construct via [`TcpListener::bind`], then call [`TcpListener::accept_blocking`] to
/// receive a fresh [`TcpTransport`] per inbound connection.
///
/// Unlike the connectionless transports (UDP, RTP, RIST), which expose a single
/// `listen(url)` one-shot factory, TCP is connection-oriented: one `TcpListener`
/// instance serves multiple peers, each accepted call returning its own
/// `TcpTransport`. `TcpListener::from_url` is the URL-style alternative to
/// `TcpListener::bind`; pass a `tcp://host:port?listen=1` URL to construct the
/// listener without a raw `SocketAddr`. See
/// See the receive-side entry-points table in `docs/reference/compatibility.md`
/// for a side-by-side comparison of all transport receive-entry patterns.
pub struct TcpListener {
    inner: StdTcpListener,
    config: SocketConfig,
    #[cfg(feature = "tls")]
    tls_config: Option<Arc<rustls::ServerConfig>>,
}

impl TcpListener {
    /// Bind a TCP listener on `addr`. Returns a listener ready for `accept_blocking`.
    pub fn bind(addr: SocketAddr) -> Result<Self, TcpError> {
        let inner = StdTcpListener::bind(addr).map_err(TcpError::Io)?;
        Ok(Self {
            inner,
            config: SocketConfig::default(),
            #[cfg(feature = "tls")]
            tls_config: None,
        })
    }

    /// Bind from a `tcp://0.0.0.0:port?listen=1` URL.
    pub fn from_url(url: &str) -> Result<Self, TcpError> {
        let parsed = crate::url::TcpUrl::parse(url)?;
        if !parsed.listen {
            return Err(TcpError::InvalidConfig(
                "URL does not have ?listen=1 — use TcpTransport::connect for caller-side".into(),
            ));
        }

        let ip: IpAddr = parsed.host.parse().map_err(|_| {
            TcpError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("listener host '{}' must be an IP literal", parsed.host),
            ))
        })?;
        let bind_addr = SocketAddr::new(ip, parsed.port);
        let mut listener = Self::bind(bind_addr)?;
        listener.config.merge_from_url(&parsed);

        #[cfg(feature = "tls")]
        if parsed.tls {
            let cert = parsed.cert.as_deref().ok_or_else(|| {
                TcpError::InvalidConfig("tcps:// listener requires ?cert=path".into())
            })?;
            let key = parsed.key.as_deref().ok_or_else(|| {
                TcpError::InvalidConfig("tcps:// listener requires ?key=path".into())
            })?;
            let server_cfg = crate::tls::load_server_config(cert, key)?;
            listener.tls_config = Some(Arc::new(server_cfg));
        }
        #[cfg(not(feature = "tls"))]
        if parsed.tls {
            return Err(TcpError::TlsDisabled);
        }

        Ok(listener)
    }

    /// Local address the listener bound to.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    /// Set the SocketConfig that will be applied to accepted connections.
    pub fn config_mut(&mut self) -> &mut SocketConfig {
        &mut self.config
    }

    /// Block until a connection arrives. Returns a fully-configured TcpTransport.
    pub fn accept_blocking(&self) -> Result<TcpTransport, TcpError> {
        let (sock, peer) = self.inner.accept().map_err(TcpError::Io)?;

        #[cfg(feature = "tls")]
        if let Some(tls_cfg) = &self.tls_config {
            return crate::tls::accept_tls(sock, peer, &self.config, tls_cfg.clone());
        }

        TcpTransport::from_accepted_plain(sock, peer, &self.config)
    }
}
