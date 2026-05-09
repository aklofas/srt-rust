//! `SocketAddr` ↔ `libc::sockaddr_storage` helpers.
//!
//! libsrt's bind/connect/getsockname/getpeername take `*const libc::sockaddr`.
//! We marshal between Rust's `std::net::SocketAddr` and the C representation
//! exclusively through these helpers so callers never touch raw FFI.
//!
//! Both IPv4 and IPv6 are supported — `Socket::connect_with` and
//! `Listener::bind_with` walk every address resolved by `to_socket_addrs`,
//! so AAAA records that resolve before A records on dual-stack hosts will
//! be tried first, falling through to v4 if v6 isn't routable.

use crate::error::AddrError;
use std::mem;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

/// Convert a Rust `SocketAddr` to a `libc::sockaddr_storage` plus its used length.
pub(crate) fn to_sockaddr(addr: SocketAddr) -> Result<(libc::sockaddr_storage, usize), AddrError> {
    let mut storage: libc::sockaddr_storage = unsafe { mem::zeroed() };
    match addr {
        SocketAddr::V4(v4) => {
            let sin = libc::sockaddr_in {
                sin_family: libc::AF_INET as libc::sa_family_t,
                sin_port: v4.port().to_be(),
                sin_addr: libc::in_addr {
                    s_addr: u32::from(*v4.ip()).to_be(),
                },
                sin_zero: [0; 8],
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                sin_len: mem::size_of::<libc::sockaddr_in>() as u8,
            };
            unsafe {
                std::ptr::copy_nonoverlapping(
                    (&raw const sin).cast::<u8>(),
                    (&raw mut storage).cast::<u8>(),
                    mem::size_of::<libc::sockaddr_in>(),
                );
            }
            Ok((storage, mem::size_of::<libc::sockaddr_in>()))
        }
        SocketAddr::V6(v6) => {
            let sin6 = libc::sockaddr_in6 {
                sin6_family: libc::AF_INET6 as libc::sa_family_t,
                sin6_port: v6.port().to_be(),
                sin6_flowinfo: v6.flowinfo().to_be(),
                sin6_addr: libc::in6_addr {
                    s6_addr: v6.ip().octets(),
                },
                sin6_scope_id: v6.scope_id(),
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                sin6_len: mem::size_of::<libc::sockaddr_in6>() as u8,
            };
            unsafe {
                std::ptr::copy_nonoverlapping(
                    (&raw const sin6).cast::<u8>(),
                    (&raw mut storage).cast::<u8>(),
                    mem::size_of::<libc::sockaddr_in6>(),
                );
            }
            Ok((storage, mem::size_of::<libc::sockaddr_in6>()))
        }
    }
}

/// Convert a `libc::sockaddr_storage` back to `std::net::SocketAddr`.
pub(crate) fn from_sockaddr(storage: &libc::sockaddr_storage) -> Result<SocketAddr, AddrError> {
    match storage.ss_family as i32 {
        libc::AF_INET => {
            // SAFETY: ss_family says this is a sockaddr_in.
            let v4 = unsafe { &*(storage as *const _ as *const libc::sockaddr_in) };
            let ip = Ipv4Addr::from(u32::from_be(v4.sin_addr.s_addr));
            let port = u16::from_be(v4.sin_port);
            Ok(SocketAddr::V4(SocketAddrV4::new(ip, port)))
        }
        libc::AF_INET6 => {
            // SAFETY: ss_family says this is a sockaddr_in6.
            let v6 = unsafe { &*(storage as *const _ as *const libc::sockaddr_in6) };
            let ip = Ipv6Addr::from(v6.sin6_addr.s6_addr);
            let port = u16::from_be(v6.sin6_port);
            let flowinfo = u32::from_be(v6.sin6_flowinfo);
            let scope_id = v6.sin6_scope_id;
            Ok(SocketAddr::V6(SocketAddrV6::new(
                ip, port, flowinfo, scope_id,
            )))
        }
        other => Err(AddrError::Resolve(format!(
            "unknown address family: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_v4() {
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let (storage, len) = to_sockaddr(addr).unwrap();
        assert_eq!(len, mem::size_of::<libc::sockaddr_in>());
        let back = from_sockaddr(&storage).unwrap();
        assert_eq!(addr, back);
    }

    #[test]
    fn round_trip_zero_port() {
        let addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
        let (storage, _) = to_sockaddr(addr).unwrap();
        let back = from_sockaddr(&storage).unwrap();
        assert_eq!(addr, back);
    }

    #[test]
    fn round_trip_v6_loopback() {
        let addr: SocketAddr = "[::1]:12345".parse().unwrap();
        let (storage, len) = to_sockaddr(addr).unwrap();
        assert_eq!(len, mem::size_of::<libc::sockaddr_in6>());
        assert_eq!(storage.ss_family as i32, libc::AF_INET6);
        let back = from_sockaddr(&storage).unwrap();
        assert_eq!(addr, back);
    }

    #[test]
    fn round_trip_v6_with_scope_id() {
        // SocketAddrV6 carries flowinfo + scope_id. Verify both round-trip
        // through the libc::sockaddr_in6 marshalling.
        let v6 = SocketAddrV6::new(
            std::net::Ipv6Addr::LOCALHOST,
            12345,
            /*flowinfo=*/ 0,
            /*scope_id=*/ 7,
        );
        let addr = SocketAddr::V6(v6);
        let (storage, _) = to_sockaddr(addr).unwrap();
        let back = from_sockaddr(&storage).unwrap();
        assert_eq!(addr, back);
    }

    #[test]
    fn round_trip_v6_full_address() {
        let addr: SocketAddr = "[2001:db8::1]:443".parse().unwrap();
        let (storage, _) = to_sockaddr(addr).unwrap();
        let back = from_sockaddr(&storage).unwrap();
        assert_eq!(addr, back);
    }
}
