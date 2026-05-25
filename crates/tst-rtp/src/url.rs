//! `rtp://host:port?key=value&...` URL parsing.
//!
//! Built on the scheme-neutral [`tst_core::url::common`] helpers from
//! Plan #97. Only the `rtp://` scheme is accepted here; `rtsp://` and
//! `rtsps://` are parsed by `tst-rtp`'s future `rtsp::client` module
//! (Phase 2, not yet shipped).
//!
//! Supported query keys (matches the master spec table):
//!
//! | Key | Value | Default |
//! |---|---|---|
//! | `ttl` | 1..=255 | 1 (unicast); 8 (multicast send) |
//! | `iface` | interface name (`eth0`) or IPv4 literal | OS default |
//! | `pkt_size` | positive multiple of 188 | 1316 |
//! | `ssrc` | u32 decimal or `0x`-prefixed hex | random |
//!
//! `ttl` is wire-format-shared between IPv4 (`IP_MULTICAST_TTL`) and
//! IPv6 (`IPV6_MULTICAST_HOPS`). The transport applies the right
//! setsockopt based on the destination address family.

use std::net::IpAddr;

use thiserror::Error;
use tst_core::url::common::{ParsedUrl, UrlError as CoreUrlError, parse_url};

/// Default UDP payload size: 7 MPEG-TS packets per RTP packet = 1316 B
/// (RFC 2250 §2 mandates an integral multiple of 188; 7×188 matches
/// libsrt live-mode payload, and the SocketStats / transport `max_payload`
/// shape we cascade through `Transport`).
pub const DEFAULT_PKT_SIZE: usize = 1316;

/// Maximum value we accept for `ttl` / `pkt_size` etc; just `u8::MAX`
/// for now.
const MAX_TTL: u32 = 255;

/// MPEG-TS packet size (RFC 2250 §2).
const TS_PACKET_SIZE: usize = 188;

/// Parsed `rtp://` URL. Construction validates the scheme and query keys
/// but does NOT touch the network — interface names, host resolvability,
/// and bind permissions are checked by the `transport` layer at
/// `connect` / `listen` time.
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpUrl {
    /// Destination/source host as written in the URL (literal IP or
    /// domain). Parsed to `IpAddr` lazily by the transport when needed.
    pub host: String,
    /// Port.
    pub port: u16,
    /// `?ttl=N` query key, or `None` to use the default.
    pub ttl: Option<u8>,
    /// `?iface=eth0` or `?iface=192.168.1.50` — empty when absent.
    pub iface: Option<String>,
    /// `?pkt_size=N` — bytes per UDP datagram (188-multiple).
    pub pkt_size: usize,
    /// `?ssrc=N` — explicit SSRC; `None` means "randomize at construct".
    pub ssrc: Option<u32>,
}

/// Errors specific to parsing the `rtp://` URL form.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum UrlError {
    /// Structural URL parse failure (delegated to `tst_core::url`).
    #[error(transparent)]
    Syntax(#[from] CoreUrlError),
    /// Scheme was not `rtp` (e.g., user passed an `srt://` URL by
    /// mistake). `rtsp://` / `rtsps://` rejected here too — they are
    /// parsed by a separate Phase 2 module.
    #[error("expected 'rtp' scheme, got '{got}'")]
    WrongScheme { got: String },
    /// `rtp://` requires a port. We do not pick a default.
    #[error("rtp:// URL requires :port")]
    MissingPort,
    /// `?ttl=` failed validation (out of `1..=255` range or non-numeric).
    #[error("invalid ttl '{got}': {detail}")]
    BadTtl { got: String, detail: String },
    /// `?pkt_size=` not a positive multiple of 188.
    #[error("invalid pkt_size '{got}': {detail}")]
    BadPktSize { got: String, detail: String },
    /// `?ssrc=` couldn't be parsed as decimal or `0x`-prefixed hex.
    #[error("invalid ssrc '{got}': {detail}")]
    BadSsrc { got: String, detail: String },
    /// `?iface=` value couldn't be interpreted as either a non-empty
    /// interface name or a literal IPv4 address.
    #[error("invalid iface '{got}'")]
    BadIface { got: String },
    /// Query key wasn't recognized. (The full set is documented in the
    /// module rustdoc table.) We reject unknown keys hard so typos don't
    /// silently get ignored.
    #[error("unknown rtp:// URL query key '{got}'")]
    UnknownKey { got: String },
}

impl RtpUrl {
    /// Parse `rtp://host:port?key=value&...`.
    ///
    /// On success, all fields are validated independently — the
    /// transport layer doesn't need to re-check ranges. The `host`
    /// field is preserved as-written (no DNS resolution at this stage).
    pub fn parse(s: &str) -> Result<Self, UrlError> {
        let ParsedUrl {
            scheme,
            host,
            port,
            query,
            ..
        } = parse_url(s)?;
        if !scheme.eq_ignore_ascii_case("rtp") {
            return Err(UrlError::WrongScheme {
                got: scheme.to_string(),
            });
        }
        let port = port.ok_or(UrlError::MissingPort)?;
        let mut ttl = None;
        let mut iface = None;
        let mut pkt_size = DEFAULT_PKT_SIZE;
        let mut ssrc = None;
        for (k, v) in query.iter() {
            match k.as_ref() {
                "ttl" => ttl = Some(parse_ttl(v.as_ref())?),
                "iface" => iface = Some(parse_iface(v.as_ref())?),
                "pkt_size" => pkt_size = parse_pkt_size(v.as_ref())?,
                "ssrc" => ssrc = Some(parse_ssrc(v.as_ref())?),
                other => {
                    return Err(UrlError::UnknownKey {
                        got: other.to_string(),
                    });
                }
            }
        }
        Ok(Self {
            host: host.to_string(),
            port,
            ttl,
            iface,
            pkt_size,
            ssrc,
        })
    }
}

fn parse_ttl(v: &str) -> Result<u8, UrlError> {
    let n: u32 = v
        .parse()
        .map_err(|e: std::num::ParseIntError| UrlError::BadTtl {
            got: v.to_string(),
            detail: e.to_string(),
        })?;
    if !(1..=MAX_TTL).contains(&n) {
        return Err(UrlError::BadTtl {
            got: v.to_string(),
            detail: format!("must be in 1..={MAX_TTL}"),
        });
    }
    Ok(n as u8)
}

fn parse_pkt_size(v: &str) -> Result<usize, UrlError> {
    let n: usize = v
        .parse()
        .map_err(|e: std::num::ParseIntError| UrlError::BadPktSize {
            got: v.to_string(),
            detail: e.to_string(),
        })?;
    if n == 0 || n % TS_PACKET_SIZE != 0 {
        return Err(UrlError::BadPktSize {
            got: v.to_string(),
            detail: format!("must be a positive multiple of {TS_PACKET_SIZE}"),
        });
    }
    Ok(n)
}

fn parse_ssrc(v: &str) -> Result<u32, UrlError> {
    let (radix, body) = if let Some(rest) = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        (16, rest)
    } else {
        (10, v)
    };
    u32::from_str_radix(body, radix).map_err(|e| UrlError::BadSsrc {
        got: v.to_string(),
        detail: e.to_string(),
    })
}

fn parse_iface(v: &str) -> Result<String, UrlError> {
    if v.is_empty() {
        return Err(UrlError::BadIface { got: v.to_string() });
    }
    // Interface name OR literal IPv4 (libsrt-style: `?iface=192.168.1.50`).
    // We don't restrict ASCII here; OS interface names can be Unicode on
    // some platforms.
    if v.parse::<IpAddr>().is_ok() || !v.chars().any(|c| c.is_whitespace()) {
        Ok(v.to_string())
    } else {
        Err(UrlError::BadIface { got: v.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_unicast_url() {
        let u = RtpUrl::parse("rtp://10.0.0.5:5004").unwrap();
        assert_eq!(u.host, "10.0.0.5");
        assert_eq!(u.port, 5004);
        assert!(u.ttl.is_none());
        assert!(u.iface.is_none());
        assert_eq!(u.pkt_size, DEFAULT_PKT_SIZE);
        assert!(u.ssrc.is_none());
    }

    #[test]
    fn parse_multicast_url_with_ttl_iface() {
        let u = RtpUrl::parse("rtp://239.10.10.1:5004?ttl=8&iface=eth0").unwrap();
        assert_eq!(u.host, "239.10.10.1");
        assert_eq!(u.ttl, Some(8));
        assert_eq!(u.iface, Some("eth0".to_string()));
    }

    #[test]
    fn parse_url_with_pkt_size_and_ssrc() {
        let u = RtpUrl::parse("rtp://239.10.10.1:5004?pkt_size=1316&ssrc=0xCAFEBABE").unwrap();
        assert_eq!(u.pkt_size, 1316);
        assert_eq!(u.ssrc, Some(0xCAFE_BABE));
    }

    #[test]
    fn parse_ssrc_decimal() {
        let u = RtpUrl::parse("rtp://h:5004?ssrc=4242").unwrap();
        assert_eq!(u.ssrc, Some(4242));
    }

    #[test]
    fn rejects_non_rtp_scheme() {
        let err = RtpUrl::parse("srt://h:9000").unwrap_err();
        assert!(matches!(err, UrlError::WrongScheme { .. }));
    }

    #[test]
    fn rejects_rtsp_scheme() {
        // RTSP is a separate Phase 2 surface, not parsed by RtpUrl.
        let err = RtpUrl::parse("rtsp://camera.lan/path").unwrap_err();
        assert!(matches!(err, UrlError::WrongScheme { .. }));
    }

    #[test]
    fn rejects_missing_port() {
        let err = RtpUrl::parse("rtp://239.10.10.1").unwrap_err();
        assert!(matches!(err, UrlError::MissingPort));
    }

    #[test]
    fn rejects_zero_ttl() {
        let err = RtpUrl::parse("rtp://h:5004?ttl=0").unwrap_err();
        assert!(matches!(err, UrlError::BadTtl { .. }));
    }

    #[test]
    fn rejects_oob_ttl() {
        let err = RtpUrl::parse("rtp://h:5004?ttl=256").unwrap_err();
        assert!(matches!(err, UrlError::BadTtl { .. }));
    }

    #[test]
    fn rejects_pkt_size_not_188_multiple() {
        let err = RtpUrl::parse("rtp://h:5004?pkt_size=1500").unwrap_err();
        assert!(matches!(err, UrlError::BadPktSize { .. }));
    }

    #[test]
    fn rejects_pkt_size_zero() {
        let err = RtpUrl::parse("rtp://h:5004?pkt_size=0").unwrap_err();
        assert!(matches!(err, UrlError::BadPktSize { .. }));
    }

    #[test]
    fn rejects_unknown_query_key() {
        // `latency` makes sense for srt://; for rtp:// it's a typo.
        let err = RtpUrl::parse("rtp://h:5004?latency=200").unwrap_err();
        assert!(matches!(err, UrlError::UnknownKey { .. }));
    }

    #[test]
    fn rejects_bad_ssrc() {
        let err = RtpUrl::parse("rtp://h:5004?ssrc=notanint").unwrap_err();
        assert!(matches!(err, UrlError::BadSsrc { .. }));
    }
}
