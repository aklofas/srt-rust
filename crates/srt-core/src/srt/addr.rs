//! `SocketAddr` ↔ `libc::sockaddr_storage` helpers.
//!
//! libsrt's bind/connect/getsockname/getpeername take `*const libc::sockaddr`.
//! We marshal between Rust's `std::net::SocketAddr` and the C representation
//! exclusively through these helpers so callers never touch raw FFI.
//!
//! v0 supports IPv4 only. IPv6 is straightforward to add but isn't load-bearing
//! for current consumers (loopback in tests; well-known IPs in deployments).

use crate::error::AddrError;
use std::mem;
use std::net::{Ipv4Addr, SocketAddr};

/// Convert a Rust `SocketAddr` to a `libc::sockaddr_storage` plus its used length.
#[allow(dead_code)]
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
        SocketAddr::V6(_) => Err(AddrError::Ipv6Unsupported),
    }
}

/// Convert a `libc::sockaddr_storage` back to `std::net::SocketAddr`.
#[allow(dead_code)]
pub(crate) fn from_sockaddr(storage: &libc::sockaddr_storage) -> Result<SocketAddr, AddrError> {
    match storage.ss_family as i32 {
        libc::AF_INET => {
            // SAFETY: ss_family says this is a sockaddr_in.
            let v4 = unsafe { &*(storage as *const _ as *const libc::sockaddr_in) };
            let ip = Ipv4Addr::from(u32::from_be(v4.sin_addr.s_addr));
            let port = u16::from_be(v4.sin_port);
            Ok(SocketAddr::V4(std::net::SocketAddrV4::new(ip, port)))
        }
        libc::AF_INET6 => Err(AddrError::Ipv6Unsupported),
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
    fn v6_rejected() {
        let addr: SocketAddr = "[::1]:12345".parse().unwrap();
        assert!(matches!(to_sockaddr(addr), Err(AddrError::Ipv6Unsupported)));
    }

    #[test]
    fn round_trip_zero_port() {
        let addr: SocketAddr = "0.0.0.0:0".parse().unwrap();
        let (storage, _) = to_sockaddr(addr).unwrap();
        let back = from_sockaddr(&storage).unwrap();
        assert_eq!(addr, back);
    }
}
