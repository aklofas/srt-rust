//! `RtpTransport` (send) + `RtpRecvTransport` (recv) — sync UDP socket
//! wrappers behind the [`tst_core::transport`] traits.
//!
//! Phase 1 ships only the UDP data plane; RTSP control plane (Phase 2)
//! is what makes negotiated transports work. For now, sender + receiver
//! agree on a fixed `host:port` and use it directly.

use std::io;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};

use tst_core::net::udp_socket::{
    CANCEL_POLL_INTERVAL, apply_multicast_recv_join, apply_multicast_send_knobs,
};
use tst_core::transport::{RecvTransport, SocketStats, Transport, TransportCancel, TransportError};

use crate::cancel::RtpCancelHandle;
use crate::clock::RtpClock;
use crate::packet::{RTP_HEADER_LEN, RtpHeader};
use crate::rtcp::ingest::{ingest_rr, ingest_sr};
use crate::rtcp::reporter::RtcpReporterHandle;
use crate::rtcp::stats::RtcpStats;
use crate::rtcp::{ReceiverReport, SdesPacket, SenderReport};
use crate::url::{RtpUrl, UrlError as RtpUrlError};

/// Convert an `io::Error` from the shared UDP socket helpers into a
/// `ConnectError`. `io::ErrorKind::Unsupported` (produced by
/// `apply_multicast_iface` / `set_multicast_hops_v6` / `set_multicast_if_v4`)
/// maps to `ConnectError::IfaceUnsupported`; everything else maps to
/// `ConnectError::Io`.
fn udp_err_to_connect(e: io::Error) -> ConnectError {
    if e.kind() == io::ErrorKind::Unsupported {
        ConnectError::IfaceUnsupported {
            iface: String::new(),
            detail: e.to_string(),
        }
    } else {
        ConnectError::Io(e)
    }
}

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
    /// Companion RTCP socket bound on `port + 1` per RFC 3550 §11.
    /// `None` when the caller opted out via `RtpSocketBuilder::rtcp(false)`.
    /// Retained on the transport so it stays alive for the reporter
    /// thread's lifetime — the reporter holds its own `try_clone`'d FD.
    #[allow(dead_code)]
    rtcp_socket: Option<UdpSocket>,
    /// RTCP-derived counters, shared with the reporter thread (which
    /// ticks `sr_packets_sent` on each SR emission) and any Task-8
    /// ingest path.
    rtcp_stats: Arc<Mutex<RtcpStats>>,
    /// Background SR-emitter handle. Dropping this cancels + joins
    /// the reporter thread. Held only for its `Drop` side effect.
    #[allow(dead_code)]
    rtcp_reporter: Option<RtcpReporterHandle>,
}

impl RtpTransport {
    /// Connect (just sets `SocketAddr::connect`-style default) and
    /// return a ready-to-send transport.
    ///
    /// `url` must have scheme `rtp://` and an explicit port.
    ///
    /// As of Phase 2 Task 10, this also binds an RTCP companion socket
    /// on `port + 1` and spawns the SR-emitter thread. Opt out via
    /// [`crate::builder::RtpSocketBuilder::rtcp`].
    pub fn connect(url: &str) -> Result<Self, ConnectError> {
        let parsed = RtpUrl::parse(url).map_err(ConnectError::Url)?;
        Self::connect_with_rtcp(&parsed, true)
    }

    /// Connect using an already-parsed URL — convenient for callers that
    /// hold an `RtpUrl` (e.g., binding crates). RTCP defaults on.
    pub fn connect_with(url: &RtpUrl) -> Result<Self, ConnectError> {
        Self::connect_with_rtcp(url, true)
    }

    /// Connect using an already-parsed URL with an explicit RTCP toggle.
    /// `rtcp_enabled = false` skips the RTCP socket-pair + reporter thread.
    pub fn connect_with_rtcp(url: &RtpUrl, rtcp_enabled: bool) -> Result<Self, ConnectError> {
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
            apply_multicast_send_knobs(&socket, ip, url.ttl, url.iface.as_deref())
                .map_err(udp_err_to_connect)?;
        }
        socket.connect(peer).map_err(ConnectError::Io)?;
        // RTCP companion socket bound on `port + 1` per RFC 3550 §11.
        // Sender binds an ephemeral local port + sends SR to peer's
        // port+1 (the symmetric pair).
        let rtcp_socket = if rtcp_enabled {
            let rtcp_local: SocketAddr = match ip {
                IpAddr::V4(_) => "0.0.0.0:0".parse().unwrap(),
                IpAddr::V6(_) => "[::]:0".parse().unwrap(),
            };
            Some(UdpSocket::bind(rtcp_local).map_err(ConnectError::Io)?)
        } else {
            None
        };
        Ok(Self::from_socket(socket, url, rtcp_socket, peer))
    }

    /// Internal: build from an already-configured socket.
    fn from_socket(
        socket: UdpSocket,
        url: &RtpUrl,
        rtcp_socket: Option<UdpSocket>,
        peer: SocketAddr,
    ) -> Self {
        let ssrc = url.ssrc.unwrap_or_else(random_u32);
        let next_seq = random_u32() as u16;
        let start_ticks = random_u32();
        let rtcp_stats = Arc::new(Mutex::new(RtcpStats::default()));
        // Spawn the SR-emitter thread when RTCP is enabled. The closure
        // grabs its own clone of the rtcp socket FD + the stats handle;
        // both live for the reporter thread's lifetime via Arc/`try_clone`.
        let rtcp_reporter = match rtcp_socket.as_ref() {
            Some(sock) => {
                // try_clone gives the reporter its own FD ref; close
                // semantics on the original FD stay intact.
                let sock_clone = match sock.try_clone() {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "rtcp socket try_clone failed; skipping reporter");
                        return Self {
                            socket: Some(socket),
                            max_payload: url.pkt_size,
                            clock: RtpClock::new(start_ticks),
                            ssrc,
                            next_seq,
                            cancel: RtpCancelHandle::new(),
                            bytes_sent: 0,
                            packets_sent: 0,
                            rtcp_socket,
                            rtcp_stats,
                            rtcp_reporter: None,
                        };
                    }
                };
                let stats_clone = rtcp_stats.clone();
                // Guard: port 65535 has no valid RTCP companion port
                // (65536 overflows u16). Skip the reporter in that case,
                // mirroring the guard in rtsp/server/handlers.rs:444.
                let Some(rtcp_companion_port) = peer.port().checked_add(1) else {
                    tracing::warn!("peer port 65535 has no RTCP companion; skipping SR reporter");
                    return Self {
                        socket: Some(socket),
                        max_payload: url.pkt_size,
                        clock: RtpClock::new(start_ticks),
                        ssrc,
                        next_seq,
                        cancel: RtpCancelHandle::new(),
                        bytes_sent: 0,
                        packets_sent: 0,
                        rtcp_socket,
                        rtcp_stats,
                        rtcp_reporter: None,
                    };
                };
                let rtcp_target = SocketAddr::new(peer.ip(), rtcp_companion_port);
                Some(RtcpReporterHandle::spawn(move || {
                    // Phase 2 v1: SR carries running totals snapshot.
                    // Real bytes_sent / packets_sent live on the
                    // transport — for the v1 reporter we emit a
                    // minimal SR (counters zero) + SDES CNAME. Full
                    // SR counter wire-up happens in Phase 2 Task 14
                    // (integration). The reporter thread + socket
                    // pair are what Task 10 retrofits — the SR's
                    // counters are a follow-up.
                    let sr = crate::rtcp::SenderReport {
                        ssrc,
                        ntp_timestamp: 0,
                        rtp_timestamp: 0,
                        sender_packet_count: 0,
                        sender_octet_count: 0,
                        report_blocks: Vec::new(),
                    };
                    let cname = format!("tst-rtp-{ssrc:08x}");
                    let sdes = SdesPacket { ssrc, cname };
                    // These are locally-built, well-formed packets (no report
                    // blocks, short CNAME) so encode never fails — but the
                    // encoders are fallible now; skip the send on the
                    // (unreachable) validation error rather than unwrapping.
                    let (Ok(mut compound), Ok(sdes_bytes)) = (sr.encode(), sdes.encode()) else {
                        tracing::error!(
                            "internal: locally-built RTCP SR/SDES failed to encode; skipping send"
                        );
                        debug_assert!(false, "locally-built RTCP SR/SDES must always encode");
                        return;
                    };
                    compound.extend_from_slice(&sdes_bytes);
                    let _ = sock_clone.send_to(&compound, rtcp_target);
                    if let Ok(mut g) = stats_clone.lock() {
                        g.sr_packets_sent = g.sr_packets_sent.saturating_add(1);
                    }
                }))
            }
            None => None,
        };
        Self {
            socket: Some(socket),
            max_payload: url.pkt_size,
            clock: RtpClock::new(start_ticks),
            ssrc,
            next_seq,
            cancel: RtpCancelHandle::new(),
            bytes_sent: 0,
            packets_sent: 0,
            rtcp_socket,
            rtcp_stats,
            rtcp_reporter,
        }
    }

    /// Snapshot of the RTCP-derived counters. Returns a clone of the
    /// internal `RtcpStats` (cheap — counters are plain integers).
    pub fn rtcp_stats(&self) -> RtcpStats {
        self.rtcp_stats
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
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

/// Inner data source for [`RtpRecvTransport`].
///
/// `Udp` — the Phase 1 default: read UDP datagrams off a bound socket
/// and strip the RTP header in `recv_bytes`.
///
/// `Mpsc` — the TCP-interleaved bridge introduced in Phase 2 Task 17:
/// an `InterleavedReader` background thread parses `$<ch><len><data>`
/// frames off the RTSP control TCP and pushes the *unwrapped* RTP
/// payload bytes (i.e., the TS bundle, already past the 12-byte RTP
/// header) through an mpsc channel. `recv_bytes` here just dequeues a
/// chunk; the RTP header strip happened in the bridge thread.
pub(crate) enum Source {
    Udp(UdpSocket),
    Mpsc(std::sync::mpsc::Receiver<bytes::Bytes>),
}

/// RTP receive-side transport: reads UDP datagrams, strips the 12-byte
/// RTP header, returns the TS payload bytes to callers.
///
/// Malformed packets (wrong version, wrong payload type, truncated,
/// CSRC list overflowing the datagram) are silently dropped — the
/// counter on [`Self::rtp_stats`] ticks for diagnosis. RFC 3550 §5.1
/// expects receivers to ignore unparseable packets.
///
/// # `SocketStats` field mapping
///
/// | [`SocketStats`] field | Source |
/// |---|---|
/// | `bytes_received` / `packets_received` | Local counters, tick per `recv_bytes` |
/// | `rtt_us` | RTCP RR-after-SR computation (RFC 3550 §6.4.1). Populates on TCP-interleaved RTSP client paths (the `RtspSession::into_recv_transport` route wires it via `from_mpsc_with_rtcp`); `0` on UDP (RTCP ingest on UDP is not yet wired) |
/// | `packets_lost_send` | RTCP RR cumulative-lost field. Populates on the same paths as `rtt_us`; `0` otherwise |
/// | `bytes_sent` / `packets_sent` | 0 (this is the receive half) |
/// | All other fields | 0 |
pub struct RtpRecvTransport {
    /// Underlying byte source — UDP socket or mpsc-fed
    /// TCP-interleaved bridge. `None` after [`Self::close`].
    source: Option<Source>,
    /// Max UDP payload — used to size the recv scratch buffer.
    max_payload: usize,
    cancel: Arc<RtpCancelHandle>,
    bytes_received: u64,
    packets_received: u64,
    /// Counter for RTP packets that failed the header check.
    malformed_packets: u64,
    /// Per-recv scratch — heap allocated once.
    scratch: Vec<u8>,
    /// Companion RTCP socket bound on `port + 1` per RFC 3550 §11.
    /// `None` when the caller opted out via `RtpRecvSocketBuilder::rtcp(false)`.
    #[allow(dead_code)]
    rtcp_socket: Option<UdpSocket>,
    /// RTCP-derived counters, shared with the reporter thread (which
    /// ticks `rr_packets_sent` on each RR emission) and any Task-8
    /// ingest path.
    rtcp_stats: Arc<Mutex<RtcpStats>>,
    /// Background RR-emitter handle. Dropping this cancels + joins
    /// the reporter thread. Held only for its `Drop` side effect.
    #[allow(dead_code)]
    rtcp_reporter: Option<RtcpReporterHandle>,
    /// Local SSRC used in the RR sender field — defaults to a random
    /// value generated at listen time; matches the RTP send-side
    /// pattern. Captured by the reporter closure; field retained for
    /// reflection by Task 8 ingest paths.
    #[allow(dead_code)]
    ssrc: u32,
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
    ///
    /// As of Phase 2 Task 10, this also binds an RTCP companion socket
    /// on `port + 1` and spawns the RR-emitter thread. Opt out via
    /// [`crate::builder::RtpRecvSocketBuilder::rtcp`].
    pub fn listen(url: &str) -> Result<Self, ConnectError> {
        let parsed = RtpUrl::parse(url).map_err(ConnectError::Url)?;
        Self::listen_with_rtcp(&parsed, true)
    }

    /// Bind using an already-parsed URL. RTCP defaults on.
    pub fn listen_with(url: &RtpUrl) -> Result<Self, ConnectError> {
        Self::listen_with_rtcp(url, true)
    }

    /// Bind using an already-parsed URL with an explicit RTCP toggle.
    /// `rtcp_enabled = false` skips the RTCP socket-pair + reporter thread.
    pub fn listen_with_rtcp(url: &RtpUrl, rtcp_enabled: bool) -> Result<Self, ConnectError> {
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
            apply_multicast_recv_join(&socket, ip, url.iface.as_deref())
                .map_err(udp_err_to_connect)?;
        }
        // RTCP companion socket bound on `port + 1` per RFC 3550 §11.
        // Use the RTP socket's actual local port (kernel-assigned when
        // url.port == 0) — `url.port + 1` would resolve to port 1 in
        // that case, which is privileged.
        let rtcp_socket = if rtcp_enabled {
            let actual_rtp_port = socket.local_addr().map_err(ConnectError::Io)?.port();
            // Guard: if the kernel handed us port 65535 (or the caller
            // requested it explicitly), there is no valid RTCP companion
            // port. Skip the RTCP socket rather than wrapping to 0.
            // Mirrors the guard in rtsp/server/handlers.rs:444.
            if let Some(rtcp_port) = actual_rtp_port.checked_add(1) {
                let rtcp_local: SocketAddr = if is_multicast {
                    match ip {
                        IpAddr::V4(_) => SocketAddr::new("0.0.0.0".parse().unwrap(), rtcp_port),
                        IpAddr::V6(_) => SocketAddr::new("::".parse().unwrap(), rtcp_port),
                    }
                } else {
                    SocketAddr::new(ip, rtcp_port)
                };
                Some(UdpSocket::bind(rtcp_local).map_err(ConnectError::Io)?)
            } else {
                tracing::warn!(
                    "RTP port 65535 has no valid RTCP companion; \
                     RTCP companion socket skipped"
                );
                None
            }
        } else {
            None
        };
        let ssrc = url.ssrc.unwrap_or_else(random_u32);
        let rtcp_stats = Arc::new(Mutex::new(RtcpStats::default()));
        // Spawn the RR-emitter thread when RTCP is enabled. v1: target
        // is symmetric — RTP-port + 1 of the peer we last received
        // from. With no peer seen yet, target the URL's host:port+1
        // (the symmetric assumption for a known-destination receiver).
        let rtcp_reporter = match rtcp_socket.as_ref() {
            Some(sock) => {
                let sock_clone = match sock.try_clone() {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(error = %e, "rtcp socket try_clone failed; skipping reporter");
                        return Ok(Self {
                            source: Some(Source::Udp(socket)),
                            max_payload: url.pkt_size,
                            cancel: RtpCancelHandle::new(),
                            bytes_received: 0,
                            packets_received: 0,
                            malformed_packets: 0,
                            scratch: vec![0u8; url.pkt_size],
                            rtcp_socket,
                            rtcp_stats,
                            rtcp_reporter: None,
                            ssrc,
                        });
                    }
                };
                let stats_clone = rtcp_stats.clone();
                // For non-multicast, the URL host is the address we
                // bound to — so the symmetric RTCP target is host:port+1.
                // For multicast, there's no per-peer notion here; v1
                // doesn't emit RR until a peer is observed (Task 8's
                // ingest path lands that wiring). For now we still
                // spawn the thread so the rr_packets_sent counter
                // ticks deterministically against the URL host.
                //
                // Guard: url.port 65535 has no valid companion port.
                // The RTCP socket is already None in that case (guarded
                // above), so this arm is unreachable at 65535 — the
                // checked_add is a belt-and-suspenders defence.
                let Some(rtcp_companion_port) = url.port.checked_add(1) else {
                    tracing::warn!("url port 65535 has no RTCP companion; skipping RR reporter");
                    return Ok(Self {
                        source: Some(Source::Udp(socket)),
                        max_payload: url.pkt_size,
                        cancel: RtpCancelHandle::new(),
                        bytes_received: 0,
                        packets_received: 0,
                        malformed_packets: 0,
                        scratch: vec![0u8; url.pkt_size],
                        rtcp_socket,
                        rtcp_stats,
                        rtcp_reporter: None,
                        ssrc,
                    });
                };
                let rtcp_target = SocketAddr::new(ip, rtcp_companion_port);
                Some(RtcpReporterHandle::spawn(move || {
                    let rr = ReceiverReport {
                        ssrc,
                        report_blocks: Vec::new(),
                    };
                    let cname = format!("tst-rtp-{ssrc:08x}");
                    let sdes = SdesPacket { ssrc, cname };
                    // Locally-built, well-formed packets (no report blocks,
                    // short CNAME) — encode is fallible now but never fails
                    // here; skip the send on the (unreachable) error.
                    let (Ok(mut compound), Ok(sdes_bytes)) = (rr.encode(), sdes.encode()) else {
                        tracing::error!(
                            "internal: locally-built RTCP RR/SDES failed to encode; skipping send"
                        );
                        debug_assert!(false, "locally-built RTCP RR/SDES must always encode");
                        return;
                    };
                    compound.extend_from_slice(&sdes_bytes);
                    let _ = sock_clone.send_to(&compound, rtcp_target);
                    if let Ok(mut g) = stats_clone.lock() {
                        g.rr_packets_sent = g.rr_packets_sent.saturating_add(1);
                    }
                }))
            }
            None => None,
        };
        Ok(Self {
            source: Some(Source::Udp(socket)),
            max_payload: url.pkt_size,
            cancel: RtpCancelHandle::new(),
            bytes_received: 0,
            packets_received: 0,
            malformed_packets: 0,
            scratch: vec![0u8; url.pkt_size],
            rtcp_socket,
            rtcp_stats,
            rtcp_reporter,
            ssrc,
        })
    }

    /// Build an `RtpRecvTransport` from an already-bound UDP socket.
    ///
    /// Used by [`crate::rtsp::client::session::RtspSession::into_recv_transport`]
    /// to wrap the UDP socket pair the RTSP SETUP exchange allocated,
    /// without re-binding via [`Self::listen`]. RTCP is not started here
    /// — the RTSP control plane drives RR/SR via a different path in the
    /// SETUP-direct flow.
    ///
    /// The caller is responsible for any platform-specific setup
    /// (multicast joins, TTL, etc.) before passing the socket in. This
    /// constructor only sets the cancel-poll read timeout to match the
    /// rest of the recv-side machinery.
    pub(crate) fn from_udp_socket(socket: UdpSocket) -> Result<Self, ConnectError> {
        socket
            .set_read_timeout(Some(CANCEL_POLL_INTERVAL))
            .map_err(ConnectError::Io)?;
        let pkt_size = crate::url::DEFAULT_PKT_SIZE;
        let ssrc = random_u32();
        Ok(Self {
            source: Some(Source::Udp(socket)),
            max_payload: pkt_size,
            cancel: RtpCancelHandle::new(),
            bytes_received: 0,
            packets_received: 0,
            malformed_packets: 0,
            scratch: vec![0u8; pkt_size],
            rtcp_socket: None,
            rtcp_stats: Arc::new(Mutex::new(RtcpStats::default())),
            rtcp_reporter: None,
            ssrc,
        })
    }

    /// Construct an `RtpRecvTransport` whose source is an mpsc channel
    /// fed by the RTSP client's `InterleavedReader` background thread.
    ///
    /// Used by
    /// [`crate::rtsp::client::session::RtspSession::into_recv_transport`]
    /// when SETUP negotiated TCP-interleaved transport. The producer
    /// (an InterleavedReader-driven thread inside the RtspClient) parses
    /// `$<ch><len><data>` frames off the RTSP control TCP, strips the
    /// 12-byte RTP header, and pushes the TS bundle into `rx`'s paired
    /// sender. `recv_bytes` on the resulting transport just dequeues a
    /// chunk per call.
    ///
    /// `rx` is the consumer side of the bridge; the producer side
    /// (`Sender<Bytes>`) is held by the InterleavedReader thread.
    pub(crate) fn from_mpsc_placeholder(rx: std::sync::mpsc::Receiver<bytes::Bytes>) -> Self {
        let pkt_size = crate::url::DEFAULT_PKT_SIZE;
        let ssrc = random_u32();
        Self {
            source: Some(Source::Mpsc(rx)),
            max_payload: pkt_size,
            cancel: RtpCancelHandle::new(),
            bytes_received: 0,
            packets_received: 0,
            malformed_packets: 0,
            // No scratch needed for mpsc path — payload is already
            // RTP-header-stripped by the bridge thread — but keep the
            // allocation so a future code path can fall back to the
            // shared buffer without conditional malloc.
            scratch: vec![0u8; pkt_size],
            rtcp_socket: None,
            rtcp_stats: Arc::new(Mutex::new(RtcpStats::default())),
            rtcp_reporter: None,
            ssrc,
        }
    }

    /// Variant of [`Self::from_mpsc_placeholder`] that also spawns a
    /// background `rtsp-rtcp-ingest` thread to drain `rtcp_rx` (the
    /// RTCP channel demuxed by the RTSP client's interleaved pump) and
    /// feed each packet into the shared [`RtcpStats`] via [`ingest_rr`]
    /// or [`ingest_sr`]. Unknown PTs (SDES/BYE/APP/etc.) are counted
    /// as ignored and skipped.
    ///
    /// Used by
    /// [`crate::rtsp::client::session::RtspSession::into_recv_transport`]
    /// on the TCP-interleaved path. Without this constructor the RTCP
    /// channel is silently consumed and `socket_stats().rtt_us` /
    /// `packets_lost_send` stay at 0. The thread exits when the pump's
    /// `Sender<Bytes>` is dropped (which produces `mpsc::RecvError`).
    pub(crate) fn from_mpsc_with_rtcp(
        data_rx: std::sync::mpsc::Receiver<bytes::Bytes>,
        rtcp_rx: std::sync::mpsc::Receiver<bytes::Bytes>,
    ) -> Self {
        let t = Self::from_mpsc_placeholder(data_rx);
        spawn_rtcp_ingest(rtcp_rx, t.rtcp_stats.clone(), t.ssrc);
        t
    }

    /// RTP-protocol-level stats — separate from [`SocketStats`].
    pub fn rtp_stats(&self) -> RtpStats {
        RtpStats {
            malformed_packets: self.malformed_packets,
        }
    }

    /// Snapshot of the RTCP-derived counters. Returns a clone of the
    /// internal `RtcpStats` (cheap — counters are plain integers).
    pub fn rtcp_stats(&self) -> RtcpStats {
        self.rtcp_stats
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }
}

impl RecvTransport for RtpRecvTransport {
    fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        let source = self.source.as_ref().ok_or(TransportError::Closed)?;
        match source {
            Source::Udp(socket) => loop {
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
                                // Use payload_end (not n) to exclude any RFC 3550
                                // padding bytes and to reflect extension skipping.
                                let payload =
                                    &self.scratch[parsed.payload_offset..parsed.payload_end];
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
                        let raw_errno = e.raw_os_error();
                        let msg = format!("UDP recv failed: {e}");
                        self.source = None;
                        return Err(TransportError::Broken {
                            msg,
                            errno_code: raw_errno,
                        });
                    }
                }
            },
            Source::Mpsc(rx) => loop {
                if self.cancel.is_cancelled() {
                    return Err(TransportError::ExplicitClose);
                }
                // Same cancel-poll cadence as the UDP path. recv_timeout
                // wakes on either a value arriving or the timeout
                // elapsing — the latter just loops to re-check cancel.
                match rx.recv_timeout(CANCEL_POLL_INTERVAL) {
                    Ok(payload) => {
                        if payload.len() > buf.len() {
                            return Err(TransportError::Broken {
                                msg: format!(
                                    "recv buf too small: {} < {}",
                                    buf.len(),
                                    payload.len()
                                ),
                                errno_code: None,
                            });
                        }
                        self.bytes_received =
                            self.bytes_received.saturating_add(payload.len() as u64);
                        self.packets_received = self.packets_received.saturating_add(1);
                        buf[..payload.len()].copy_from_slice(&payload);
                        return Ok(payload.len());
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        // The InterleavedReader thread dropped its
                        // Sender — surface as a broken transport so
                        // the recv shell can stop the demux loop.
                        self.source = None;
                        return Err(TransportError::Broken {
                            msg: "InterleavedReader bridge disconnected".to_string(),
                            errno_code: None,
                        });
                    }
                }
            },
        }
    }

    fn max_payload(&self) -> usize {
        self.max_payload.saturating_sub(RTP_HEADER_LEN)
    }

    fn is_alive(&self) -> bool {
        self.source.is_some()
    }

    fn close(&mut self) {
        self.source = None;
    }

    fn cancel_handle(&self) -> Option<Arc<dyn TransportCancel + Send + Sync>> {
        Some(self.cancel.clone() as Arc<dyn TransportCancel + Send + Sync>)
    }

    fn socket_stats(&self) -> Option<SocketStats> {
        self.source.as_ref()?;
        #[allow(clippy::field_reassign_with_default)]
        // SocketStats is #[non_exhaustive] in tst-core, so the
        // default-and-assign pattern is the only way to construct one
        // from outside that crate.
        let mut s = SocketStats::default();
        s.bytes_received = self.bytes_received;
        s.packets_received = self.packets_received;
        // Project RTCP-derived fields when ingest has populated them.
        // Paths without an ingest thread (UDP today, mpsc-placeholder
        // pre-T28) leave these at 0, matching prior behavior.
        if let Ok(rtcp) = self.rtcp_stats.lock() {
            s.rtt_us = rtcp.rtt_us;
            s.packets_lost_send = rtcp.cumulative_lost_send as u64;
        }
        Some(s)
    }
}

impl Drop for RtpRecvTransport {
    fn drop(&mut self) {
        self.close();
    }
}

/// Spawn the `rtsp-rtcp-ingest` background thread.
///
/// Drains `rtcp_rx` (fed by the RTSP client's interleaved pump on the
/// RFC 7826 §14 RTCP channel), parses each frame, and dispatches to
/// [`ingest_sr`] (PT=200) or [`ingest_rr`] (PT=201). Unknown PTs
/// (SDES/BYE/APP) are ignored — they don't affect the projected
/// `SocketStats` fields. Parse errors increment
/// `rtcp_stats.sr_parse_errors` / `rr_parse_errors`.
///
/// The thread exits cleanly when the pump's `Sender<Bytes>` drops
/// (`rtcp_rx.recv()` returns `Err(mpsc::RecvError)`). It is detached
/// (no join) — the transport's lifecycle holds the consumer side, and
/// once the transport drops + the pump dies the channel closes.
fn spawn_rtcp_ingest(
    rtcp_rx: std::sync::mpsc::Receiver<bytes::Bytes>,
    stats: Arc<Mutex<RtcpStats>>,
    our_ssrc: u32,
) {
    std::thread::Builder::new()
        .name("rtsp-rtcp-ingest".to_string())
        .spawn(move || {
            while let Ok(bytes) = rtcp_rx.recv() {
                // RFC 3550 §6.4 — PT byte lives at offset 1 of every RTCP packet.
                // Packets shorter than 2 bytes are non-conformant; drop silently.
                if bytes.len() < 2 {
                    continue;
                }
                let pt = bytes[1];
                match pt {
                    200 => match SenderReport::decode(&bytes) {
                        Ok((sr, _)) => {
                            if let Ok(mut g) = stats.lock() {
                                ingest_sr(&mut g, &sr);
                            }
                        }
                        Err(_) => {
                            if let Ok(mut g) = stats.lock() {
                                g.sr_parse_errors = g.sr_parse_errors.saturating_add(1);
                            }
                        }
                    },
                    201 => match ReceiverReport::decode(&bytes) {
                        Ok((rr, _)) => {
                            if let Ok(mut g) = stats.lock() {
                                ingest_rr(&mut g, our_ssrc, &rr);
                            }
                        }
                        Err(_) => {
                            if let Ok(mut g) = stats.lock() {
                                g.rr_parse_errors = g.rr_parse_errors.saturating_add(1);
                            }
                        }
                    },
                    _ => continue, // SDES/BYE/APP/etc. — ignored for stats.
                }
            }
        })
        .expect("failed to spawn rtsp-rtcp-ingest thread");
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

    /// Compile-time check: RtpTransport satisfies Transport (so
    /// MuxSender<RtpTransport> works) and RtpRecvTransport satisfies
    /// RecvTransport (so DemuxReceiver<RtpRecvTransport> works).
    /// Catches signature drift if the pipeline shell trait bounds tighten.
    #[test]
    fn satisfies_pipeline_trait_bounds() {
        fn accept_send<T: Transport>(_: T) {}
        fn accept_recv<T: RecvTransport>(_: T) {}
        // Use port 0 / 1 to avoid binding conflicts in CI.
        let send_url = RtpUrl::parse("rtp://127.0.0.1:1").unwrap();
        let recv_url = RtpUrl::parse("rtp://127.0.0.1:0").unwrap();
        let send = RtpTransport::connect_with(&send_url).unwrap();
        let recv = RtpRecvTransport::listen_with(&recv_url).unwrap();
        accept_send(send);
        accept_recv(recv);
    }

    /// T30 — verify that an RR pushed onto the rtcp_rx channel of an
    /// `RtpRecvTransport` built via `from_mpsc_with_rtcp` reaches the
    /// ingest thread, populates `RtcpStats.cumulative_lost_send`, and
    /// is projected into `socket_stats().packets_lost_send`. Closes
    /// the Stage 3 RR-on-interleaved-channel deliverable.
    #[test]
    fn rr_on_rtcp_rx_populates_packets_lost_send() {
        use crate::rtcp::{ReceiverReport, ReportBlock};

        let (_data_tx, data_rx) = std::sync::mpsc::channel::<bytes::Bytes>();
        let (rtcp_tx, rtcp_rx) = std::sync::mpsc::channel::<bytes::Bytes>();
        let t = RtpRecvTransport::from_mpsc_with_rtcp(data_rx, rtcp_rx);
        let our_ssrc = t.ssrc;

        // Build an RR whose first report block references our SSRC and
        // reports 2024 cumulative losses.
        let rb = ReportBlock {
            ssrc: our_ssrc,
            fraction_lost: 0x40, // 25% q8
            cumulative_lost: 2024,
            extended_highest_seq: 9000,
            jitter: 0,
            last_sr: 0,
            delay_since_last_sr: 0,
        };
        let rr = ReceiverReport {
            ssrc: 0xDEAD_BEEF,
            report_blocks: vec![rb],
        };
        let bytes = bytes::Bytes::from(rr.encode().unwrap());
        rtcp_tx.send(bytes).expect("send RR onto rtcp_rx");

        // Spin briefly for the ingest thread to wake + process.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut surfaced = 0u64;
        while std::time::Instant::now() < deadline {
            let s = t.socket_stats().expect("alive transport reports stats");
            if s.packets_lost_send > 0 {
                surfaced = s.packets_lost_send;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            surfaced, 2024,
            "expected RR-derived loss 2024 surfaced as packets_lost_send"
        );

        // RTT stays 0 because no SR anchor preceded the RR.
        let s = t.socket_stats().expect("alive transport reports stats");
        assert_eq!(s.rtt_us, 0);
    }

    /// T30 — verify that an SR followed by a matching RR populates
    /// `socket_stats().rtt_us`. The RB's `last_sr` must equal the
    /// stored anchor's mid-32 NTP for `compute_rtt_us` to fire.
    #[test]
    fn sr_then_rr_populates_rtt_us() {
        use crate::rtcp::{ReceiverReport, ReportBlock, SenderReport};

        let (_data_tx, data_rx) = std::sync::mpsc::channel::<bytes::Bytes>();
        let (rtcp_tx, rtcp_rx) = std::sync::mpsc::channel::<bytes::Bytes>();
        let t = RtpRecvTransport::from_mpsc_with_rtcp(data_rx, rtcp_rx);
        let our_ssrc = t.ssrc;

        // Peer SR — establishes the anchor.
        let sr_ntp_full: u64 = 0xDEAD_BEEF_0000_0000u64; // arbitrary
        let sr = SenderReport {
            ssrc: 0xCAFEBABE,
            ntp_timestamp: sr_ntp_full,
            rtp_timestamp: 0,
            sender_packet_count: 0,
            sender_octet_count: 0,
            report_blocks: vec![],
        };
        rtcp_tx
            .send(bytes::Bytes::from(sr.encode().unwrap()))
            .expect("send SR");

        // Give the ingest thread a moment to process the SR.
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Peer RR — RB's `last_sr` mirrors the anchor's mid-32 NTP.
        let last_sr_mid = ((sr_ntp_full >> 16) & 0xFFFF_FFFF) as u32;
        let rb = ReportBlock {
            ssrc: our_ssrc,
            fraction_lost: 0,
            cumulative_lost: 0,
            extended_highest_seq: 0,
            jitter: 0,
            last_sr: last_sr_mid,
            delay_since_last_sr: 0,
        };
        let rr = ReceiverReport {
            ssrc: 0xCAFEBABE,
            report_blocks: vec![rb],
        };
        rtcp_tx
            .send(bytes::Bytes::from(rr.encode().unwrap()))
            .expect("send RR");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut surfaced_rtt = 0u32;
        while std::time::Instant::now() < deadline {
            let s = t.socket_stats().expect("alive transport reports stats");
            if s.rtt_us > 0 {
                surfaced_rtt = s.rtt_us;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            surfaced_rtt > 0,
            "expected SR-then-RR to surface a non-zero rtt_us, got {surfaced_rtt}"
        );
    }

    /// T30 — verify that a malformed RTCP packet (PT=201 but truncated)
    /// increments `rtcp_stats().rr_parse_errors` and does NOT crash
    /// or corrupt the other stats.
    #[test]
    fn malformed_rr_increments_parse_error_counter() {
        let (_data_tx, data_rx) = std::sync::mpsc::channel::<bytes::Bytes>();
        let (rtcp_tx, rtcp_rx) = std::sync::mpsc::channel::<bytes::Bytes>();
        let t = RtpRecvTransport::from_mpsc_with_rtcp(data_rx, rtcp_rx);

        // 2-byte truncated "RR" — PT byte is 201 so the dispatcher
        // attempts to decode but ReceiverReport::decode rejects on
        // length check.
        let bytes = bytes::Bytes::from(vec![0x80u8, 201]);
        rtcp_tx.send(bytes).expect("send truncated RR");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut parse_errs = 0u64;
        while std::time::Instant::now() < deadline {
            parse_errs = t.rtcp_stats().rr_parse_errors;
            if parse_errs > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(parse_errs, 1);
        // Stats unaffected.
        assert_eq!(t.rtcp_stats().rr_packets_received, 0);
    }

    /// Guard: RtpTransport::connect with port 65535 must not panic (no u16
    /// overflow in RTCP companion port arithmetic). Ok or Err both accepted.
    #[test]
    fn rtp_connect_port_65535_does_not_panic() {
        let _ = RtpTransport::connect("rtp://127.0.0.1:65535");
    }

    /// Guard: RtpRecvTransport::listen with port 65535 must not panic.
    /// The RTCP companion would need port 65536, which is invalid — the
    /// bind must be skipped, not overflowed.
    #[test]
    fn rtp_listen_port_65535_does_not_panic() {
        let _ = RtpRecvTransport::listen("rtp://127.0.0.1:65535");
    }
}
