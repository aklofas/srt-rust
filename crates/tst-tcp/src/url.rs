//! Parsing of `tcp://` and `tcps://` URLs.
//!
//! - `tcp://host:port` — plain TCP caller
//! - `tcps://host:port` — TLS caller
//! - `tcp://0.0.0.0:port?listen=1` — listener (plain)
//! - `tcps://0.0.0.0:port?listen=1&cert=...&key=...` — listener (TLS)
//!
//! # IP-literal requirement for `tcps://`
//!
//! The parser accepts only IPv4 or IPv6 literals, never hostnames. For TLS
//! (`tcps://`), this means the TLS handshake presents the IP address as the
//! server name. The server certificate must carry a matching `iPAddress`
//! SubjectAltName (SAN); a `dnsName` SAN — even one that resolves to the same
//! IP — will cause the handshake to fail with a certificate error. Generate
//! a certificate with an IP SAN using:
//!
//! ```bash
//! openssl req -x509 -nodes -newkey rsa:2048 -subj "/CN=server" \
//!   -addext "subjectAltName=IP:192.168.1.10" -out server.crt -keyout server.key
//! ```

use std::net::IpAddr;
use std::time::Duration;

use thiserror::Error;

/// Parsed TCP URL.
#[derive(Debug, Clone)]
pub struct TcpUrl {
    /// Destination address (for caller) or bind address (for listener).
    pub addr: IpAddr,
    /// Port.
    pub port: u16,
    /// True for `tcps://`.
    pub tls: bool,
    /// True if the URL had `?listen=1` (listener intent).
    pub listen: bool,
    /// TCP_NODELAY override.
    pub nodelay: Option<bool>,
    /// SO_KEEPALIVE idle time (None = disabled, Some(d) = enabled with idle d).
    pub keepalive: Option<Duration>,
    /// SO_RCVBUF size.
    pub rcvbuf: Option<usize>,
    /// SO_SNDBUF size.
    pub sndbuf: Option<usize>,
    /// Connect timeout (caller only).
    pub connect_timeout: Option<Duration>,
    /// Server cert path (TLS listener only).
    pub cert: Option<String>,
    /// Server key path (TLS listener only).
    pub key: Option<String>,
    /// Custom CA bundle path (TLS caller only; if None uses native roots).
    pub ca: Option<String>,
}

/// Errors from `TcpUrl::parse`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TcpUrlError {
    #[error("URL scheme must be 'tcp' or 'tcps', got '{0}'")]
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

impl TcpUrl {
    pub fn parse(s: &str) -> Result<Self, TcpUrlError> {
        use tst_core::url::common::parse_url;

        let parsed = parse_url(s)?;
        let tls = match parsed.scheme {
            "tcp" => false,
            "tcps" => true,
            other => return Err(TcpUrlError::BadScheme(other.to_string())),
        };
        let port = parsed.port.ok_or(TcpUrlError::MissingPort)?;

        let host_str = parsed.host;
        let addr: IpAddr = host_str
            .parse()
            .map_err(|_| TcpUrlError::BadHost(host_str.to_string()))?;

        let mut listen = false;
        let mut nodelay = None;
        let mut keepalive = None;
        let mut rcvbuf = None;
        let mut sndbuf = None;
        let mut connect_timeout = None;
        let mut cert = None;
        let mut key = None;
        let mut ca = None;

        for (k, v) in &parsed.query {
            let key_str = k.as_ref();
            let value = v.as_ref();
            match key_str {
                "listen" => listen = parse_bool(key_str, value)?,
                "nodelay" => nodelay = Some(parse_bool(key_str, value)?),
                "keepalive" => keepalive = Some(parse_duration_secs(key_str, value)?),
                "rcvbuf" => rcvbuf = Some(parse_byte_size(key_str, value)?),
                "sndbuf" => sndbuf = Some(parse_byte_size(key_str, value)?),
                "connect_timeout" => connect_timeout = Some(parse_duration_secs(key_str, value)?),
                "cert" => cert = Some(value.to_string()),
                "key" => key = Some(value.to_string()),
                "ca" => ca = Some(value.to_string()),
                _ => { /* ignore unknown params */ }
            }
        }

        Ok(Self {
            addr,
            port,
            tls,
            listen,
            nodelay,
            keepalive,
            rcvbuf,
            sndbuf,
            connect_timeout,
            cert,
            key,
            ca,
        })
    }
}

fn parse_bool(key: &str, value: &str) -> Result<bool, TcpUrlError> {
    tst_core::url::common::parse_bool_query(value).map_err(|detail| TcpUrlError::BadQueryValue {
        key: key.to_string(),
        value: value.to_string(),
        detail,
    })
}

fn parse_duration_secs(key: &str, value: &str) -> Result<Duration, TcpUrlError> {
    let secs: u64 = tst_core::url::common::parse_int_query(value)
        .map_err(|detail| TcpUrlError::BadQueryValue {
            key: key.to_string(),
            value: value.to_string(),
            detail,
        })?;
    Ok(Duration::from_secs(secs))
}

fn parse_byte_size(key: &str, value: &str) -> Result<usize, TcpUrlError> {
    tst_core::url::common::parse_byte_size(value).map_err(|detail| TcpUrlError::BadQueryValue {
        key: key.to_string(),
        value: value.to_string(),
        detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_caller() {
        let u = TcpUrl::parse("tcp://192.168.1.5:7001").unwrap();
        assert!(!u.tls);
        assert!(!u.listen);
        assert_eq!(u.port, 7001);
    }

    #[test]
    fn tls_caller() {
        let u = TcpUrl::parse("tcps://192.168.1.5:7001").unwrap();
        assert!(u.tls);
        assert!(!u.listen);
    }

    #[test]
    fn plain_listener() {
        let u = TcpUrl::parse("tcp://0.0.0.0:7001?listen=1").unwrap();
        assert!(!u.tls);
        assert!(u.listen);
    }

    #[test]
    fn tls_listener_with_cert_key() {
        let u =
            TcpUrl::parse("tcps://0.0.0.0:7001?listen=1&cert=server.crt&key=server.key").unwrap();
        assert!(u.tls);
        assert!(u.listen);
        assert_eq!(u.cert.as_deref(), Some("server.crt"));
        assert_eq!(u.key.as_deref(), Some("server.key"));
    }

    #[test]
    fn byte_size_suffix_overflow_is_rejected_not_panic() {
        // The "M" multiply must not panic in debug or wrap in release.
        let err = TcpUrl::parse("tcp://1.2.3.4:7001?rcvbuf=999999999999999999999M")
            .expect_err("overflowing rcvbuf must be rejected");
        assert!(matches!(err, TcpUrlError::BadQueryValue { .. }));
    }

    #[test]
    fn byte_size_enormous_but_valid_is_rejected_by_bound() {
        // 999999M ≈ 1 TiB: valid digits + recognized suffix, but absurd.
        let err = TcpUrl::parse("tcp://1.2.3.4:7001?sndbuf=999999M")
            .expect_err("absurd sndbuf must exceed the byte-size ceiling");
        match err {
            TcpUrlError::BadQueryValue { detail, .. } => {
                assert!(detail.contains("exceeds maximum"), "detail: {detail}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn byte_size_at_ceiling_is_accepted() {
        let u = TcpUrl::parse("tcp://1.2.3.4:7001?rcvbuf=256M").unwrap();
        assert_eq!(u.rcvbuf, Some(256 * 1024 * 1024));
    }

    #[test]
    fn cited_adversarial_input_returns_err() {
        // The exact adversarial fixture: `999999999999G`. Rejected (never
        // panic/wrap).
        assert!(TcpUrl::parse("tcp://1.2.3.4:7001?rcvbuf=999999999999G").is_err());
    }

    #[test]
    fn rejects_bad_scheme() {
        assert!(matches!(
            TcpUrl::parse("udp://host:7001"),
            Err(TcpUrlError::BadScheme(_))
        ));
    }

    #[test]
    fn rejects_missing_port() {
        assert!(matches!(
            TcpUrl::parse("tcp://1.2.3.4"),
            Err(TcpUrlError::MissingPort)
        ));
    }

    #[test]
    fn knobs_parse() {
        let u = TcpUrl::parse(
            "tcp://1.2.3.4:7001?nodelay=1&keepalive=30&rcvbuf=8M&sndbuf=2M&connect_timeout=10",
        )
        .unwrap();
        assert_eq!(u.nodelay, Some(true));
        assert_eq!(u.keepalive, Some(Duration::from_secs(30)));
        assert_eq!(u.rcvbuf, Some(8 * 1024 * 1024));
        assert_eq!(u.sndbuf, Some(2 * 1024 * 1024));
        assert_eq!(u.connect_timeout, Some(Duration::from_secs(10)));
    }
}
