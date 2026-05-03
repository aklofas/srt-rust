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

/// `?latency=N` is parsed as N milliseconds (libsrt-URL canonical), but
/// ffmpeg's URL parses it as microseconds. A user copying an ffmpeg URL
/// gets 1000x too much receiver buffer with no error. This threshold
/// (10s) is well above any realistic live-streaming latency, so any
/// value at or above it is almost certainly a unit-misunderstanding.
const SUSPICIOUS_LATENCY_MS: i32 = 10_000;

fn warn_if_suspicious_latency(key: &str, ms: i32) {
    if ms >= SUSPICIOUS_LATENCY_MS {
        tracing::warn!(
            url_key = key,
            value_ms = ms,
            "SRT URL '{}={}' parses to {}ms ({}s) of buffer - \
             unusually high. Note: ffmpeg uses microseconds while srt-rust \
             (libsrt-URL canonical) uses milliseconds. If you copied this \
             from an ffmpeg URL, divide by 1000.",
            key,
            ms,
            ms,
            ms / 1000,
        );
    }
}

/// Group 3 (spec §4.3): libsrt-URL keys we recognize but don't yet expose.
/// Each entry maps the URL key to its `SRTO_*` name for error messages.
const GROUP3_REJECTED: &[(&str, &str)] = &[
    ("bindtodevice", "SRTO_BINDTODEVICE"),
    ("cryptomode", "SRTO_CRYPTOMODE"),
    ("drifttracer", "SRTO_DRIFTTRACER"),
    ("enforcedencryption", "SRTO_ENFORCEDENCRYPTION"),
    ("groupconnect", "SRTO_GROUPCONNECT"),
    ("groupminstabletimeo", "SRTO_GROUPMINSTABLETIMEO"),
    ("iptos", "SRTO_IPTOS"),
    ("ipttl", "SRTO_IPTTL"),
    ("ipv6only", "SRTO_IPV6ONLY"),
    ("kmpreannounce", "SRTO_KMPREANNOUNCE"),
    ("kmrefreshrate", "SRTO_KMREFRESHRATE"),
    ("maxrexmitbw", "SRTO_MAXREXMITBW"),
    ("messageapi", "SRTO_MESSAGEAPI"),
    ("mininputbw", "SRTO_MININPUTBW"),
    ("minversion", "SRTO_MINVERSION"),
    ("nakreport", "SRTO_NAKREPORT"),
    ("peeridletimeo", "SRTO_PEERIDLETIMEO"),
    ("rcvbuf", "SRTO_RCVBUF"),
    ("retransmitalgo", "SRTO_RETRANSMITALGO"),
    ("sndbuf", "SRTO_SNDBUF"),
    ("snddropdelay", "SRTO_SNDDROPDELAY"),
    ("transtype", "SRTO_TRANSTYPE"),
    ("tsbpdmode", "SRTO_TSBPDMODE"),
];

fn group3_lookup(key: &str) -> Option<&'static str> {
    GROUP3_REJECTED
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, srto)| *srto)
}

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
pub struct UrlOverlay {
    // Group 1 — libsrt-URL honored keys.
    pub passphrase: Option<Passphrase>,
    pub key_length: Option<KeyLength>,
    pub latency: Option<Duration>,
    pub recv_latency: Option<Duration>,
    pub peer_latency: Option<Duration>,
    pub mss: Option<u16>,
    pub payload_size: Option<u16>,
    pub max_bandwidth: Option<MaxBandwidth>,
    pub input_bandwidth: Option<u64>,
    pub overhead_bandwidth_pct: Option<u8>,
    pub stream_id: Option<StreamId>,
    pub loss_max_ttl: Option<u32>,
    pub too_late_packet_drop: Option<bool>,
    pub flow_window_packets: Option<u32>,
    pub packet_filter: Option<PacketFilter>,
    pub congestion: Option<Congestion>,

    // Group 2 — `srt-c` extension keys.
    pub recv_timeout: Option<Duration>,
    pub send_timeout: Option<Duration>,

    // Group 1 — connect timeout. Honored from libsrt URL vocabulary
    // (`conntimeo`); also accepts ffmpeg-style alias `connect_timeout`.
    pub connect_timeout: Option<Duration>,

    // Group 1 — `SRTO_LINGER` close-grace period (seconds, matching ffmpeg).
    pub linger: Option<Duration>,

    // Group 1 — kernel UDP socket buffer sizes. Honored from libsrt URL
    // vocabulary (`udprcvbuf`/`udpsndbuf`); also accepts ffmpeg-style
    // aliases `recv_buffer_size`/`send_buffer_size`.
    pub udp_recv_buffer_bytes: Option<u32>,
    pub udp_send_buffer_bytes: Option<u32>,
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
    ///
    /// Strict ASCII canonical forms (decimal-only integers, `0`/`1` for
    /// bools, lowercase enums); the `url` crate URL-decodes percent
    /// sequences. Last-occurrence wins on duplicate keys.
    ///
    /// # Example
    ///
    /// ```
    /// use srt_core::SrtUrl;
    /// use std::time::Duration;
    ///
    /// let u = SrtUrl::parse(
    ///     "srt://camera.local:9000?streamid=front&latency=200",
    /// ).unwrap();
    /// assert_eq!(u.host, "camera.local");
    /// assert_eq!(u.port, 9000);
    /// assert_eq!(u.overlay.latency, Some(Duration::from_millis(200)));
    /// ```
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

fn parse_int_nonneg<T>(key: &str, value: &str) -> Result<T, UrlError>
where
    T: std::str::FromStr<Err = std::num::ParseIntError>,
{
    // Strict-A: bare decimal, no suffix, non-negative. We rely on T's
    // from_str to enforce range. Use this for unsigned T (u8/u16/u32/u64);
    // signed callers go through parse_i32_nonneg, which adds the sign check.
    value.parse::<T>().map_err(|e| UrlError::InvalidValue {
        key: key.to_string(),
        detail: format!("expected non-negative decimal integer in range, got '{value}': {e}"),
    })
}

fn parse_i32_nonneg(key: &str, value: &str) -> Result<i32, UrlError> {
    let n: i32 = value.parse().map_err(|e| UrlError::InvalidValue {
        key: key.to_string(),
        detail: format!("expected i32 decimal, got '{value}': {e}"),
    })?;
    if n < 0 {
        return Err(UrlError::InvalidValue {
            key: key.to_string(),
            detail: format!("must be non-negative, got {n}"),
        });
    }
    Ok(n)
}

fn parse_oheadbw(value: &str) -> Result<u8, UrlError> {
    let n: u32 = parse_int_nonneg("oheadbw", value)?;
    if !(5..=100).contains(&n) {
        return Err(UrlError::InvalidValue {
            key: "oheadbw".into(),
            detail: format!("must be in 5..=100, got {n}"),
        });
    }
    Ok(n as u8)
}

fn parse_bool_strict(key: &str, value: &str) -> Result<bool, UrlError> {
    match value {
        "0" => Ok(false),
        "1" => Ok(true),
        other => Err(UrlError::InvalidValue {
            key: key.to_string(),
            detail: format!("expected '0' or '1', got '{other}'"),
        }),
    }
}

fn apply_query_pair(overlay: &mut UrlOverlay, key: &str, value: &str) -> Result<(), UrlError> {
    match key {
        "congestion" | "smoother" => {
            // libsrt-URL canonical key is `congestion` (renamed from `smoother`
            // in libsrt 1.4.1); `smoother` is the pre-rename ffmpeg-style alias.
            overlay.congestion = Some(Congestion::from_str_strict(value).map_err(|source| {
                UrlError::OptionValidation {
                    key: "congestion".into(),
                    source,
                }
            })?);
        }
        "conntimeo" | "connect_timeout" => {
            // libsrt-URL canonical key is `conntimeo` (milliseconds);
            // `connect_timeout` is the ffmpeg-style alias.
            let n = parse_i32_nonneg("conntimeo", value)?;
            overlay.connect_timeout = Some(Duration::from_millis(n as u64));
        }
        "fc" | "ffs" => {
            // libsrt-URL canonical key is `fc` (flow control window);
            // `ffs` is the ffmpeg-style alias (flight flag size).
            overlay.flow_window_packets = Some(parse_int_nonneg("fc", value)?);
        }
        "inputbw" => {
            overlay.input_bandwidth = Some(parse_int_nonneg("inputbw", value)?);
        }
        "latency" | "tsbpddelay" => {
            // libsrt-URL canonical key is `latency`; `tsbpddelay` is the
            // ffmpeg-style alias (the `SRTO_*` constant for SRT's TSBPD
            // mechanism is `SRTO_LATENCY`).
            let n = parse_i32_nonneg("latency", value)?;
            warn_if_suspicious_latency("latency", n);
            // n is a non-negative i32; widening to u64 is lossless.
            overlay.latency = Some(Duration::from_millis(n as u64));
        }
        "linger" => {
            // SRTO_LINGER value is in seconds (matches ffmpeg's URL).
            let n = parse_i32_nonneg("linger", value)?;
            overlay.linger = Some(Duration::from_secs(n as u64));
        }
        "lossmaxttl" => {
            overlay.loss_max_ttl = Some(parse_int_nonneg("lossmaxttl", value)?);
        }
        "maxbw" => {
            // SRTO_MAXBW is i64; we expose non-negative as Limited(u64).
            // Negative-sentinel forms (Auto/Infinite) are not URL-settable
            // under strict-A.
            let n = parse_int_nonneg::<u64>("maxbw", value)?;
            overlay.max_bandwidth = Some(MaxBandwidth::Limited(n));
        }
        "mss" => {
            overlay.mss = Some(parse_int_nonneg::<u16>("mss", value)?);
        }
        "oheadbw" => {
            overlay.overhead_bandwidth_pct = Some(parse_oheadbw(value)?);
        }
        "packetfilter" => {
            overlay.packet_filter = Some(PacketFilter::new(value.to_string()).map_err(|e| {
                UrlError::OptionValidation {
                    key: "packetfilter".into(),
                    source: OptionError::from(e),
                }
            })?);
        }
        "passphrase" => {
            overlay.passphrase = Some(Passphrase::new(value.to_string()).map_err(|e| {
                UrlError::OptionValidation {
                    key: "passphrase".into(),
                    source: OptionError::from(e),
                }
            })?);
        }
        "payloadsize" | "pkt_size" | "payload_size" => {
            // libsrt-URL canonical key is `payloadsize`; `pkt_size` and
            // `payload_size` are ffmpeg-style aliases.
            overlay.payload_size = Some(parse_int_nonneg::<u16>("payloadsize", value)?);
        }
        "pbkeylen" => {
            let n = parse_i32_nonneg("pbkeylen", value)?;
            overlay.key_length =
                Some(
                    KeyLength::from_bytes(n).map_err(|source| UrlError::OptionValidation {
                        key: "pbkeylen".into(),
                        source,
                    })?,
                );
        }
        "peerlatency" => {
            let n = parse_i32_nonneg("peerlatency", value)?;
            warn_if_suspicious_latency("peerlatency", n);
            overlay.peer_latency = Some(Duration::from_millis(n as u64));
        }
        "rcvlatency" => {
            let n = parse_i32_nonneg("rcvlatency", value)?;
            warn_if_suspicious_latency("rcvlatency", n);
            overlay.recv_latency = Some(Duration::from_millis(n as u64));
        }
        "streamid" | "srt_streamid" => {
            // libsrt-URL canonical key is `streamid`; `srt_streamid` is the
            // ffmpeg-style alias.
            overlay.stream_id =
                Some(
                    StreamId::new(value.to_string()).map_err(|e| UrlError::OptionValidation {
                        key: "streamid".into(),
                        source: OptionError::from(e),
                    })?,
                );
        }
        "tlpktdrop" => {
            overlay.too_late_packet_drop = Some(parse_bool_strict("tlpktdrop", value)?);
        }
        "udprcvbuf" | "recv_buffer_size" => {
            overlay.udp_recv_buffer_bytes = Some(parse_int_nonneg("udprcvbuf", value)?);
        }
        "udpsndbuf" | "send_buffer_size" => {
            overlay.udp_send_buffer_bytes = Some(parse_int_nonneg("udpsndbuf", value)?);
        }
        "x-recvtimeout" => {
            let n = parse_i32_nonneg("x-recvtimeout", value)?;
            overlay.recv_timeout = Some(Duration::from_millis(n as u64));
        }
        "x-sendtimeout" => {
            let n = parse_i32_nonneg("x-sendtimeout", value)?;
            overlay.send_timeout = Some(Duration::from_millis(n as u64));
        }
        "mode" => match value {
            "caller" => { /* no-op */ }
            other => {
                return Err(UrlError::UnsupportedMode {
                    mode: other.to_string(),
                });
            }
        },
        other => {
            if let Some(srto) = group3_lookup(other) {
                return Err(UrlError::UnsupportedKey {
                    key: other.to_string(),
                    srto,
                });
            }
            return Err(UrlError::UnknownKey {
                key: other.to_string(),
            });
        }
    }
    Ok(())
}

impl UrlOverlay {
    /// Write `Some(_)` fields through to `cfg`. URL wins on conflict.
    pub fn apply_to_socket(&self, cfg: &mut SocketConfig) {
        if let Some(v) = self.passphrase.as_ref() {
            cfg.passphrase = Some(v.clone());
        }
        if let Some(v) = self.key_length {
            cfg.key_length = v;
        }
        if let Some(v) = self.latency {
            cfg.latency = Some(v);
        }
        if let Some(v) = self.recv_latency {
            cfg.recv_latency = Some(v);
        }
        if let Some(v) = self.peer_latency {
            cfg.peer_latency = Some(v);
        }
        if let Some(v) = self.mss {
            cfg.mss = Some(v);
        }
        if let Some(v) = self.payload_size {
            cfg.payload_size = Some(v);
        }
        if let Some(v) = self.max_bandwidth {
            cfg.max_bandwidth = Some(v);
        }
        if let Some(v) = self.input_bandwidth {
            cfg.input_bandwidth = Some(v);
        }
        if let Some(v) = self.overhead_bandwidth_pct {
            cfg.overhead_bandwidth_pct = Some(v);
        }
        if let Some(v) = self.stream_id.as_ref() {
            cfg.stream_id = Some(v.clone());
        }
        if let Some(v) = self.loss_max_ttl {
            cfg.loss_max_ttl = Some(v);
        }
        if let Some(v) = self.too_late_packet_drop {
            cfg.too_late_packet_drop = Some(v);
        }
        if let Some(v) = self.flow_window_packets {
            cfg.flow_window_packets = Some(v);
        }
        if let Some(v) = self.packet_filter.as_ref() {
            cfg.packet_filter = Some(v.clone());
        }
        if let Some(v) = self.congestion {
            cfg.congestion = Some(v);
        }
        if let Some(v) = self.recv_timeout {
            cfg.recv_timeout = Some(v);
        }
        if let Some(v) = self.send_timeout {
            cfg.send_timeout = Some(v);
        }
        if let Some(v) = self.connect_timeout {
            cfg.connect_timeout = Some(v);
        }
        if let Some(v) = self.linger {
            cfg.linger = Some(v);
        }
        if let Some(v) = self.udp_recv_buffer_bytes {
            cfg.udp_recv_buffer_bytes = Some(v);
        }
        if let Some(v) = self.udp_send_buffer_bytes {
            cfg.udp_send_buffer_bytes = Some(v);
        }
    }

    /// Same shape for `ListenerConfig` (for symmetry with future
    /// listener-side URL support; v1 has no listener-side _open in srt-c).
    pub fn apply_to_listener(&self, cfg: &mut ListenerConfig) {
        if let Some(v) = self.passphrase.as_ref() {
            cfg.passphrase = Some(v.clone());
        }
        if let Some(v) = self.key_length {
            cfg.key_length = v;
        }
        if let Some(v) = self.latency {
            cfg.latency = Some(v);
        }
        if let Some(v) = self.recv_latency {
            cfg.recv_latency = Some(v);
        }
        if let Some(v) = self.mss {
            cfg.mss = Some(v);
        }
        if let Some(v) = self.payload_size {
            cfg.payload_size = Some(v);
        }
        if let Some(v) = self.max_bandwidth {
            cfg.max_bandwidth = Some(v);
        }
        if let Some(v) = self.overhead_bandwidth_pct {
            cfg.overhead_bandwidth_pct = Some(v);
        }
        if let Some(v) = self.loss_max_ttl {
            cfg.loss_max_ttl = Some(v);
        }
        if let Some(v) = self.too_late_packet_drop {
            cfg.too_late_packet_drop = Some(v);
        }
        if let Some(v) = self.flow_window_packets {
            cfg.flow_window_packets = Some(v);
        }
        if let Some(v) = self.packet_filter.as_ref() {
            cfg.packet_filter = Some(v.clone());
        }
        if let Some(v) = self.congestion {
            cfg.congestion = Some(v);
        }
        if let Some(v) = self.recv_timeout {
            cfg.recv_timeout = Some(v);
        }
        if let Some(v) = self.send_timeout {
            cfg.send_timeout = Some(v);
        }
        if let Some(v) = self.linger {
            cfg.linger = Some(v);
        }
        if let Some(v) = self.udp_recv_buffer_bytes {
            cfg.udp_recv_buffer_bytes = Some(v);
        }
        // udp_send_buffer_bytes is sender-only — no listener field.
        // ListenerConfig has no peer_latency, stream_id, or input_bandwidth.
        // peer_latency is a caller-side option (libsrt allows it to be set
        // on listeners but it has no effect there).
        // stream_id is set by the caller during handshake; the listener
        // reads it via Socket::stream_id on accepted sockets.
        // input_bandwidth (SRTO_INPUTBW) is sender-side bandwidth budgeting.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_test::traced_test;

    #[test]
    #[traced_test]
    fn warns_on_high_latency_likely_ffmpeg_units() {
        // 120000 ms = 120s; ffmpeg URLs use µs, so this is a likely paste-from-ffmpeg.
        let _ = SrtUrl::parse("srt://h:9000?latency=120000").unwrap();
        assert!(
            logs_contain("ffmpeg uses microseconds while srt-rust"),
            "expected µs/ms warning to fire for latency=120000",
        );
    }

    #[test]
    #[traced_test]
    fn no_warn_on_realistic_latency() {
        let _ = SrtUrl::parse("srt://h:9000?latency=200").unwrap();
        assert!(
            !logs_contain("ffmpeg uses microseconds"),
            "should not warn on realistic 200ms latency",
        );
        let _ = SrtUrl::parse("srt://h:9000?latency=4000").unwrap();
        assert!(
            !logs_contain("ffmpeg uses microseconds"),
            "should not warn on 4000ms latency (still realistic)",
        );
    }

    #[test]
    #[traced_test]
    fn warns_on_high_rcvlatency() {
        let _ = SrtUrl::parse("srt://h:9000?rcvlatency=15000").unwrap();
        assert!(logs_contain("ffmpeg uses microseconds"));
    }

    #[test]
    #[traced_test]
    fn warns_on_high_peerlatency() {
        let _ = SrtUrl::parse("srt://h:9000?peerlatency=15000").unwrap();
        assert!(logs_contain("ffmpeg uses microseconds"));
    }

    #[test]
    fn url_conntimeo_parses() {
        let u = SrtUrl::parse("srt://h:9000?conntimeo=10000").unwrap();
        assert_eq!(
            u.overlay.connect_timeout,
            Some(Duration::from_millis(10000))
        );
    }

    #[test]
    fn url_connect_timeout_alias_parses() {
        let u = SrtUrl::parse("srt://h:9000?connect_timeout=10000").unwrap();
        assert_eq!(
            u.overlay.connect_timeout,
            Some(Duration::from_millis(10000))
        );
    }

    #[test]
    fn url_linger_parses() {
        let u = SrtUrl::parse("srt://h:9000?linger=5").unwrap();
        assert_eq!(u.overlay.linger, Some(Duration::from_secs(5)));
    }

    #[test]
    fn url_udprcvbuf_parses() {
        let u = SrtUrl::parse("srt://h:9000?udprcvbuf=2000000").unwrap();
        assert_eq!(u.overlay.udp_recv_buffer_bytes, Some(2_000_000));
    }

    #[test]
    fn url_udpsndbuf_parses() {
        let u = SrtUrl::parse("srt://h:9000?udpsndbuf=2000000").unwrap();
        assert_eq!(u.overlay.udp_send_buffer_bytes, Some(2_000_000));
    }

    #[test]
    fn url_recv_buffer_size_alias_parses() {
        let u = SrtUrl::parse("srt://h:9000?recv_buffer_size=2000000").unwrap();
        assert_eq!(u.overlay.udp_recv_buffer_bytes, Some(2_000_000));
    }

    #[test]
    fn url_send_buffer_size_alias_parses() {
        let u = SrtUrl::parse("srt://h:9000?send_buffer_size=2000000").unwrap();
        assert_eq!(u.overlay.udp_send_buffer_bytes, Some(2_000_000));
    }

    #[test]
    fn url_alias_pkt_size_matches_payloadsize() {
        let canonical = SrtUrl::parse("srt://h:9000?payloadsize=1316").unwrap();
        let alias = SrtUrl::parse("srt://h:9000?pkt_size=1316").unwrap();
        assert_eq!(canonical.overlay.payload_size, alias.overlay.payload_size);
        assert_eq!(alias.overlay.payload_size, Some(1316));
    }

    #[test]
    fn url_alias_payload_size_matches_payloadsize() {
        let alias = SrtUrl::parse("srt://h:9000?payload_size=1316").unwrap();
        assert_eq!(alias.overlay.payload_size, Some(1316));
    }

    #[test]
    fn url_alias_srt_streamid_matches_streamid() {
        let canonical = SrtUrl::parse("srt://h:9000?streamid=abc").unwrap();
        let alias = SrtUrl::parse("srt://h:9000?srt_streamid=abc").unwrap();
        assert_eq!(
            canonical.overlay.stream_id.as_ref().map(|s| s.as_str()),
            alias.overlay.stream_id.as_ref().map(|s| s.as_str()),
        );
        assert_eq!(
            alias.overlay.stream_id.as_ref().map(|s| s.as_str()),
            Some("abc")
        );
    }

    #[test]
    fn url_alias_tsbpddelay_matches_latency() {
        let alias = SrtUrl::parse("srt://h:9000?tsbpddelay=200").unwrap();
        assert_eq!(alias.overlay.latency, Some(Duration::from_millis(200)));
    }

    #[test]
    #[traced_test]
    fn url_alias_tsbpddelay_high_value_warns() {
        let _ = SrtUrl::parse("srt://h:9000?tsbpddelay=120000").unwrap();
        assert!(
            logs_contain("ffmpeg uses microseconds while srt-rust"),
            "the µs/ms warning should fire through the tsbpddelay alias too",
        );
    }

    #[test]
    fn url_alias_smoother_matches_congestion() {
        let canonical = SrtUrl::parse("srt://h:9000?congestion=live").unwrap();
        let alias = SrtUrl::parse("srt://h:9000?smoother=live").unwrap();
        assert_eq!(canonical.overlay.congestion, alias.overlay.congestion);
    }

    #[test]
    fn url_alias_ffs_matches_fc() {
        let canonical = SrtUrl::parse("srt://h:9000?fc=25600").unwrap();
        let alias = SrtUrl::parse("srt://h:9000?ffs=25600").unwrap();
        assert_eq!(
            canonical.overlay.flow_window_packets,
            alias.overlay.flow_window_packets
        );
    }
}
