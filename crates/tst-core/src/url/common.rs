//! Scheme-neutral URL parsing for `srt://`, `rtp://`, `rtsp://`, `rtsps://`.
//!
//! See [the module docs](super) for the URL shape we accept.

use std::borrow::Cow;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use thiserror::Error;

/// A parsed URL with no scheme-specific interpretation applied.
///
/// Lifetime-borrows from the input string for the small fields; the query
/// vector owns its key/value pairs because percent-decoding may produce
/// new strings.
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUrl<'a> {
    /// Scheme as written in the URL (e.g., `"srt"`, `"rtp"`, `"rtsp"`,
    /// `"rtsps"`). Returned verbatim — the parser does NOT case-fold;
    /// callers that need case-insensitive comparison should use
    /// `eq_ignore_ascii_case` or canonicalize at their layer.
    pub scheme: &'a str,
    /// `user` from `user[:password]@host` — None when no userinfo present.
    pub username: Option<&'a str>,
    /// `password` from `user:password@host` — None when no `:` in userinfo.
    pub password: Option<&'a str>,
    /// Host, with IPv6 brackets stripped (`::1` not `[::1]`).
    pub host: &'a str,
    /// Port — None when the URL omits it (`scheme://host/path`).
    pub port: Option<u16>,
    /// Path, including the leading `/`. Empty string when the URL has no path.
    pub path: &'a str,
    /// Query pairs in URL order. Last-occurrence wins is the caller's
    /// responsibility. Values are percent-decoded.
    pub query: Vec<(Cow<'a, str>, Cow<'a, str>)>,
}

/// Errors that may arise from [`parse_url`] / [`parse_host_port`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum UrlError {
    /// `://` separator missing.
    #[error("URL missing '://' separator")]
    MissingSchemeSeparator,
    /// Scheme component is empty (e.g., `"://host"`).
    #[error("URL has empty scheme before '://'")]
    EmptyScheme,
    /// Port couldn't be parsed as a u16.
    #[error("invalid port '{got}': {detail}")]
    InvalidPort { got: String, detail: String },
    /// IPv6 host opened with `[` but never closed with `]`.
    #[error("URL has '[' but no matching ']' in host")]
    UnclosedIpv6Bracket,
    /// IPv6 literal had unexpected content after the closing `]`.
    /// E.g. `[::1]garbage` rather than `[::1]:port` or `[::1]`.
    #[error("malformed IPv6 literal: unexpected '{got}' after ']'")]
    MalformedIpv6Literal { got: String },
    /// A `%XY` percent-escape was malformed (non-hex, truncated).
    #[error("malformed percent-encoding in URL: {detail}")]
    BadPercentEncoding { detail: String },
    /// Host string is empty in a context where it must be present
    /// (caller's responsibility to call this; `parse_url` itself accepts
    /// empty hosts and leaves the policy to the caller).
    #[error("URL must include a host")]
    MissingHost,
    /// Port is absent in a context where it must be present (e.g.
    /// [`parse_host_port`] requires a port). Distinct from
    /// [`Self::InvalidPort`] which fires when a port string is present
    /// but unparseable.
    #[error("URL must include a port")]
    MissingPort,
}

/// Parse a URL of shape `scheme://[user[:password]@]host[:port][/path][?query]`.
///
/// The function performs structural splitting only — it does NOT validate
/// that the scheme is one we support, or that any query keys are recognized.
/// Callers (per-transport-crate URL parsers) layer their own scheme-acceptance
/// + key recognition on top.
///
/// Path and host are returned verbatim (callers parse host into IP address
/// via [`parse_host_port`] when needed). Query values are percent-decoded;
/// path and host strings are returned verbatim.
pub fn parse_url(s: &str) -> Result<ParsedUrl<'_>, UrlError> {
    // Split scheme from rest at first `://`.
    let sep = s.find("://").ok_or(UrlError::MissingSchemeSeparator)?;
    let scheme = &s[..sep];
    if scheme.is_empty() {
        return Err(UrlError::EmptyScheme);
    }
    let rest = &s[sep + 3..];

    // Split off path (first `/` outside any userinfo `@`) and query (first `?`).
    let (authority_with_userinfo, path, query_raw) = split_path_query(rest);

    // Split userinfo from authority on `@`.
    let (userinfo_opt, host_port) = match authority_with_userinfo.rfind('@') {
        Some(at) => (
            Some(&authority_with_userinfo[..at]),
            &authority_with_userinfo[at + 1..],
        ),
        None => (None, authority_with_userinfo),
    };
    let (username, password) = match userinfo_opt {
        None => (None, None),
        Some(u) => match u.find(':') {
            Some(c) => (Some(&u[..c]), Some(&u[c + 1..])),
            None => (Some(u), None),
        },
    };

    // Split host[:port], handling IPv6 brackets.
    let (host, port) = split_host_port(host_port)?;

    let query = match query_raw {
        Some(q) => parse_query(q)?,
        None => Vec::new(),
    };

    Ok(ParsedUrl {
        scheme,
        username,
        password,
        host,
        port,
        path,
        query,
    })
}

/// Split `authority[/path][?query]` into the three components. Path
/// component includes the leading `/`. Query is the substring after `?`,
/// not yet decoded.
fn split_path_query(rest: &str) -> (&str, &str, Option<&str>) {
    // Find `?` first so that a `/` inside a query value is not mistaken
    // for the path separator. Per RFC 3986 §3, query starts at the first
    // `?` after authority.
    let (pre_query, query) = match rest.find('?') {
        Some(q) => (&rest[..q], Some(&rest[q + 1..])),
        None => (rest, None),
    };
    let (authority, path) = match pre_query.find('/') {
        Some(p) => (&pre_query[..p], &pre_query[p..]),
        None => (pre_query, ""),
    };
    (authority, path, query)
}

/// Split `host[:port]` into host (without IPv6 brackets) and optional port.
fn split_host_port(s: &str) -> Result<(&str, Option<u16>), UrlError> {
    if let Some(rest) = s.strip_prefix('[') {
        // IPv6 literal: `[v6]:port` or `[v6]`.
        let close = rest.find(']').ok_or(UrlError::UnclosedIpv6Bracket)?;
        let host = &rest[..close];
        let after = &rest[close + 1..];
        let port = if let Some(p) = after.strip_prefix(':') {
            Some(parse_port(p)?)
        } else if after.is_empty() {
            None
        } else {
            return Err(UrlError::MalformedIpv6Literal {
                got: after.to_string(),
            });
        };
        Ok((host, port))
    } else {
        // IPv4 or domain — last `:` is the port separator.
        match s.rfind(':') {
            Some(c) => Ok((&s[..c], Some(parse_port(&s[c + 1..])?))),
            None => Ok((s, None)),
        }
    }
}

/// Parse a port from a decimal string. Returns [`UrlError::InvalidPort`]
/// on parse failure or out-of-range value.
fn parse_port(s: &str) -> Result<u16, UrlError> {
    s.parse::<u16>().map_err(|e| UrlError::InvalidPort {
        got: s.to_string(),
        detail: e.to_string(),
    })
}

/// A single decoded query pair: `(key, value)`.
type QueryPair<'a> = (Cow<'a, str>, Cow<'a, str>);

/// Parse `key=value&key=value` into a vector of decoded pairs.
/// Order is preserved (caller decides last-wins / first-wins semantics).
fn parse_query(q: &str) -> Result<Vec<QueryPair<'_>>, UrlError> {
    let mut out = Vec::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = match pair.find('=') {
            Some(i) => (&pair[..i], &pair[i + 1..]),
            None => (pair, ""),
        };
        out.push((percent_decode(k)?, percent_decode(v)?));
    }
    Ok(out)
}

/// Percent-decode a string. Returns `Cow::Borrowed` when no `%` appears
/// (the fast path); otherwise allocates. Invalid `%XY` sequences return
/// `Err(UrlError::BadPercentEncoding)`.
fn percent_decode(s: &str) -> Result<Cow<'_, str>, UrlError> {
    if !s.contains('%') {
        return Ok(Cow::Borrowed(s));
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(UrlError::BadPercentEncoding {
                    detail: format!(
                        "percent escape at offset {i} in '{s}' needs exactly 2 hex digits"
                    ),
                });
            }
            let h = hex_nibble(bytes[i + 1])?;
            let l = hex_nibble(bytes[i + 2])?;
            out.push((h << 4) | l);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out)
        .map(Cow::Owned)
        .map_err(|e| UrlError::BadPercentEncoding {
            detail: format!("percent-decoded bytes in '{s}' are not valid UTF-8: {e}"),
        })
}

fn hex_nibble(b: u8) -> Result<u8, UrlError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(UrlError::BadPercentEncoding {
            detail: format!("non-hex char in escape: '{}' (byte 0x{b:02x})", b as char),
        }),
    }
}

/// Parse a literal `IP:port` pair. The IP must be a literal IPv4 or
/// bracketed IPv6 address — domain names return [`UrlError::InvalidPort`]
/// because DNS resolution is the caller's responsibility.
///
/// Examples:
/// - `192.168.1.10:5004` → `([192.168.1.10], 5004)`
/// - `[2001:db8::1]:5004` → `([2001:db8::1], 5004)`
pub fn parse_host_port(s: &str) -> Result<(IpAddr, u16), UrlError> {
    let (host, port) = split_host_port(s)?;
    let port = port.ok_or(UrlError::MissingPort)?;
    let ip: IpAddr = host
        .parse()
        .map_err(|e: std::net::AddrParseError| UrlError::InvalidPort {
            got: host.to_string(),
            detail: format!("expected literal IPv4 or IPv6 address, got '{host}': {e}"),
        })?;
    Ok((ip, port))
}

/// IPv4 multicast: `224.0.0.0/4` (RFC 5771). Delegates to
/// [`Ipv4Addr::is_multicast`] from `std`; named function exposed to
/// match the `is_multicast_v6` shape for callers that work generically
/// over IP family.
#[must_use]
pub fn is_multicast_v4(addr: Ipv4Addr) -> bool {
    addr.is_multicast()
}

/// IPv6 multicast: `ff00::/8` (RFC 4291 §2.7). Delegates to
/// [`Ipv6Addr::is_multicast`] from `std`.
#[must_use]
pub fn is_multicast_v6(addr: Ipv6Addr) -> bool {
    addr.is_multicast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_url_scheme_host_port() {
        let u = parse_url("srt://example.com:9000").unwrap();
        assert_eq!(u.scheme, "srt");
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, Some(9000));
        assert_eq!(u.username, None);
        assert_eq!(u.password, None);
        assert_eq!(u.path, "");
        assert!(u.query.is_empty());
    }

    #[test]
    fn parse_url_no_port() {
        let u = parse_url("rtsp://camera.lan").unwrap();
        assert_eq!(u.scheme, "rtsp");
        assert_eq!(u.host, "camera.lan");
        assert_eq!(u.port, None);
    }

    #[test]
    fn parse_url_missing_separator_rejected() {
        let err = parse_url("srt:host:9000").unwrap_err();
        assert!(matches!(err, UrlError::MissingSchemeSeparator));
    }

    #[test]
    fn parse_url_empty_scheme_rejected() {
        let err = parse_url("://host:9000").unwrap_err();
        assert!(matches!(err, UrlError::EmptyScheme));
    }

    #[test]
    fn parse_url_invalid_port_rejected() {
        let err = parse_url("srt://host:99999").unwrap_err();
        assert!(matches!(err, UrlError::InvalidPort { .. }));
    }

    #[test]
    fn parse_url_with_userinfo() {
        let u = parse_url("rtsp://alice:secret@cam.lan:554/h264").unwrap();
        assert_eq!(u.username, Some("alice"));
        assert_eq!(u.password, Some("secret"));
        assert_eq!(u.host, "cam.lan");
        assert_eq!(u.port, Some(554));
        assert_eq!(u.path, "/h264");
    }

    #[test]
    fn parse_url_with_username_only() {
        let u = parse_url("rtsp://alice@cam.lan/h264").unwrap();
        assert_eq!(u.username, Some("alice"));
        assert_eq!(u.password, None);
    }

    #[test]
    fn parse_url_path_only() {
        let u = parse_url("rtsp://cam.lan:554/main/sub").unwrap();
        assert_eq!(u.path, "/main/sub");
    }

    #[test]
    fn parse_url_ipv6_bracketed() {
        let u = parse_url("rtp://[2001:db8::1]:5004/").unwrap();
        assert_eq!(u.host, "2001:db8::1");
        assert_eq!(u.port, Some(5004));
        assert_eq!(u.path, "/");
    }

    #[test]
    fn parse_url_ipv6_no_port() {
        let u = parse_url("rtp://[::1]").unwrap();
        assert_eq!(u.host, "::1");
        assert_eq!(u.port, None);
    }

    #[test]
    fn parse_url_ipv6_unclosed_bracket_rejected() {
        let err = parse_url("rtp://[::1:5004").unwrap_err();
        assert!(matches!(err, UrlError::UnclosedIpv6Bracket));
    }

    #[test]
    fn parse_url_query_pairs() {
        let u = parse_url("srt://h:9000?streamid=front&latency=200").unwrap();
        assert_eq!(u.query.len(), 2);
        assert_eq!(u.query[0].0.as_ref(), "streamid");
        assert_eq!(u.query[0].1.as_ref(), "front");
        assert_eq!(u.query[1].0.as_ref(), "latency");
        assert_eq!(u.query[1].1.as_ref(), "200");
    }

    #[test]
    fn parse_url_query_value_percent_decoded() {
        // `%20` → space, `%3D` → `=` inside a value.
        let u = parse_url("srt://h:9000?passphrase=hello%20world%3Dx").unwrap();
        assert_eq!(u.query[0].0.as_ref(), "passphrase");
        assert_eq!(u.query[0].1.as_ref(), "hello world=x");
    }

    #[test]
    fn parse_url_query_value_without_eq() {
        // `?flag&latency=200` — flag has no `=`, value is empty.
        let u = parse_url("srt://h:9000?flag&latency=200").unwrap();
        assert_eq!(u.query[0].0.as_ref(), "flag");
        assert_eq!(u.query[0].1.as_ref(), "");
        assert_eq!(u.query[1].0.as_ref(), "latency");
        assert_eq!(u.query[1].1.as_ref(), "200");
    }

    #[test]
    fn parse_url_query_bad_percent_rejected() {
        let err = parse_url("srt://h:9000?key=%ZZ").unwrap_err();
        assert!(matches!(err, UrlError::BadPercentEncoding { .. }));
    }

    #[test]
    fn parse_url_query_truncated_percent_rejected() {
        let err = parse_url("srt://h:9000?key=%2").unwrap_err();
        assert!(matches!(err, UrlError::BadPercentEncoding { .. }));
    }

    #[test]
    fn parse_url_query_value_non_utf8_rejected() {
        // %FE and %FF are valid hex but never start a valid UTF-8 sequence.
        let err = parse_url("srt://h:9000?key=%FE%FF").unwrap_err();
        assert!(matches!(err, UrlError::BadPercentEncoding { .. }));
    }

    #[test]
    fn parse_host_port_ipv4() {
        let (ip, port) = parse_host_port("192.168.1.10:5004").unwrap();
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)));
        assert_eq!(port, 5004);
    }

    #[test]
    fn parse_host_port_ipv6() {
        let (ip, port) = parse_host_port("[2001:db8::1]:5004").unwrap();
        assert_eq!(ip, IpAddr::V6("2001:db8::1".parse().unwrap()));
        assert_eq!(port, 5004);
    }

    #[test]
    fn parse_host_port_rejects_non_literal_host() {
        // parse_host_port requires a literal IP; domain names need DNS
        // resolution which is the caller's responsibility.
        let err = parse_host_port("cam.lan:554").unwrap_err();
        assert!(matches!(err, UrlError::InvalidPort { .. }));
    }

    #[test]
    fn parse_host_port_rejects_bare_ip_without_port() {
        // `parse_host_port` requires both host AND port. A bare IPv4 string
        // without `:port` should yield MissingPort (not InvalidPort).
        let err = parse_host_port("192.168.1.10").unwrap_err();
        assert!(matches!(err, UrlError::MissingPort));
    }

    #[test]
    fn is_multicast_v4_classifies_correctly() {
        assert!(is_multicast_v4(Ipv4Addr::new(239, 0, 0, 1)));
        assert!(is_multicast_v4(Ipv4Addr::new(224, 0, 0, 1)));
        assert!(!is_multicast_v4(Ipv4Addr::new(192, 168, 1, 1)));
        assert!(!is_multicast_v4(Ipv4Addr::new(240, 0, 0, 1))); // class E
    }

    #[test]
    fn is_multicast_v6_classifies_correctly() {
        assert!(is_multicast_v6("ff00::1".parse().unwrap()));
        assert!(is_multicast_v6("ff05::1".parse().unwrap()));
        assert!(!is_multicast_v6("2001:db8::1".parse().unwrap()));
        assert!(!is_multicast_v6("::1".parse().unwrap()));
    }

    #[test]
    fn parse_url_ipv6_trailing_garbage_rejected() {
        let err = parse_url("rtp://[::1]garbage").unwrap_err();
        assert!(matches!(err, UrlError::MalformedIpv6Literal { .. }));
    }
}
