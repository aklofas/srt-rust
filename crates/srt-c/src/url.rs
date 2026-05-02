//! Minimal `srt://host:port` URL parser. Hand-rolled to avoid pulling in
//! the `url` crate for one parse site.
//!
//! Accepted forms (v0):
//!
//! - `srt://1.2.3.4:9000`         (IPv4 + port)
//! - `srt://example.com:9000`     (DNS + port)
//! - `srt://[2001:db8::1]:9000`   (bracketed IPv6 + port)
//!
//! Rejected:
//!
//! - missing scheme, or scheme other than `srt://`
//! - missing port
//! - any query string (`?...`) — deferred to a follow-up.

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedSrtUrl {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UrlError {
    MissingScheme,
    QueryNotSupported,
    MissingPort,
    InvalidPort,
    EmptyHost,
}

impl std::fmt::Display for UrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingScheme => write!(f, "url must begin with 'srt://'"),
            Self::QueryNotSupported => write!(f, "query parameters not supported in v0"),
            Self::MissingPort => write!(f, "url must include a port"),
            Self::InvalidPort => write!(f, "port is not a valid u16"),
            Self::EmptyHost => write!(f, "host part is empty"),
        }
    }
}

#[allow(dead_code)]
pub(crate) fn parse(s: &str) -> Result<ParsedSrtUrl, UrlError> {
    let rest = s.strip_prefix("srt://").ok_or(UrlError::MissingScheme)?;

    // Reject query string.
    if rest.contains('?') {
        return Err(UrlError::QueryNotSupported);
    }

    // Bracketed IPv6 form: `[<addr>]:<port>`.
    let (host, port_str) = if let Some(rest_inner) = rest.strip_prefix('[') {
        let close = rest_inner.find(']').ok_or(UrlError::MissingPort)?;
        let host = &rest_inner[..close];
        let after = &rest_inner[close + 1..];
        let port_str = after.strip_prefix(':').ok_or(UrlError::MissingPort)?;
        (host, port_str)
    } else {
        let colon = rest.rfind(':').ok_or(UrlError::MissingPort)?;
        (&rest[..colon], &rest[colon + 1..])
    };

    if host.is_empty() {
        return Err(UrlError::EmptyHost);
    }

    let port: u16 = port_str.parse().map_err(|_| UrlError::InvalidPort)?;

    Ok(ParsedSrtUrl { host: host.to_string(), port })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_with_port() {
        let r = parse("srt://1.2.3.4:9000").unwrap();
        assert_eq!(r, ParsedSrtUrl { host: "1.2.3.4".into(), port: 9000 });
    }

    #[test]
    fn dns_with_port() {
        let r = parse("srt://example.com:9000").unwrap();
        assert_eq!(r, ParsedSrtUrl { host: "example.com".into(), port: 9000 });
    }

    #[test]
    fn bracketed_ipv6() {
        let r = parse("srt://[2001:db8::1]:9000").unwrap();
        assert_eq!(r, ParsedSrtUrl { host: "2001:db8::1".into(), port: 9000 });
    }

    #[test]
    fn rejects_missing_scheme() {
        assert_eq!(parse("1.2.3.4:9000"), Err(UrlError::MissingScheme));
    }

    #[test]
    fn rejects_query_string() {
        assert_eq!(
            parse("srt://1.2.3.4:9000?streamid=foo"),
            Err(UrlError::QueryNotSupported),
        );
    }

    #[test]
    fn rejects_missing_port() {
        assert_eq!(parse("srt://1.2.3.4"), Err(UrlError::MissingPort));
    }

    #[test]
    fn rejects_invalid_port() {
        assert_eq!(parse("srt://1.2.3.4:abc"), Err(UrlError::InvalidPort));
    }
}
