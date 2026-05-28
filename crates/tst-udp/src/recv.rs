//! [`UdpRecvTransport`] — UDP receiver implementing `tst_core::transport::RecvTransport`.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tst_core::net::udp_socket::{apply_multicast_recv_join, bind_udp_socket};
use tst_core::transport::{RecvTransport, SocketStats, TransportError};

use crate::config::SocketConfig;
use crate::error::UdpError;
use crate::stats::UdpStats;
use crate::transport_recv_knobs;
use crate::url::UdpUrl;

/// UDP receiver.
///
/// Construct via [`UdpRecvTransport::listen`] for the URL fast-path, or via
/// [`crate::builder::UdpRecvTransportBuilder`] (added in a later phase).
pub struct UdpRecvTransport {
    socket: UdpSocket,
    pkt_size: usize,
    local: SocketAddr,
    stats: UdpStats,
    alive: Arc<AtomicBool>,
}

impl UdpRecvTransport {
    /// Build a `UdpRecvTransport` from a `udp://...` URL.
    ///
    /// URL semantics:
    /// - `udp://@bind_addr:port` — bind (the `@` is the ffmpeg recv convention)
    /// - `udp://bind_addr:port` — also accepted; behavior is identical
    /// - For multicast groups, the socket joins the group on bind.
    pub fn listen(url: &str) -> Result<Self, UdpError> {
        let url = UdpUrl::parse(url)?;
        let mut cfg = SocketConfig::default();
        cfg.merge_from_url(&url);
        Self::with_config(&url, &cfg)
    }

    /// Build from already-parsed `UdpUrl` + config.
    pub fn with_config(url: &UdpUrl, cfg: &SocketConfig) -> Result<Self, UdpError> {
        let bind_addr: SocketAddr = if url.is_multicast() {
            match url.addr {
                IpAddr::V4(_) => {
                    SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), url.port)
                }
                IpAddr::V6(_) => {
                    SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), url.port)
                }
            }
        } else {
            SocketAddr::new(url.addr, url.port)
        };

        let socket = bind_udp_socket(bind_addr).map_err(UdpError::Io)?;

        if url.is_multicast() {
            apply_multicast_recv_join(&socket, url.addr, cfg.iface.as_deref())
                .map_err(UdpError::Io)?;
        }

        transport_recv_knobs::apply_recv_knobs(&socket, cfg).map_err(UdpError::Io)?;

        let local = socket.local_addr().map_err(UdpError::Io)?;

        Ok(Self {
            socket,
            pkt_size: cfg.pkt_size_or_default(),
            local,
            stats: UdpStats::default(),
            alive: Arc::new(AtomicBool::new(true)),
        })
    }

    /// Local bound address (useful for tests that bind to port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// Snapshot of UDP stats.
    pub fn stats(&self) -> UdpStats {
        self.stats
    }

    /// Receive one datagram with a deadline. Returns `None` on timeout
    /// (no data arrived within `deadline`). Returns `Some(n)` on success.
    ///
    /// Implemented by setting `SO_RCVTIMEO` on the underlying socket for
    /// the duration of this call, then restoring it to no-timeout. Not
    /// concurrency-safe — callers must ensure no concurrent `recv_bytes`
    /// is in progress on the same transport handle (the `Mutex<Option<…>>`
    /// in the Python binding guarantees this).
    pub fn recv_timeout(
        &mut self,
        buf: &mut [u8],
        deadline: std::time::Duration,
    ) -> Result<Option<usize>, crate::error::UdpError> {
        self.socket
            .set_read_timeout(Some(deadline))
            .map_err(crate::error::UdpError::Io)?;
        let result = self.socket.recv(buf);
        // Restore no-timeout; ignore error (best-effort).
        let _ = self.socket.set_read_timeout(None);
        match result {
            Ok(n) => {
                self.stats.datagrams_received = self.stats.datagrams_received.saturating_add(1);
                self.stats.bytes_received = self.stats.bytes_received.saturating_add(n as u64);
                Ok(Some(n))
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                Ok(None)
            }
            Err(e) => {
                self.stats.recv_errors = self.stats.recv_errors.saturating_add(1);
                Err(crate::error::UdpError::Io(e))
            }
        }
    }
}

impl RecvTransport for UdpRecvTransport {
    fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        loop {
            if !self.alive.load(Ordering::Acquire) {
                return Err(TransportError::Closed);
            }
            match self.socket.recv(buf) {
                Ok(n) => {
                    self.stats.datagrams_received = self.stats.datagrams_received.saturating_add(1);
                    self.stats.bytes_received = self.stats.bytes_received.saturating_add(n as u64);
                    return Ok(n);
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    // Cancel-poll tick — loop to re-check `alive`.
                    continue;
                }
                Err(e) => {
                    self.stats.recv_errors = self.stats.recv_errors.saturating_add(1);
                    return Err(TransportError::Broken {
                        msg: format!("recv error: {e}"),
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
    }

    fn socket_stats(&self) -> Option<SocketStats> {
        Some(self.stats.to_socket_stats())
    }
}
