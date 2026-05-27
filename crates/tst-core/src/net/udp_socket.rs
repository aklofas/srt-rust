//! UDP socket helpers shared by `tst-rtp` and `tst-udp`.
//!
//! Provides:
//! - `bind_udp_socket` — bind + apply cancel-poll timeouts
//! - Multicast send knobs (TTL + iface, IPv4 + IPv6)
//! - Multicast recv group join (IPv4 + IPv6)
//!
//! All multicast iface/hop helpers that require raw `setsockopt` are
//! `cfg(unix)` — the non-Unix paths return
//! `io::Error::new(ErrorKind::Unsupported, …)` matching the Phase-1
//! deferral documented in the Windows-runtime-test plan.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

/// Standard cancel-poll interval used across all UDP-based transports.
/// Receivers/senders set this as the socket read/write timeout so that
/// close/cancel flags get checked at most this often during a blocking call.
pub const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Bind a UDP socket at `local` and apply the cancel-poll read/write timeouts.
pub fn bind_udp_socket(local: SocketAddr) -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind(local)?;
    socket.set_read_timeout(Some(CANCEL_POLL_INTERVAL))?;
    socket.set_write_timeout(Some(CANCEL_POLL_INTERVAL))?;
    Ok(socket)
}

/// Apply multicast SEND-side knobs (TTL + iface).
///
/// `ttl`: optional TTL for IPv4 (`IP_MULTICAST_TTL`) or hop limit for IPv6
///        (`IPV6_MULTICAST_HOPS`). `None` keeps the OS default.
/// `iface`: optional outgoing interface (literal IPv4 address; see
///          [`apply_multicast_iface`] for limitations).
///
/// IPv4 TTL uses stable `std::net::UdpSocket::set_multicast_ttl_v4`.
/// IPv6 hop limit and IPv4 `IP_MULTICAST_IF` are not exposed on stable
/// std as of Rust 1.85 — we drop to `libc::setsockopt` on Unix and
/// surface `io::ErrorKind::Unsupported` on non-Unix platforms when an
/// IPv6 mcast hop or `iface=` knob is requested.
pub fn apply_multicast_send_knobs(
    socket: &UdpSocket,
    group: IpAddr,
    ttl: Option<u8>,
    iface: Option<&str>,
) -> io::Result<()> {
    let hops = ttl.unwrap_or(MCAST_DEFAULT_TTL);
    match group {
        IpAddr::V4(_) => socket.set_multicast_ttl_v4(hops as u32)?,
        IpAddr::V6(_) => set_multicast_hops_v6(socket, hops)?,
    }
    if let Some(iface_str) = iface {
        apply_multicast_iface(socket, group, iface_str)?;
    }
    Ok(())
}

/// Default multicast TTL for sends when `?ttl=` is absent — small but
/// non-1 so single-router LAN multicast works out of the box. Matches
/// the master spec's URL defaults table.
pub const MCAST_DEFAULT_TTL: u8 = 8;

/// Set `IPV6_MULTICAST_HOPS` via raw `setsockopt`. Stable std::net does
/// not expose this in Rust 1.85 (tracking issue rust-lang/rust#92517).
#[cfg(unix)]
pub fn set_multicast_hops_v6(socket: &UdpSocket, hops: u8) -> io::Result<()> {
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
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Non-Unix fallback: report IPv6 multicast hop-limit knob as unsupported
/// in Phase 1 rather than silently ignoring `?ttl=`.
#[cfg(not(unix))]
pub fn set_multicast_hops_v6(_socket: &UdpSocket, _hops: u8) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "IPV6_MULTICAST_HOPS via raw setsockopt is Unix-only in Phase 1",
    ))
}

/// Set `IP_MULTICAST_IF` for IPv4 (interface IP) or surface a Phase-1
/// limitation for IPv6 (needs scope-id integer lookup, not yet wired).
///
/// IPv4 path accepts a literal IPv4 address string (e.g. `"192.168.1.50"`).
/// Name → IP resolution for interface names (e.g., `eth0`) is not done
/// in Phase 1 — callers needing name-based binding can resolve via
/// `if_indextoname` and pass the IP. This is the same UX libsrt's
/// `?iface=` query parameter ships with.
pub fn apply_multicast_iface(socket: &UdpSocket, group: IpAddr, iface: &str) -> io::Result<()> {
    match group {
        IpAddr::V4(_) => {
            let v4: Ipv4Addr = iface.parse().map_err(|e: std::net::AddrParseError| {
                io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!(
                        "IPv4 multicast iface requires literal IPv4 address, got '{iface}': {e}"
                    ),
                )
            })?;
            set_multicast_if_v4(socket, v4)?;
        }
        IpAddr::V6(_) => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "IPv6 multicast iface name lookup not implemented in Phase 1; \
                     pre-resolve to scope-id and use the URL form directly (iface='{iface}')"
                ),
            ));
        }
    }
    Ok(())
}

/// Set `IP_MULTICAST_IF` via raw `setsockopt`. Stable std::net does not
/// expose this in Rust 1.85 (tracking issue rust-lang/rust#92517).
#[cfg(unix)]
pub fn set_multicast_if_v4(socket: &UdpSocket, addr: Ipv4Addr) -> io::Result<()> {
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
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Non-Unix fallback: surface the iface knob as unsupported.
#[cfg(not(unix))]
pub fn set_multicast_if_v4(_socket: &UdpSocket, addr: Ipv4Addr) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("IP_MULTICAST_IF via raw setsockopt is Unix-only in Phase 1 (addr={addr})"),
    ))
}

/// Join a multicast group on a bound recv socket.
///
/// For IPv4 uses `join_multicast_v4(group, interface)`; for IPv6 uses
/// `join_multicast_v6(group, interface_index)`.
///
/// `iface` parsing matches the send-side rules in [`apply_multicast_iface`]:
/// IPv4 takes a literal IPv4 address; IPv6 is not supported in Phase 1.
pub fn apply_multicast_recv_join(
    socket: &UdpSocket,
    group: IpAddr,
    iface: Option<&str>,
) -> io::Result<()> {
    match group {
        IpAddr::V4(v4_group) => {
            let iface_v4 = match iface {
                Some(iface_str) => iface_str.parse::<Ipv4Addr>().map_err(|e| {
                    io::Error::new(
                        io::ErrorKind::Unsupported,
                        format!(
                            "IPv4 multicast iface requires literal IPv4 address, \
                                 got '{iface_str}': {e}"
                        ),
                    )
                })?,
                None => Ipv4Addr::UNSPECIFIED, // 0.0.0.0 — OS default
            };
            socket.join_multicast_v4(&v4_group, &iface_v4)?;
        }
        IpAddr::V6(v6_group) => {
            let scope_id = match iface {
                None => 0, // Default scope.
                Some(iface_str) => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        format!(
                            "IPv6 multicast iface name lookup not implemented in Phase 1; \
                             ?iface= must be omitted for ipv6 receive (iface='{iface_str}')"
                        ),
                    ));
                }
            };
            socket.join_multicast_v6(&v6_group, scope_id)?;
        }
    }
    Ok(())
}
