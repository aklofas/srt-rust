//! `RtpTransport` (send) + `RtpRecvTransport` (recv) — sync UDP socket
//! wrappers behind the [`tst_core::transport`] traits.
//!
//! Phase 1 ships only the UDP data plane; RTSP control plane (Phase 2)
//! is what makes negotiated transports work. For now, sender + receiver
//! agree on a fixed `host:port` and use it directly.

use std::io;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use tst_core::transport::{RecvTransport, SocketStats, Transport, TransportCancel, TransportError};

use crate::cancel::RtpCancelHandle;
use crate::clock::RtpClock;
use crate::packet::{RTP_HEADER_LEN, RtpHeader};
use crate::url::{RtpUrl, UrlError as RtpUrlError};

/// Wakeup interval for cancel-flag checks. Mirrors the libsrt-side 100 ms
/// `SRTO_RCVTIMEO`/`SNDTIMEO` convention.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// RTP send-side transport: writes 12-byte RTP header + TS payload to a
/// connected [`UdpSocket`].
///
/// # `SocketStats` field mapping
///
/// | [`SocketStats`] field | Phase 1 source |
/// |---|---|
/// | `bytes_sent` | Local counter, ticks per successful `send_bytes` |
/// | `packets_sent` | Local counter, ticks per RTP packet |
/// | `bytes_received` / `packets_received` | 0 (this is the send half) |
/// | `rtt_us` | 0 — RTCP RR/SR ingestion is deferred past Phase 1 |
/// | `packets_lost_send` | 0 — same; would come from RTCP RR fraction-lost |
/// | `link_bandwidth_bps` | 0 — RTP has no link estimate |
/// | All other fields | 0 |
///
/// # `TransportError::Broken` `errno_code` mapping
///
/// For send-side `Broken`, `errno_code` carries the OS `errno` from the
/// underlying `sendto` call (`EAGAIN`=11, `EHOSTUNREACH`=113,
/// `ECONNREFUSED`=111 on Linux). `Backpressure` is not produced in
/// Phase 1 — UDP either accepts the datagram or surfaces an error.
pub struct RtpTransport {
    socket: Option<UdpSocket>,
    /// Negotiated max UDP payload (RTP header + TS bundle) — defaults to
    /// 1316 + 12 = 1328 bytes from `RtpUrl::pkt_size`.
    max_payload: usize,
    clock: RtpClock,
    ssrc: u32,
    next_seq: u16,
    cancel: Arc<RtpCancelHandle>,
    /// Local stats — bytes_sent / packets_sent only in Phase 1; the
    /// RTCP-derived fields stay zero per the master spec's SocketStats
    /// table.
    bytes_sent: u64,
    packets_sent: u64,
}

impl RtpTransport {
    /// Connect (just sets `SocketAddr::connect`-style default) and
    /// return a ready-to-send transport.
    ///
    /// `url` must have scheme `rtp://` and an explicit port.
    pub fn connect(url: &str) -> Result<Self, ConnectError> {
        let parsed = RtpUrl::parse(url).map_err(ConnectError::Url)?;
        Self::connect_with(&parsed)
    }

    /// Connect using an already-parsed URL — convenient for callers that
    /// hold an `RtpUrl` (e.g., binding crates).
    pub fn connect_with(url: &RtpUrl) -> Result<Self, ConnectError> {
        let ip: IpAddr = url.host.parse().map_err(|e: std::net::AddrParseError| {
            ConnectError::HostNotLiteral {
                host: url.host.clone(),
                detail: e.to_string(),
            }
        })?;
        let peer = SocketAddr::new(ip, url.port);
        let local: SocketAddr = match ip {
            IpAddr::V4(_) => "0.0.0.0:0".parse().unwrap(),
            IpAddr::V6(_) => "[::]:0".parse().unwrap(),
        };
        let socket = UdpSocket::bind(local).map_err(ConnectError::Io)?;
        socket
            .set_write_timeout(Some(CANCEL_POLL_INTERVAL))
            .map_err(ConnectError::Io)?;
        // Multicast knobs (no-op for unicast).
        let is_multicast = match ip {
            IpAddr::V4(v4) => v4.is_multicast(),
            IpAddr::V6(v6) => v6.is_multicast(),
        };
        if is_multicast {
            apply_multicast_send_knobs(&socket, &ip, url)?;
        }
        socket.connect(peer).map_err(ConnectError::Io)?;
        Ok(Self::from_socket(socket, url))
    }

    /// Internal: build from an already-configured socket.
    fn from_socket(socket: UdpSocket, url: &RtpUrl) -> Self {
        let ssrc = url.ssrc.unwrap_or_else(random_u32);
        let next_seq = random_u32() as u16;
        let start_ticks = random_u32();
        Self {
            socket: Some(socket),
            max_payload: url.pkt_size,
            clock: RtpClock::new(start_ticks),
            ssrc,
            next_seq,
            cancel: RtpCancelHandle::new(),
            bytes_sent: 0,
            packets_sent: 0,
        }
    }
}

/// Failure shape for [`RtpTransport::connect`] / [`RtpTransport::connect_with`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConnectError {
    #[error("URL parse failed: {0}")]
    Url(#[from] RtpUrlError),
    /// `RtpUrl::host` couldn't be parsed as a literal IP. Phase 1
    /// doesn't do DNS resolution — callers can pre-resolve and pass the
    /// literal.
    #[error("host '{host}' is not a literal IPv4/IPv6 address: {detail}")]
    HostNotLiteral { host: String, detail: String },
    /// OS-level socket failure (bind, connect, setsockopt).
    #[error("UDP socket error: {0}")]
    Io(#[from] io::Error),
    /// `?iface=` couldn't be applied — typically because the platform
    /// requires a different form (e.g., IPv6 needs scope-id integer).
    #[error("multicast iface '{iface}' unsupported: {detail}")]
    IfaceUnsupported { iface: String, detail: String },
}

impl Transport for RtpTransport {
    fn send_bytes(&mut self, msg: &[u8]) -> Result<(), TransportError> {
        if msg.len() + RTP_HEADER_LEN > self.max_payload {
            return Err(TransportError::TooLarge {
                len: msg.len() + RTP_HEADER_LEN,
                max: self.max_payload,
            });
        }
        let socket = self.socket.as_ref().ok_or(TransportError::Closed)?;
        // Build datagram: RTP header (12 B) + TS payload.
        let mut datagram = Vec::with_capacity(RTP_HEADER_LEN + msg.len());
        datagram.resize(RTP_HEADER_LEN, 0);
        RtpHeader::new(self.next_seq, self.clock.now_ticks(), self.ssrc).encode_into(&mut datagram);
        datagram.extend_from_slice(msg);
        loop {
            if self.cancel.is_cancelled() {
                return Err(TransportError::ExplicitClose);
            }
            match socket.send(&datagram) {
                Ok(n) => {
                    self.next_seq = self.next_seq.wrapping_add(1);
                    self.bytes_sent += n as u64;
                    self.packets_sent += 1;
                    return Ok(());
                }
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    // Timeout — re-check cancel and retry.
                    continue;
                }
                Err(e) => {
                    self.socket = None;
                    return Err(TransportError::Broken {
                        msg: format!("UDP send failed: {e}"),
                        errno_code: e.raw_os_error(),
                    });
                }
            }
        }
    }

    fn max_payload(&self) -> usize {
        self.max_payload.saturating_sub(RTP_HEADER_LEN)
    }

    fn is_alive(&self) -> bool {
        self.socket.is_some()
    }

    fn close(&mut self) {
        self.socket = None;
    }

    fn cancel_handle(&self) -> Option<Arc<dyn TransportCancel + Send + Sync>> {
        Some(self.cancel.clone() as Arc<dyn TransportCancel + Send + Sync>)
    }

    fn socket_stats(&self) -> Option<SocketStats> {
        self.socket.as_ref()?;
        // `SocketStats` is `#[non_exhaustive]` in tst-core, so neither
        // a field-init expression nor `..Default::default()` works at a
        // distance — mirror tst-srt's `map_stats` and default-then-assign.
        #[allow(clippy::field_reassign_with_default)]
        let mut s = SocketStats::default();
        s.bytes_sent = self.bytes_sent;
        s.packets_sent = self.packets_sent;
        Some(s)
    }
}

impl Drop for RtpTransport {
    fn drop(&mut self) {
        self.close();
    }
}

/// Helper: 4 random bytes from `getrandom`.
fn random_u32() -> u32 {
    let mut buf = [0u8; 4];
    // `getrandom` Result type cannot fail on a healthy system; if it
    // somehow does, fall back to a process-stable default (0). Logging
    // a tracing event preserves the diagnostic.
    if let Err(e) = getrandom::getrandom(&mut buf) {
        tracing::warn!(error = %e, "getrandom failed; using zero for RTP randomness field");
    }
    u32::from_be_bytes(buf)
}

/// Apply multicast-send socket options (`IP_MULTICAST_TTL` /
/// `IPV6_MULTICAST_HOPS`, optional `IP_MULTICAST_IF`).
///
/// `?ttl=N` from the URL maps to TTL on IPv4 and "hop limit" on IPv6
/// — both `u8`-typed wire knobs that mean the same thing for routing
/// scope. Default 8 for multicast send when the URL doesn't specify.
///
/// IPv4 TTL uses stable `std::net::UdpSocket::set_multicast_ttl_v4`.
/// IPv6 hop limit and IPv4 `IP_MULTICAST_IF` are not exposed on stable
/// std as of Rust 1.85 — we drop to `libc::setsockopt` on Unix and
/// surface a Phase-1 `IfaceUnsupported` on non-Unix platforms when an
/// IPv6 mcast hop or `iface=` knob is requested.
fn apply_multicast_send_knobs(
    socket: &UdpSocket,
    ip: &IpAddr,
    url: &RtpUrl,
) -> Result<(), ConnectError> {
    let ttl = url.ttl.unwrap_or(MCAST_DEFAULT_TTL);
    match ip {
        IpAddr::V4(_) => socket
            .set_multicast_ttl_v4(ttl as u32)
            .map_err(ConnectError::Io)?,
        IpAddr::V6(_) => set_multicast_hops_v6(socket, ttl)?,
    }
    if let Some(iface) = url.iface.as_deref() {
        apply_multicast_iface(socket, ip, iface)?;
    }
    Ok(())
}

/// Default multicast TTL for sends when `?ttl=` is absent — small but
/// non-1 so single-router LAN multicast works out of the box. Matches
/// the master spec's URL defaults table.
const MCAST_DEFAULT_TTL: u8 = 8;

/// Set `IPV6_MULTICAST_HOPS` via raw `setsockopt`. Stable std::net does
/// not expose this in Rust 1.85 (tracking issue rust-lang/rust#92517).
#[cfg(unix)]
fn set_multicast_hops_v6(socket: &UdpSocket, hops: u8) -> Result<(), ConnectError> {
    use std::os::fd::AsRawFd;
    let val: libc::c_int = hops as libc::c_int;
    // SAFETY: `socket.as_raw_fd()` returns an FD owned by `socket` for
    // its lifetime; `&val` is a valid pointer to a c_int sized to
    // `size_of::<c_int>()`. setsockopt with these args is documented in
    // ipv6(7).
    let rc = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::IPPROTO_IPV6,
            libc::IPV6_MULTICAST_HOPS,
            &val as *const libc::c_int as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(ConnectError::Io(io::Error::last_os_error()));
    }
    Ok(())
}

/// Non-Unix fallback: report IPv6 multicast hop-limit knob as unsupported
/// in Phase 1 rather than silently ignoring `?ttl=`.
#[cfg(not(unix))]
fn set_multicast_hops_v6(_socket: &UdpSocket, _hops: u8) -> Result<(), ConnectError> {
    Err(ConnectError::IfaceUnsupported {
        iface: "<ipv6-hops>".to_string(),
        detail: "IPV6_MULTICAST_HOPS via raw setsockopt is Unix-only in Phase 1".to_string(),
    })
}

/// Set `IP_MULTICAST_IF` for IPv4 (interface IP) or surface a Phase-1
/// limitation for IPv6 (needs scope-id integer lookup, not yet wired).
///
/// IPv4 path accepts `?iface=192.168.1.50` (literal IP) directly. Name
/// → IP resolution for unicast nameservers (e.g., `eth0`) is not done
/// in Phase 1 — callers needing name-based binding can resolve via
/// `if_indextoname` and pass the IP. This is the same UX libsrt's
/// `?iface=` query parameter ships with.
fn apply_multicast_iface(socket: &UdpSocket, ip: &IpAddr, iface: &str) -> Result<(), ConnectError> {
    match ip {
        IpAddr::V4(_) => {
            let v4: std::net::Ipv4Addr = iface.parse().map_err(|e: std::net::AddrParseError| {
                ConnectError::IfaceUnsupported {
                    iface: iface.to_string(),
                    detail: format!(
                        "IPv4 multicast iface requires literal IPv4 address, got '{iface}': {e}"
                    ),
                }
            })?;
            set_multicast_if_v4(socket, &v4)?;
        }
        IpAddr::V6(_) => {
            return Err(ConnectError::IfaceUnsupported {
                iface: iface.to_string(),
                detail: "IPv6 multicast iface name lookup not implemented in Phase 1; pre-resolve to scope-id and use the rtp:// URL form directly".to_string(),
            });
        }
    }
    Ok(())
}

/// Set `IP_MULTICAST_IF` via raw `setsockopt`. Stable std::net does not
/// expose this in Rust 1.85 (tracking issue rust-lang/rust#92517).
#[cfg(unix)]
fn set_multicast_if_v4(socket: &UdpSocket, addr: &std::net::Ipv4Addr) -> Result<(), ConnectError> {
    use std::os::fd::AsRawFd;
    // `IP_MULTICAST_IF` accepts a 4-byte in_addr (the IP of the local
    // interface to send out on). Pass the network-byte-order octets
    // directly — `Ipv4Addr::octets()` is already big-endian.
    let in_addr = libc::in_addr {
        s_addr: u32::from_ne_bytes(addr.octets()),
    };
    // SAFETY: `socket.as_raw_fd()` returns an FD owned by `socket` for
    // its lifetime; `&in_addr` is a valid pointer to a struct in_addr
    // sized to `size_of::<in_addr>()`. setsockopt with these args is
    // documented in ip(7).
    let rc = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::IPPROTO_IP,
            libc::IP_MULTICAST_IF,
            &in_addr as *const libc::in_addr as *const libc::c_void,
            std::mem::size_of::<libc::in_addr>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(ConnectError::Io(io::Error::last_os_error()));
    }
    Ok(())
}

/// Non-Unix fallback: surface the iface knob as unsupported.
#[cfg(not(unix))]
fn set_multicast_if_v4(_socket: &UdpSocket, addr: &std::net::Ipv4Addr) -> Result<(), ConnectError> {
    Err(ConnectError::IfaceUnsupported {
        iface: addr.to_string(),
        detail: "IP_MULTICAST_IF via raw setsockopt is Unix-only in Phase 1".to_string(),
    })
}

/// RTP receive-side transport: reads UDP datagrams, strips the 12-byte
/// RTP header, returns the TS payload bytes to callers.
///
/// Malformed packets (wrong version, wrong payload type, truncated,
/// CSRC list overflowing the datagram) are silently dropped — the
/// counter on [`Self::rtp_stats`] ticks for diagnosis. RFC 3550 §5.1
/// expects receivers to ignore unparseable packets.
pub struct RtpRecvTransport {
    socket: Option<UdpSocket>,
    /// Max UDP payload — used to size the recv scratch buffer.
    max_payload: usize,
    cancel: Arc<RtpCancelHandle>,
    bytes_received: u64,
    packets_received: u64,
    /// Counter for RTP packets that failed the header check.
    malformed_packets: u64,
    /// Per-recv scratch — heap allocated once.
    scratch: Vec<u8>,
}

/// RTP-protocol-level stats separate from [`SocketStats`].
///
/// Currently exposes only the malformed-packet counter. Future fields
/// (out-of-order delta, gap counter, etc.) can be added under
/// `#[non_exhaustive]` without breaking consumers.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RtpStats {
    /// Number of UDP datagrams received whose RTP header was invalid
    /// (wrong V, wrong PT, truncated, CSRC overflow). Cumulative since
    /// `listen()`.
    pub malformed_packets: u64,
}

impl RtpRecvTransport {
    /// Bind to `url`'s host:port and (for multicast) join the group.
    pub fn listen(url: &str) -> Result<Self, ConnectError> {
        let parsed = RtpUrl::parse(url).map_err(ConnectError::Url)?;
        Self::listen_with(&parsed)
    }

    /// Bind using an already-parsed URL.
    pub fn listen_with(url: &RtpUrl) -> Result<Self, ConnectError> {
        let ip: IpAddr = url.host.parse().map_err(|e: std::net::AddrParseError| {
            ConnectError::HostNotLiteral {
                host: url.host.clone(),
                detail: e.to_string(),
            }
        })?;
        // For multicast recv, bind to ANY:port and JoinMulticast on the
        // group; for unicast recv, bind to the literal host:port. This
        // matches GStreamer's `udpsrc address=...` behavior and tcpdump.
        let is_multicast = match ip {
            IpAddr::V4(v4) => v4.is_multicast(),
            IpAddr::V6(v6) => v6.is_multicast(),
        };
        let local: SocketAddr = if is_multicast {
            match ip {
                IpAddr::V4(_) => SocketAddr::new("0.0.0.0".parse().unwrap(), url.port),
                IpAddr::V6(_) => SocketAddr::new("::".parse().unwrap(), url.port),
            }
        } else {
            SocketAddr::new(ip, url.port)
        };
        let socket = UdpSocket::bind(local).map_err(ConnectError::Io)?;
        socket
            .set_read_timeout(Some(CANCEL_POLL_INTERVAL))
            .map_err(ConnectError::Io)?;
        if is_multicast {
            apply_multicast_recv_join(&socket, &ip, url)?;
        }
        Ok(Self {
            socket: Some(socket),
            max_payload: url.pkt_size,
            cancel: RtpCancelHandle::new(),
            bytes_received: 0,
            packets_received: 0,
            malformed_packets: 0,
            scratch: vec![0u8; url.pkt_size],
        })
    }

    /// RTP-protocol-level stats — separate from [`SocketStats`].
    pub fn rtp_stats(&self) -> RtpStats {
        RtpStats {
            malformed_packets: self.malformed_packets,
        }
    }
}

impl RecvTransport for RtpRecvTransport {
    fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        let socket = self.socket.as_ref().ok_or(TransportError::Closed)?;
        loop {
            if self.cancel.is_cancelled() {
                return Err(TransportError::ExplicitClose);
            }
            match socket.recv(&mut self.scratch) {
                Ok(0) => continue, // Zero-byte recv is meaningless on UDP; loop.
                Ok(n) => {
                    self.bytes_received += n as u64;
                    self.packets_received += 1;
                    match RtpHeader::decode(&self.scratch[..n]) {
                        Ok(parsed) => {
                            let payload = &self.scratch[parsed.payload_offset..n];
                            if payload.len() > buf.len() {
                                // Caller buf too small. Treat as broken,
                                // since the recv shell is misconfigured
                                // (it should have sized buf to at least
                                // max_payload()).
                                return Err(TransportError::Broken {
                                    msg: format!(
                                        "recv buf too small: {} < {}",
                                        buf.len(),
                                        payload.len()
                                    ),
                                    errno_code: None,
                                });
                            }
                            buf[..payload.len()].copy_from_slice(payload);
                            return Ok(payload.len());
                        }
                        Err(parse_err) => {
                            self.malformed_packets = self.malformed_packets.saturating_add(1);
                            tracing::debug!(
                                error = ?parse_err,
                                "RTP packet rejected at recv; counter ticked",
                            );
                            // Drop + continue the recv loop.
                            continue;
                        }
                    }
                }
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.kind() == io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(e) => {
                    self.socket = None;
                    return Err(TransportError::Broken {
                        msg: format!("UDP recv failed: {e}"),
                        errno_code: e.raw_os_error(),
                    });
                }
            }
        }
    }

    fn max_payload(&self) -> usize {
        self.max_payload.saturating_sub(RTP_HEADER_LEN)
    }

    fn is_alive(&self) -> bool {
        self.socket.is_some()
    }

    fn close(&mut self) {
        self.socket = None;
    }

    fn cancel_handle(&self) -> Option<Arc<dyn TransportCancel + Send + Sync>> {
        Some(self.cancel.clone() as Arc<dyn TransportCancel + Send + Sync>)
    }

    fn socket_stats(&self) -> Option<SocketStats> {
        self.socket.as_ref()?;
        #[allow(clippy::field_reassign_with_default)]
        // SocketStats is #[non_exhaustive] in tst-core, so the
        // default-and-assign pattern is the only way to construct one
        // from outside that crate.
        let mut s = SocketStats::default();
        s.bytes_received = self.bytes_received;
        s.packets_received = self.packets_received;
        Some(s)
    }
}

impl Drop for RtpRecvTransport {
    fn drop(&mut self) {
        self.close();
    }
}

/// Join the multicast group on the receive socket. IPv4 uses
/// `join_multicast_v4(group, interface)`; IPv6 uses
/// `join_multicast_v6(group, interface_index)`.
///
/// `iface` parsing matches the send-side rules in
/// [`apply_multicast_iface`]: IPv4 takes a literal IPv4 address;
/// IPv6 isn't supported in Phase 1 (returns
/// [`ConnectError::IfaceUnsupported`]).
fn apply_multicast_recv_join(
    socket: &UdpSocket,
    ip: &IpAddr,
    url: &RtpUrl,
) -> Result<(), ConnectError> {
    match ip {
        IpAddr::V4(group) => {
            let iface_v4 = match url.iface.as_deref() {
                Some(iface) => iface.parse::<std::net::Ipv4Addr>().map_err(|e| {
                    ConnectError::IfaceUnsupported {
                        iface: iface.to_string(),
                        detail: format!(
                            "IPv4 multicast iface requires literal IPv4 address, got '{iface}': {e}"
                        ),
                    }
                })?,
                None => std::net::Ipv4Addr::UNSPECIFIED, // 0.0.0.0 — OS default
            };
            socket
                .join_multicast_v4(group, &iface_v4)
                .map_err(ConnectError::Io)?;
        }
        IpAddr::V6(group) => {
            let scope_id = match url.iface.as_deref() {
                None => 0, // Default scope.
                Some(iface) => {
                    return Err(ConnectError::IfaceUnsupported {
                        iface: iface.to_string(),
                        detail: "IPv6 multicast iface name lookup not implemented in Phase 1; ?iface= must be omitted for ipv6 receive".to_string(),
                    });
                }
            };
            socket
                .join_multicast_v6(group, scope_id)
                .map_err(ConnectError::Io)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify socket_stats() now returns Some(_) once Task 9 wires up
    /// the local counters. bytes_sent / packets_sent advance through
    /// the integration test in Task 14; here we just check the shape.
    #[test]
    fn socket_stats_returns_some_when_alive() {
        let url = RtpUrl::parse("rtp://127.0.0.1:1").unwrap();
        let t = RtpTransport::connect_with(&url).unwrap();
        let stats = t.socket_stats().expect("alive transport reports stats");
        assert_eq!(stats.bytes_sent, 0);
        assert_eq!(stats.packets_sent, 0);
        // RTCP-derived fields should stay zero in Phase 1.
        assert_eq!(stats.rtt_us, 0);
        assert_eq!(stats.packets_lost_send, 0);
    }
}
