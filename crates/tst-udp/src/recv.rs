//! [`UdpRecvTransport`] — UDP receiver implementing `tst_core::transport::RecvTransport`.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tst_core::net::udp_socket::{
    CANCEL_POLL_INTERVAL, apply_multicast_recv_join, bind_udp_socket,
    bind_udp_socket_multicast,
};
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

        // Multicast receivers must use SO_REUSEADDR (+ SO_REUSEPORT on BSD/macOS)
        // so that more than one receiver process can bind the same group:port on
        // the same host. Unicast receivers use the plain bind to avoid sharing
        // ports unintentionally.
        let socket = if url.is_multicast() {
            bind_udp_socket_multicast(bind_addr).map_err(UdpError::Io)?
        } else {
            bind_udp_socket(bind_addr).map_err(UdpError::Io)?
        };

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
    /// the duration of this call, then restoring it to the cancel-poll
    /// interval so that subsequent `recv_bytes` calls continue to wake
    /// periodically and re-check the `alive` flag. Not concurrency-safe —
    /// callers must ensure no concurrent `recv_bytes` is in progress on
    /// the same transport handle (the `Mutex<Option<…>>` in the Python
    /// binding guarantees this).
    pub fn recv_timeout(
        &mut self,
        buf: &mut [u8],
        deadline: std::time::Duration,
    ) -> Result<Option<usize>, crate::error::UdpError> {
        self.socket
            .set_read_timeout(Some(deadline))
            .map_err(crate::error::UdpError::Io)?;
        let result = self.socket.recv(buf);
        // Restore the cancel-poll interval so that a subsequent recv_bytes
        // continues to wake periodically and can observe alive=false set by
        // close().  Restoring None (no timeout) would cause recv_bytes to
        // block forever, making close() unable to interrupt it.
        //
        // A failed restore leaves SO_RCVTIMEO at `deadline` rather than the
        // cancel-poll interval, silently breaking close()'s ability to
        // interrupt a later recv_bytes. Surface it: log, and propagate it
        // when the recv itself didn't already fail (a recv error takes
        // priority since it is the more actionable failure).
        let restore = self.socket.set_read_timeout(Some(CANCEL_POLL_INTERVAL));
        if let Err(e) = &restore {
            tracing::warn!(
                error = %e,
                "failed to restore SO_RCVTIMEO after recv_timeout; \
                 cancel-poll may be disabled for subsequent recv_bytes"
            );
        }
        match result {
            Ok(n) => {
                // The datagram arrived, but the socket is now in a wrong-timeout
                // state — report the restore failure rather than silently
                // returning data on a broken socket.
                restore.map_err(crate::error::UdpError::Io)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Regression test: `recv_timeout` must not permanently disable the
    /// cancel-poll `SO_RCVTIMEO` on the socket.
    ///
    /// `recv_bytes` relies on the socket waking every `CANCEL_POLL_INTERVAL`
    /// (~100 ms) to re-check the `alive` flag — that is how `close()` interrupts
    /// a blocked recv.  Before the fix, `recv_timeout` restored `SO_RCVTIMEO` to
    /// `None` (block forever), so a subsequent `recv_bytes` on a quiet socket
    /// would never wake and `close()` (which only sets `alive=false`) could not
    /// interrupt it.
    #[test]
    fn close_unblocks_recv_bytes_after_recv_timeout() {
        let mut recv = UdpRecvTransport::listen("udp://@127.0.0.1:0").expect("bind recv");

        // Step 1 — call recv_timeout with a short deadline; no sender, so it
        // times out and returns Ok(None).  This is the call that, before the fix,
        // would leave SO_RCVTIMEO=None on the socket.
        let mut buf = vec![0u8; recv.max_payload()];
        let result = recv.recv_timeout(&mut buf, Duration::from_millis(50));
        assert!(
            matches!(result, Ok(None)),
            "expected timeout (Ok(None)), got {result:?}"
        );

        // Step 2 — clone the alive flag directly (we are in the same crate so
        // private fields are accessible inside this #[cfg(test)] module) and
        // spawn a closer thread that waits briefly then signals close().
        let alive = Arc::clone(&recv.alive);
        let _closer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            alive.store(false, Ordering::Release);
        });

        // Step 3 — recv_bytes must return Closed within 2 s.  If the
        // SO_RCVTIMEO was cleared to None, recv_bytes blocks forever and the
        // assert on elapsed fires (or the test suite hangs).
        let start = Instant::now();
        let err = recv.recv_bytes(&mut buf);
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_secs(2),
            "recv_bytes blocked for {elapsed:?}; \
             cancel-poll timeout was likely disabled by recv_timeout"
        );
        assert!(
            matches!(err, Err(TransportError::Closed)),
            "expected Err(TransportError::Closed), got {err:?}"
        );
    }

    /// DA-NET-7 regression: two receivers joining the same multicast group on
    /// the same loopback port must both bind successfully. Before the fix, the
    /// second `UdpRecvTransport::listen` would fail with EADDRINUSE because the
    /// plain `UdpSocket::bind` path did not set SO_REUSEADDR.
    ///
    /// This test only asserts bind-and-join success (not datagram delivery),
    /// because getting multicast loopback delivery reliably across all CI
    /// platforms is a separate concern.
    #[test]
    fn two_multicast_receivers_can_bind_same_group_port() {
        let url = "udp://@239.255.42.1:0";
        // Parse to get the actual port after binding.
        let recv1 = match UdpRecvTransport::listen(url) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("skip: first multicast bind failed ({e}); likely no multicast support");
                return;
            }
        };
        let port = recv1.local_addr().port();
        let url_with_port = format!("udp://@239.255.42.1:{port}");
        let recv2 = match UdpRecvTransport::listen(&url_with_port) {
            Ok(r) => r,
            Err(e) => panic!("second multicast receiver bind failed (EADDRINUSE?): {e}"),
        };
        // Both receivers bound to the same group:port — drop order doesn't matter.
        drop(recv1);
        drop(recv2);
    }
}
