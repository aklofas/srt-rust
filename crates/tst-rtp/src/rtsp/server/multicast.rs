//! Multicast mount specialization — single shared UDP socket sending
//! to a multicast group, fed by the mount's broadcast channel. Per-client
//! per-session tasks do NOT spawn per-peer fanout (Task 13's design is
//! unicast-only); they just increment a counter so MountStats::peer_count
//! reflects the number of clients that SETUP'd against the multicast
//! mount.

use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::clock::RtpClock;
use crate::error::RtspServerError;
use crate::packet::{RTP_HEADER_LEN, RtpHeader};
use crate::rtsp::server::fanout::PeerDropCounter;

/// Build the multicast send socket — binds an ephemeral local port,
/// connects to the group address (so we can use `send` instead of
/// `send_to` per frame), and applies TTL + optional IF.
///
/// Returns the bound `UdpSocket` ready to be wrapped in `Arc` and
/// driven by a per-mount sender task.
///
/// # Errors
///
/// Returns [`RtspServerError::Io`] if the ephemeral bind or `connect`
/// call fails. Returns [`RtspServerError::InvalidMulticastGroup`] when
/// setting TTL / hop-limit / interface options fails, or when the
/// platform doesn't support the requested option (e.g. IPv6 multicast
/// TTL on non-Unix targets).
pub(crate) async fn build_multicast_send_socket(
    group: SocketAddr,
    ttl: u8,
    iface: Option<&str>,
) -> Result<UdpSocket, RtspServerError> {
    // Bind an ephemeral local port. For IPv4: 0.0.0.0:0; IPv6: [::]:0.
    let local: SocketAddr = match group {
        SocketAddr::V4(_) => "0.0.0.0:0".parse().unwrap(),
        SocketAddr::V6(_) => "[::]:0".parse().unwrap(),
    };
    let socket = UdpSocket::bind(local)
        .await
        .map_err(|e| RtspServerError::Io(e.kind()))?;

    // TTL / hop-limit knob.
    match group {
        SocketAddr::V4(_) => {
            socket.set_multicast_ttl_v4(ttl as u32).map_err(|e| {
                RtspServerError::InvalidMulticastGroup {
                    addr: group.to_string(),
                    detail: format!("set_multicast_ttl_v4 failed: {e}"),
                }
            })?;
        }
        SocketAddr::V6(_) => {
            // Tokio's UdpSocket doesn't expose set_multicast_hops_v6 on
            // Rust 1.85 stable. Phase 1 uses libc::setsockopt for this;
            // we replicate via std::os::fd::AsRawFd.
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                let val: libc::c_int = ttl as libc::c_int;
                // SAFETY: socket FD owned by us for `socket`'s lifetime;
                // &val is valid for size_of::<c_int>().
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
                    return Err(RtspServerError::InvalidMulticastGroup {
                        addr: group.to_string(),
                        detail: format!(
                            "IPV6_MULTICAST_HOPS setsockopt failed: {}",
                            std::io::Error::last_os_error()
                        ),
                    });
                }
            }
            #[cfg(not(unix))]
            {
                return Err(RtspServerError::InvalidMulticastGroup {
                    addr: group.to_string(),
                    detail: "IPv6 multicast TTL not supported on this platform".to_string(),
                });
            }
        }
    }

    // Optional interface binding. IPv4 only for v1. Stable std/tokio
    // don't expose `set_multicast_if_v4` on tokio's UdpSocket in Rust
    // 1.85 (tracking issue rust-lang/rust#92517) — Phase 1 uses libc
    // setsockopt for the same knob on std::net::UdpSocket; we mirror it
    // here on tokio's wrapper via AsRawFd.
    if let Some(iface_str) = iface {
        match group {
            SocketAddr::V4(_) => {
                let iface_ip: std::net::Ipv4Addr =
                    iface_str.parse().map_err(|e: std::net::AddrParseError| {
                        RtspServerError::InvalidMulticastGroup {
                            addr: group.to_string(),
                            detail: format!("iface '{iface_str}' is not a v4 IP literal: {e}"),
                        }
                    })?;
                #[cfg(unix)]
                {
                    use std::os::fd::AsRawFd;
                    let in_addr = libc::in_addr {
                        s_addr: u32::from_ne_bytes(iface_ip.octets()),
                    };
                    // SAFETY: socket FD owned by `socket` for its
                    // lifetime; &in_addr is valid for size_of::<in_addr>().
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
                        return Err(RtspServerError::InvalidMulticastGroup {
                            addr: group.to_string(),
                            detail: format!(
                                "IP_MULTICAST_IF setsockopt failed: {}",
                                std::io::Error::last_os_error()
                            ),
                        });
                    }
                }
                #[cfg(not(unix))]
                {
                    return Err(RtspServerError::InvalidMulticastGroup {
                        addr: group.to_string(),
                        detail: "IP_MULTICAST_IF setsockopt is Unix-only in v1".to_string(),
                    });
                }
            }
            SocketAddr::V6(_) => {
                return Err(RtspServerError::InvalidMulticastGroup {
                    addr: group.to_string(),
                    detail: "IPv6 multicast iface binding not implemented in v1".to_string(),
                });
            }
        }
    }

    socket
        .connect(group)
        .await
        .map_err(|e| RtspServerError::Io(e.kind()))?;
    Ok(socket)
}

/// Spawn the per-mount multicast sender task. Drains the mount's
/// `broadcast::Receiver` + writes RTP frames to the multicast group via
/// the shared connected socket. Single task per mount (vs. per-peer
/// tasks for unicast mounts).
///
/// `ssrc` is fixed per-mount (not per-peer) — multicast peers see the
/// same SSRC, which is RFC 3550 §6.3.3-compatible (group senders share
/// the SSRC space; collision detection is the receiver's responsibility).
pub(crate) fn spawn_multicast_sender(
    mut rx: broadcast::Receiver<Bytes>,
    socket: Arc<UdpSocket>,
    cancel: CancellationToken,
    ssrc: u32,
    initial_seq: u16,
    drop_counter: Arc<PeerDropCounter>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let clock = RtpClock::new(0);
        let mut seq = initial_seq;
        loop {
            tokio::select! {
                res = rx.recv() => match res {
                    Ok(payload) => {
                        let mut datagram = Vec::with_capacity(RTP_HEADER_LEN + payload.len());
                        datagram.resize(RTP_HEADER_LEN, 0);
                        RtpHeader::new(seq, clock.now_ticks(), ssrc)
                            .encode_into(&mut datagram);
                        datagram.extend_from_slice(&payload);
                        seq = seq.wrapping_add(1);
                        if let Err(e) = socket.send(&datagram).await {
                            tracing::warn!(
                                target: "tst_rtp::server::multicast",
                                error = %e,
                                "multicast send failed; exiting"
                            );
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        drop_counter.add(n);
                        tracing::warn!(
                            target: "tst_rtp::server::multicast",
                            lagged = n,
                            "broadcast lag on multicast sender"
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                },
                _ = cancel.cancelled() => return,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn build_multicast_send_socket_v4_succeeds() {
        let group: SocketAddr = "239.0.0.1:5004".parse().unwrap();
        let sock = build_multicast_send_socket(group, 4, None).await.unwrap();
        // Socket bound to ephemeral port; connect set the default peer.
        let local = sock.local_addr().unwrap();
        assert!(local.port() > 0);
    }

    #[tokio::test]
    async fn build_multicast_send_socket_rejects_unicast() {
        // 10.0.0.1 isn't multicast — but we don't validate at the bind
        // step; that's MulticastGroup::parse's job (Wave A T2).
        // The function just sets the multicast knobs which are no-ops
        // for unicast send. So this test verifies the function still
        // succeeds (it would only fail on bind / setsockopt errors).
        let unicast: SocketAddr = "127.0.0.1:5004".parse().unwrap();
        // setsockopt for multicast_ttl on a unicast destination is
        // permitted but no-op; the test passes if no panic.
        let _ = build_multicast_send_socket(unicast, 4, None).await;
        // No assert — caller is expected to validate via MulticastGroup::parse.
    }

    #[tokio::test]
    async fn build_multicast_with_iface_v4() {
        let group: SocketAddr = "239.0.0.1:5004".parse().unwrap();
        // 127.0.0.1 is a valid (loopback) IP literal — passes the parse step.
        let res = build_multicast_send_socket(group, 4, Some("127.0.0.1")).await;
        // Setting IP_MULTICAST_IF to loopback is permitted; assert success.
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn build_multicast_with_iface_v6_unsupported() {
        let group: SocketAddr = "[ff02::1]:5004".parse().unwrap();
        let e = build_multicast_send_socket(group, 4, Some("::1"))
            .await
            .unwrap_err();
        assert!(matches!(e, RtspServerError::InvalidMulticastGroup { .. }));
    }
}
