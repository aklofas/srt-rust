//! `srt://host:port?key=value&...` URL parser.
//!
//! Vocabulary: libsrt-URL keys (strict — recognizes libsrt's key set,
//! rejects any key for which this library doesn't yet expose a builder
//! setter, with a clear "known but unsupported" error).
//!
//! Plus two `srt-c`-flavor extensions: `x-recvtimeout` / `x-sendtimeout`
//! (no libsrt-URL precedent; `SRTO_RCVTIMEO` / `SRTO_SNDTIMEO`).
//!
//! Spec: `docs/specs/2026-05-02-srt-c-url-query-params-design.md`.

use crate::error::OptionError;
use crate::srt::config::{ListenerConfig, SocketConfig};
use crate::srt::options::{
    Congestion, KeyLength, MaxBandwidth, PacketFilter, Passphrase, StreamId,
};
use std::time::Duration;

/// Parsed `srt://host:port?...` URL: connection target + a typed overlay
/// of the recognized query parameters.
#[derive(Debug)]
pub struct SrtUrl {
    pub host: String,
    pub port: u16,
    pub overlay: UrlOverlay,
}

/// Typed overlay of query-parameter values. Apply via `apply_to_socket`
/// or `apply_to_listener`. URL wins on conflict (Q4-A precedence rule).
#[derive(Debug, Default, Clone)]
#[allow(dead_code)] // Fields populated by Tasks 3–7; applied to socket in Task 8.
pub struct UrlOverlay {
    // Group 1 — libsrt-URL honored keys.
    pub passphrase: Option<Passphrase>,
    pub(crate) key_length: Option<KeyLength>,
    pub(crate) latency: Option<Duration>,
    pub(crate) recv_latency: Option<Duration>,
    pub(crate) peer_latency: Option<Duration>,
    pub(crate) mss: Option<u16>,
    pub(crate) payload_size: Option<u16>,
    pub(crate) max_bandwidth: Option<MaxBandwidth>,
    pub(crate) input_bandwidth: Option<u64>,
    pub(crate) overhead_bandwidth_pct: Option<u8>,
    pub stream_id: Option<StreamId>,
    pub(crate) loss_max_ttl: Option<u32>,
    pub(crate) too_late_packet_drop: Option<bool>,
    pub(crate) flow_window_packets: Option<u32>,
    pub packet_filter: Option<PacketFilter>,
    pub congestion: Option<Congestion>,

    // Group 2 — `srt-c` extension keys.
    pub(crate) recv_timeout: Option<Duration>,
    pub(crate) send_timeout: Option<Duration>,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UrlError {
    #[error("URL parse failed: {0}")]
    Syntax(#[from] url::ParseError),

    #[error("scheme must be 'srt', got '{got}'")]
    WrongScheme { got: String },

    #[error("URL must include port (e.g. srt://host:9000), got no port")]
    MissingPort,

    #[error("URL must include host")]
    MissingHost,

    #[error("userinfo (user:pass@) is not supported in SRT URLs; use ?passphrase=... instead")]
    UserinfoNotSupported,

    #[error("unsupported mode '{mode}'; only mode=caller is accepted")]
    UnsupportedMode { mode: String },

    #[error(
        "option '{key}' (libsrt {srto}) is recognized but not yet exposed by this library; see deferred-features.md"
    )]
    UnsupportedKey { key: String, srto: &'static str },

    #[error("unknown URL key '{key}'")]
    UnknownKey { key: String },

    #[error("invalid value for '{key}': {detail}")]
    InvalidValue { key: String, detail: String },

    #[error("option validation failed for '{key}': {source}")]
    OptionValidation {
        key: String,
        #[source]
        source: OptionError,
    },
}

impl SrtUrl {
    /// Parse `srt://host:port?key=value&...` into validated parts.
    pub fn parse(s: &str) -> Result<Self, UrlError> {
        let parsed = url::Url::parse(s).map_err(|e| {
            // url::ParseError::EmptyHost means the URL had a bare ":" after the
            // authority separator with no host — surface as MissingHost rather
            // than the opaque Syntax variant.
            if e == url::ParseError::EmptyHost {
                UrlError::MissingHost
            } else {
                UrlError::Syntax(e)
            }
        })?;
        if parsed.scheme() != "srt" {
            return Err(UrlError::WrongScheme {
                got: parsed.scheme().to_string(),
            });
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(UrlError::UserinfoNotSupported);
        }
        // Use parsed.host() (the enum) rather than host_str() so that IPv6
        // addresses come back without brackets. host_str() preserves the
        // bracketed form "[::1]"; Ipv6Addr::to_string() gives "::1".
        let host = match parsed.host() {
            Some(url::Host::Ipv4(addr)) => addr.to_string(),
            Some(url::Host::Ipv6(addr)) => addr.to_string(),
            Some(url::Host::Domain(d)) if !d.is_empty() => d.to_string(),
            _ => return Err(UrlError::MissingHost),
        };
        let port = parsed.port().ok_or(UrlError::MissingPort)?;

        let mut overlay = UrlOverlay::default();
        // url::Url::query_pairs() URL-decodes values automatically.
        // Last-occurrence wins (Q4-A): we just overwrite as we go.
        for (key, value) in parsed.query_pairs() {
            apply_query_pair(&mut overlay, &key, &value)?;
        }

        Ok(Self {
            host,
            port,
            overlay,
        })
    }
}

fn apply_query_pair(overlay: &mut UrlOverlay, key: &str, value: &str) -> Result<(), UrlError> {
    match key {
        "passphrase" => {
            overlay.passphrase = Some(Passphrase::new(value.to_string()).map_err(|e| {
                UrlError::OptionValidation {
                    key: "passphrase".into(),
                    source: OptionError::from(e),
                }
            })?);
        }
        "streamid" => {
            overlay.stream_id =
                Some(
                    StreamId::new(value.to_string()).map_err(|e| UrlError::OptionValidation {
                        key: "streamid".into(),
                        source: OptionError::from(e),
                    })?,
                );
        }
        "packetfilter" => {
            overlay.packet_filter = Some(PacketFilter::new(value.to_string()).map_err(|e| {
                UrlError::OptionValidation {
                    key: "packetfilter".into(),
                    source: OptionError::from(e),
                }
            })?);
        }
        "congestion" => {
            overlay.congestion = Some(Congestion::from_str_strict(value).map_err(|source| {
                UrlError::OptionValidation {
                    key: "congestion".into(),
                    source,
                }
            })?);
        }
        // INT / INT64 keys land in Task 4.
        // BOOL keys land in Task 5.
        // Group 2 (x-*) and Group 3 (rejects) land in Tasks 6 and 7.
        // For now, anything unrecognized is UnknownKey.
        other => {
            return Err(UrlError::UnknownKey {
                key: other.to_string(),
            });
        }
    }
    Ok(())
}

impl UrlOverlay {
    /// Write `Some(_)` fields through to `cfg`. URL wins on conflict.
    pub fn apply_to_socket(&self, _cfg: &mut SocketConfig) {
        unimplemented!("see Task 8")
    }

    /// Same shape for `ListenerConfig` (for symmetry with future
    /// listener-side URL support; v1 has no listener-side _open in srt-c).
    pub fn apply_to_listener(&self, _cfg: &mut ListenerConfig) {
        unimplemented!("see Task 8")
    }
}
