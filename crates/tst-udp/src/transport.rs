//! [`UdpTransport`] — UDP sender implementing `tst_core::transport::Transport`.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tst_core::net::udp_socket::{apply_multicast_send_knobs, bind_udp_socket};
use tst_core::transport::{SocketStats, Transport, TransportError};

use crate::config::SocketConfig;
use crate::error::UdpError;
use crate::stats::UdpStats;
use crate::url::UdpUrl;

/// UDP sender.
///
/// Construct via [`UdpTransport::connect`] for the URL fast-path, or via
/// [`crate::builder::UdpTransportBuilder`] (added in a later phase) for full
/// control over knobs.
///
/// Always represents a single peer (set via `connect()` on the underlying
/// std socket). To send to a different peer, build a new transport.
pub struct UdpTransport {
    socket: UdpSocket,
    pkt_size: usize,
    peer: SocketAddr,
    stats: UdpStats,
    alive: Arc<AtomicBool>,
}

impl UdpTransport {
    /// Build a `UdpTransport` from a `udp://...` URL.
    ///
    /// For multicast destinations, applies TTL + iface knobs from the URL.
    pub fn connect(url: &str) -> Result<Self, UdpError> {
        let url = UdpUrl::parse(url)?;
        let mut cfg = SocketConfig::default();
        cfg.merge_from_url(&url);
        Self::with_config(&url, &cfg)
    }

    /// Build a `UdpTransport` from an already-parsed `UdpUrl` + config.
    pub fn with_config(url: &UdpUrl, cfg: &SocketConfig) -> Result<Self, UdpError> {
        if url.recv_bind {
            return Err(UdpError::InvalidConfig(
                "URL has '@' prefix indicating recv-bind; use UdpRecvTransport".into(),
            ));
        }

        let local: SocketAddr = match (cfg.localaddr, url.addr) {
            (Some(a), _) => SocketAddr::new(a, 0),
            (None, IpAddr::V4(_)) => {
                SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 0)
            }
            (None, IpAddr::V6(_)) => {
                SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED), 0)
            }
        };
        let socket = bind_udp_socket(local).map_err(UdpError::Io)?;

        if url.is_multicast() {
            apply_multicast_send_knobs(&socket, url.addr, cfg.ttl, cfg.iface.as_deref())
                .map_err(UdpError::Io)?;
        }

        apply_socket2_knobs(&socket, cfg).map_err(UdpError::Io)?;

        let peer = SocketAddr::new(url.addr, url.port);
        socket.connect(peer).map_err(UdpError::Io)?;

        Ok(Self {
            socket,
            pkt_size: cfg.pkt_size_or_default(),
            peer,
            stats: UdpStats::default(),
            alive: Arc::new(AtomicBool::new(true)),
        })
    }

    /// Snapshot of UDP stats.
    pub fn stats(&self) -> UdpStats {
        self.stats
    }

    /// Peer address this transport is connected to.
    pub fn peer(&self) -> SocketAddr {
        self.peer
    }
}

fn apply_socket2_knobs(socket: &UdpSocket, cfg: &SocketConfig) -> std::io::Result<()> {
    let s = socket2::SockRef::from(socket);
    if let Some(rcv) = cfg.rcvbuf {
        s.set_recv_buffer_size(rcv)?;
    }
    if let Some(snd) = cfg.sndbuf {
        s.set_send_buffer_size(snd)?;
    }
    if let Some(tos) = cfg.tos {
        // socket2 0.5 set_tos for IPv4. IPv6 traffic-class would need IPV6_TCLASS
        // via libc directly; deferred (low-priority knob).
        let _ = s.set_tos(tos as u32);
    }
    Ok(())
}

impl Transport for UdpTransport {
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
        match self.socket.send(msg) {
            Ok(_n) => {
                self.stats.datagrams_sent = self.stats.datagrams_sent.saturating_add(1);
                self.stats.bytes_sent = self.stats.bytes_sent.saturating_add(msg.len() as u64);
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                Err(TransportError::Backpressure {
                    msg: format!("send WouldBlock: {e}"),
                    errno_code: e.raw_os_error(),
                })
            }
            Err(e) => {
                self.stats.send_errors = self.stats.send_errors.saturating_add(1);
                Err(TransportError::Broken {
                    msg: format!("send error: {e}"),
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
    }

    fn socket_stats(&self) -> Option<SocketStats> {
        Some(self.stats.to_socket_stats())
    }
}
