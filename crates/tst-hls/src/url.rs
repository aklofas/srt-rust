//! Parsing of `hls://` URLs for binding the HLS HTTP server.
//!
//! - `hls://0.0.0.0:8080` — plain HTTP, bind on `0.0.0.0:8080`
//! - `hlss://0.0.0.0:8443?cert=server.crt&key=server.key` — HTTPS via rustls
//! - Query params: `output_dir`, `segment_duration`, `playlist_window`,
//!   `mode` (live/event/vod), `auth_user`, `auth_pass`, `cert`, `key`
//!
//! `output_dir` is optional — see [`crate::config::HlsConfig::output_dir`]
//! for the default when omitted.

use std::net::IpAddr;
use std::time::Duration;

use thiserror::Error;

use crate::config::HlsMode;

/// Parsed HLS publisher URL.
#[derive(Debug, Clone)]
pub struct HlsUrl {
    pub addr: IpAddr,
    pub port: u16,
    pub tls: bool,
    pub output_dir: Option<String>,
    pub segment_duration: Option<Duration>,
    pub playlist_window: Option<usize>,
    pub mode: Option<HlsMode>,
    pub auth_user: Option<String>,
    pub auth_pass: Option<String>,
    pub cert: Option<String>,
    pub key: Option<String>,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HlsUrlError {
    #[error("URL scheme must be 'hls' or 'hlss', got '{0}'")]
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

impl HlsUrl {
    pub fn parse(s: &str) -> Result<Self, HlsUrlError> {
        use tst_core::url::common::parse_url;

        let parsed = parse_url(s)?;
        let tls = match parsed.scheme {
            "hls" => false,
            "hlss" => true,
            other => return Err(HlsUrlError::BadScheme(other.to_string())),
        };
        let port = parsed.port.ok_or(HlsUrlError::MissingPort)?;

        let host_str = parsed.host;
        let addr: IpAddr = host_str
            .parse()
            .map_err(|_| HlsUrlError::BadHost(host_str.to_string()))?;

        let mut output_dir = None;
        let mut segment_duration = None;
        let mut playlist_window = None;
        let mut mode = None;
        let mut auth_user = None;
        let mut auth_pass = None;
        let mut cert = None;
        let mut key = None;

        for (k, v) in &parsed.query {
            let key_str = k.as_ref();
            let value = v.as_ref();
            match key_str {
                "output_dir" => output_dir = Some(value.to_string()),
                "segment_duration" => {
                    let secs: u64 =
                        tst_core::url::common::parse_int_query(value).map_err(|detail| {
                            HlsUrlError::BadQueryValue {
                                key: key_str.to_string(),
                                value: value.to_string(),
                                detail,
                            }
                        })?;
                    segment_duration = Some(Duration::from_secs(secs));
                }
                "playlist_window" => {
                    let n: usize =
                        tst_core::url::common::parse_int_query(value).map_err(|detail| {
                            HlsUrlError::BadQueryValue {
                                key: key_str.to_string(),
                                value: value.to_string(),
                                detail,
                            }
                        })?;
                    playlist_window = Some(n);
                }
                "mode" => {
                    mode = Some(match value {
                        "live" => HlsMode::Live,
                        "event" => HlsMode::Event,
                        "vod" => HlsMode::Vod,
                        other => {
                            return Err(HlsUrlError::BadQueryValue {
                                key: key_str.to_string(),
                                value: other.to_string(),
                                detail: "expected one of: live, event, vod".into(),
                            });
                        }
                    });
                }
                "auth_user" => auth_user = Some(value.to_string()),
                "auth_pass" => auth_pass = Some(value.to_string()),
                "cert" => cert = Some(value.to_string()),
                "key" => key = Some(value.to_string()),
                _ => {}
            }
        }

        Ok(Self {
            addr,
            port,
            tls,
            output_dir,
            segment_duration,
            playlist_window,
            mode,
            auth_user,
            auth_pass,
            cert,
            key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_hls() {
        let u = HlsUrl::parse("hls://0.0.0.0:8080").unwrap();
        assert!(!u.tls);
        assert_eq!(u.port, 8080);
    }

    #[test]
    fn https_with_cert_key() {
        let u = HlsUrl::parse("hlss://0.0.0.0:8443?cert=server.crt&key=server.key").unwrap();
        assert!(u.tls);
        assert_eq!(u.cert.as_deref(), Some("server.crt"));
    }

    #[test]
    fn mode_is_parsed() {
        let u = HlsUrl::parse("hls://0.0.0.0:8080?mode=event").unwrap();
        assert_eq!(u.mode, Some(HlsMode::Event));
    }

    #[test]
    fn rejects_bad_mode() {
        assert!(matches!(
            HlsUrl::parse("hls://0.0.0.0:8080?mode=garbage"),
            Err(HlsUrlError::BadQueryValue { .. })
        ));
    }

    #[test]
    fn rejects_bad_scheme() {
        assert!(matches!(
            HlsUrl::parse("hls2://host:8080"),
            Err(HlsUrlError::BadScheme(_))
        ));
    }
}
