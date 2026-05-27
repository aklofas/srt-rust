//! [`TcpTransport`] — TCP transport implementing both Transport + RecvTransport.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tst_core::transport::{RecvTransport, SocketStats, Transport, TransportError};

use crate::config::SocketConfig;
use crate::error::TcpError;
use crate::recv_knobs::apply_knobs;
use crate::stats::TcpStats;
use crate::url::TcpUrl;

/// Inner stream — Plain for `tcp://`, Tls for `tcps://` (Phase 8 wires Tls).
pub(crate) enum InnerStream {
    Plain(TcpStream),
    #[cfg(feature = "tls")]
    Tls(crate::tls::TlsStream),
}

impl InnerStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(s) => s.read(buf),
            #[cfg(feature = "tls")]
            Self::Tls(s) => s.read(buf),
        }
    }
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        match self {
            Self::Plain(s) => s.write_all(buf),
            #[cfg(feature = "tls")]
            Self::Tls(s) => s.write_all(buf),
        }
    }
    fn shutdown(&mut self) {
        if let Self::Plain(s) = self {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        // TLS shutdown handled in TlsStream::Drop (or noop in Phase 6 stub).
    }
}

/// TCP transport. Implements both Transport (sender) and RecvTransport (receiver).
///
/// Build via [`TcpTransport::connect`] (caller) or via
/// [`crate::listener::TcpListener::accept_blocking`] (server-side).
pub struct TcpTransport {
    pub(crate) inner: InnerStream,
    pub(crate) pkt_size: usize,
    pub(crate) peer: SocketAddr,
    pub(crate) stats: TcpStats,
    pub(crate) alive: Arc<AtomicBool>,
}

impl TcpTransport {
    /// Build a caller-side `TcpTransport` from a URL (TLS automatically
    /// applied for `tcps://`).
    pub fn connect(url: &str) -> Result<Self, TcpError> {
        let url = TcpUrl::parse(url)?;
        if url.listen {
            return Err(TcpError::InvalidConfig(
                "URL has ?listen=1 — use TcpListener::bind".into(),
            ));
        }
        let mut cfg = SocketConfig::default();
        cfg.merge_from_url(&url);
        Self::connect_with_config(&url, &cfg)
    }

    /// Build a caller-side `TcpTransport` from an already-parsed URL + config.
    pub fn connect_with_config(url: &TcpUrl, cfg: &SocketConfig) -> Result<Self, TcpError> {
        if url.tls {
            #[cfg(feature = "tls")]
            {
                return crate::tls::connect_tls(url, cfg);
            }
            #[cfg(not(feature = "tls"))]
            {
                return Err(TcpError::TlsDisabled);
            }
        }

        let peer = SocketAddr::new(url.addr, url.port);
        let socket = TcpStream::connect_timeout(&peer, cfg.connect_timeout_or_default())
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::TimedOut {
                    TcpError::ConnectTimeout {
                        seconds: cfg.connect_timeout_or_default().as_secs(),
                    }
                } else {
                    TcpError::Io(e)
                }
            })?;
        apply_knobs(&socket, cfg).map_err(TcpError::Io)?;

        Ok(Self {
            inner: InnerStream::Plain(socket),
            pkt_size: cfg.pkt_size_or_default(),
            peer,
            stats: TcpStats::default(),
            alive: Arc::new(AtomicBool::new(true)),
        })
    }

    /// Build from an accepted plain socket (called by TcpListener::accept_blocking).
    pub(crate) fn from_accepted_plain(
        socket: TcpStream,
        peer: SocketAddr,
        cfg: &SocketConfig,
    ) -> Result<Self, TcpError> {
        apply_knobs(&socket, cfg).map_err(TcpError::Io)?;
        Ok(Self {
            inner: InnerStream::Plain(socket),
            pkt_size: cfg.pkt_size_or_default(),
            peer,
            stats: TcpStats::default(),
            alive: Arc::new(AtomicBool::new(true)),
        })
    }

    /// Peer address.
    pub fn peer(&self) -> SocketAddr {
        self.peer
    }

    /// Snapshot stats.
    pub fn stats(&self) -> TcpStats {
        self.stats
    }
}

impl Transport for TcpTransport {
    fn send_bytes(&mut self, msg: &[u8]) -> Result<(), TransportError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(TransportError::Closed);
        }
        if msg.len() > self.pkt_size {
            self.stats.send_errors = self.stats.send_errors.saturating_add(1);
            return Err(TransportError::TooLarge {
                len: msg.len(),
                max: self.pkt_size,
            });
        }
        match self.inner.write_all(msg) {
            Ok(()) => {
                self.stats.send_calls = self.stats.send_calls.saturating_add(1);
                self.stats.bytes_sent = self.stats.bytes_sent.saturating_add(msg.len() as u64);
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                Err(TransportError::Backpressure {
                    msg: format!("write WouldBlock: {e}"),
                    errno_code: e.raw_os_error(),
                })
            }
            Err(e) => {
                self.stats.send_errors = self.stats.send_errors.saturating_add(1);
                Err(TransportError::Broken {
                    msg: format!("write error: {e}"),
                    errno_code: e.raw_os_error(),
                })
            }
        }
    }

    fn max_payload(&self) -> usize {
        self.pkt_size
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn close(&mut self) {
        self.alive.store(false, Ordering::Release);
        self.inner.shutdown();
    }

    fn socket_stats(&self) -> Option<SocketStats> {
        Some(self.stats.to_socket_stats())
    }
}

impl RecvTransport for TcpTransport {
    fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        loop {
            if !self.alive.load(Ordering::Acquire) {
                return Err(TransportError::Closed);
            }
            match self.inner.read(buf) {
                Ok(0) => {
                    return Err(TransportError::Broken {
                        msg: "peer closed connection".into(),
                        errno_code: None,
                    });
                }
                Ok(n) => {
                    self.stats.recv_calls = self.stats.recv_calls.saturating_add(1);
                    self.stats.bytes_received = self.stats.bytes_received.saturating_add(n as u64);
                    return Ok(n);
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(e) => {
                    self.stats.recv_errors = self.stats.recv_errors.saturating_add(1);
                    return Err(TransportError::Broken {
                        msg: format!("read error: {e}"),
                        errno_code: e.raw_os_error(),
                    });
                }
            }
        }
    }

    fn max_payload(&self) -> usize {
        self.pkt_size
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn close(&mut self) {
        self.alive.store(false, Ordering::Release);
        self.inner.shutdown();
    }

    fn socket_stats(&self) -> Option<SocketStats> {
        Some(self.stats.to_socket_stats())
    }
}
