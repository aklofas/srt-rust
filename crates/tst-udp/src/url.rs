//! Parsing of `udp://...` URLs.
//!
//! The format mirrors what ffmpeg + VLC accept:
//!
//! - `udp://host:port` — unicast send/recv
//! - `udp://@group:port` — multicast recv (the `@` prefix is the ffmpeg convention)
//! - `udp://group:port` (group in 224.0.0.0/4 or ff00::/8) — multicast send
//!
//! Query parameters: `iface`, `ttl`, `tos`, `rcvbuf`, `sndbuf`, `pkt_size`, `localaddr`.

use std::net::IpAddr;

use thiserror::Error;

/// Parsed UDP URL.
#[derive(Debug, Clone)]
pub struct UdpUrl {
    /// Destination address (for send) or bind address (for recv).
    pub addr: IpAddr,
    /// Port (always required).
    pub port: u16,
    /// Whether the URL had an `@` prefix → "this is a receive-side bind".
    pub recv_bind: bool,
    /// Multicast outgoing interface (literal IPv4/IPv6 addr or interface name).
    pub iface: Option<String>,
    /// IPv4 multicast TTL or IPv6 hop limit.
    pub ttl: Option<u8>,
    /// IP TOS / DSCP byte.
    pub tos: Option<u8>,
    /// SO_RCVBUF size in bytes.
    pub rcvbuf: Option<usize>,
    /// SO_SNDBUF size in bytes.
    pub sndbuf: Option<usize>,
    /// Send-side datagram size; receiver-side buffer hint.
    pub pkt_size: Option<usize>,
    /// Local bind address override (for sending from a specific NIC).
    pub localaddr: Option<IpAddr>,
}

/// Errors from `UdpUrl::parse`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum UdpUrlError {
    #[error("URL scheme must be 'udp', got '{0}'")]
    BadScheme(String),
    #[error("URL must include a port")]
    MissingPort,
    #[error("host '{0}' is not a literal IPv4/IPv6 address")]
    BadHost(String),
    #[error("query param '{key}' has invalid value '{value}': {detail}")]
    BadQueryValue {
        key: String,
        value: String,
        detail: String,
    },
    #[error("URL parse failed: {0}")]
    Parse(#[from] tst_core::url::common::UrlError),
}

impl UdpUrl {
    /// Parse a `udp://...` URL into the structured form.
    pub fn parse(s: &str) -> Result<Self, UdpUrlError> {
        use tst_core::url::common::parse_url;

        let parsed = parse_url(s)?;
        if parsed.scheme != "udp" {
            return Err(UdpUrlError::BadScheme(parsed.scheme.to_string()));
        }
        let port = parsed.port.ok_or(UdpUrlError::MissingPort)?;

        // ffmpeg `@` prefix on host means "bind on this address (recv-side)".
        // `parse_url` treats `@` as the userinfo separator, so `udp://@host:port`
        // arrives here as `username = Some("")` + `host = "host"`. The presence
        // of a (possibly empty) username signals recv-bind intent.
        let recv_bind = parsed.username.is_some();
        let host_str = parsed.host;

        // Strip IPv6 brackets if present.
        let host_str = host_str.trim_start_matches('[').trim_end_matches(']');

        let addr: IpAddr = host_str
            .parse()
            .map_err(|_| UdpUrlError::BadHost(host_str.to_string()))?;

        let mut iface = None;
        let mut ttl = None;
        let mut tos = None;
        let mut rcvbuf = None;
        let mut sndbuf = None;
        let mut pkt_size = None;
        let mut localaddr = None;

        for (k, v) in &parsed.query {
            let key = k.as_ref();
            let value = v.as_ref();
            match key {
                "iface" => iface = Some(value.to_string()),
                "ttl" => ttl = Some(parse_u8_dec(key, value)?),
                "tos" => tos = Some(parse_u8_hex_or_dec(key, value)?),
                "rcvbuf" => rcvbuf = Some(parse_byte_size(key, value)?),
                "sndbuf" => sndbuf = Some(parse_byte_size(key, value)?),
                "pkt_size" => pkt_size = Some(parse_byte_size(key, value)?),
                "localaddr" => {
                    let a: IpAddr = value.parse().map_err(|e: std::net::AddrParseError| {
                        UdpUrlError::BadQueryValue {
                            key: key.to_string(),
                            value: value.to_string(),
                            detail: e.to_string(),
                        }
                    })?;
                    localaddr = Some(a);
                }
                _ => { /* unknown params silently ignored — matches ffmpeg behavior */ }
            }
        }

        Ok(Self {
            addr,
            port,
            recv_bind,
            iface,
            ttl,
            tos,
            rcvbuf,
            sndbuf,
            pkt_size,
            localaddr,
        })
    }

    /// True if `addr` is in 224.0.0.0/4 or ff00::/8.
    pub fn is_multicast(&self) -> bool {
        match self.addr {
            IpAddr::V4(v4) => v4.is_multicast(),
            IpAddr::V6(v6) => v6.is_multicast(),
        }
    }
}

fn parse_u8_dec(key: &str, value: &str) -> Result<u8, UdpUrlError> {
    value.parse().map_err(|e: std::num::ParseIntError| UdpUrlError::BadQueryValue {
        key: key.to_string(),
        value: value.to_string(),
        detail: e.to_string(),
    })
}

fn parse_u8_hex_or_dec(key: &str, value: &str) -> Result<u8, UdpUrlError> {
    let v = if let Some(hex) = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) {
        u8::from_str_radix(hex, 16)
    } else {
        value.parse::<u8>()
    };
    v.map_err(|e| UdpUrlError::BadQueryValue {
        key: key.to_string(),
        value: value.to_string(),
        detail: e.to_string(),
    })
}

fn parse_byte_size(key: &str, value: &str) -> Result<usize, UdpUrlError> {
    // Accept "12345", "12K", "12k", "12M", "12m"; matches ffmpeg.
    let (num, mul) = match value.chars().last() {
        Some('K') | Some('k') => (&value[..value.len() - 1], 1024usize),
        Some('M') | Some('m') => (&value[..value.len() - 1], 1024 * 1024),
        _ => (value, 1usize),
    };
    let n: usize = num.parse().map_err(|e: std::num::ParseIntError| {
        UdpUrlError::BadQueryValue {
            key: key.to_string(),
            value: value.to_string(),
            detail: e.to_string(),
        }
    })?;
    Ok(n * mul)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicast_send() {
        let u = UdpUrl::parse("udp://192.168.1.5:5004").unwrap();
        assert!(!u.recv_bind);
        assert!(!u.is_multicast());
        assert_eq!(u.port, 5004);
    }

    #[test]
    fn multicast_send_v4() {
        let u = UdpUrl::parse("udp://239.10.0.1:5004").unwrap();
        assert!(!u.recv_bind);
        assert!(u.is_multicast());
        assert_eq!(u.port, 5004);
    }

    #[test]
    fn multicast_recv_v4_with_at_prefix() {
        let u = UdpUrl::parse("udp://@239.10.0.1:5004").unwrap();
        assert!(u.recv_bind);
        assert!(u.is_multicast());
    }

    #[test]
    fn unicast_recv_with_at_prefix() {
        let u = UdpUrl::parse("udp://@0.0.0.0:5004").unwrap();
        assert!(u.recv_bind);
        assert!(!u.is_multicast());
    }

    #[test]
    fn multicast_v6_with_query() {
        let u = UdpUrl::parse("udp://[ff0e::1]:5004?iface=eth0&ttl=8").unwrap();
        assert!(u.is_multicast());
        assert_eq!(u.iface.as_deref(), Some("eth0"));
        assert_eq!(u.ttl, Some(8));
    }

    #[test]
    fn knobs_pkt_size_tos_bufs_hex_suffix() {
        let u = UdpUrl::parse(
            "udp://239.10.0.1:5004?pkt_size=1316&tos=0xb8&sndbuf=2M&rcvbuf=8M",
        )
        .unwrap();
        assert_eq!(u.pkt_size, Some(1316));
        assert_eq!(u.tos, Some(0xb8));
        assert_eq!(u.sndbuf, Some(2 * 1024 * 1024));
        assert_eq!(u.rcvbuf, Some(8 * 1024 * 1024));
    }

    #[test]
    fn rejects_bad_scheme() {
        assert!(matches!(
            UdpUrl::parse("https://host:5004"),
            Err(UdpUrlError::BadScheme(_))
        ));
    }

    #[test]
    fn rejects_missing_port() {
        assert!(matches!(
            UdpUrl::parse("udp://239.10.0.1"),
            Err(UdpUrlError::MissingPort)
        ));
    }
}
