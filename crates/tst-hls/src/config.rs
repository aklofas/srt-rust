//! [`HlsConfig`] — segment duration, playlist window, mode, auth, TLS, output dir.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

#[cfg(feature = "serve")]
use crate::url::HlsUrl;

/// HLS playlist mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HlsMode {
    /// LIVE — rolling-window playlist; older segments evict from disk and
    /// playlist.  No ENDLIST until [`Publisher::finish`].
    ///
    /// [`Publisher::finish`]: tst_core::publisher::Publisher::finish
    Live,
    /// EVENT — playlist monotone-grows (no segments evicted) until finish.
    /// ENDLIST written on finish.
    Event,
    /// VOD — same as Event but written all-at-once when finish is called
    /// (no incremental playlist updates during the run).
    Vod,
}

/// Configuration for [`HlsPublisher`].
///
/// Use [`HlsConfig::default`] then mutate, or build via
/// [`crate::HlsPublisherBuilder`].
///
/// [`HlsPublisher`]: crate::HlsPublisher
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HlsConfig {
    /// Where the HTTP server binds.
    ///
    /// Defaults to `127.0.0.1:8080` (loopback only).  Binding all interfaces
    /// (`0.0.0.0`) is an explicit choice — front the server with a reverse
    /// proxy, or use auth+TLS (`basic_auth`/`tls_cert`/`tls_key`), when
    /// exposing it beyond localhost.
    ///
    /// Ignored when the `serve` feature is disabled.
    pub bind: SocketAddr,
    /// Filesystem directory where segments + playlist are written.
    /// Created if it doesn't exist; emptied of stale `segment_*.ts` /
    /// `playlist.m3u8` on construction.
    pub output_dir: PathBuf,
    /// Target segment duration.  Real segments cut on `cut_segment()` calls
    /// (IDR-aligned) OR when this duration is exceeded since the segment opened.
    pub segment_duration: Duration,
    /// Number of segments visible in the LIVE playlist (rolling window).
    /// Ignored for Event/Vod modes.
    pub playlist_window: usize,
    /// Playlist mode.
    pub mode: HlsMode,
    /// Optional HTTP Basic auth (user, password).  None disables auth.
    /// Ignored when the `serve` feature is disabled.
    pub basic_auth: Option<(String, String)>,
    /// Optional TLS server cert path (PEM).  Required if [`Self::tls_key`] set.
    /// Ignored when the `serve` feature is disabled.
    pub tls_cert: Option<PathBuf>,
    /// Optional TLS server key path (PEM).  Required if [`Self::tls_cert`] set.
    /// Ignored when the `serve` feature is disabled.
    pub tls_key: Option<PathBuf>,
}

impl Default for HlsConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8080".parse().expect("hard-coded default"),
            output_dir: std::env::temp_dir().join("tstrans-hls"),
            segment_duration: Duration::from_secs(4),
            playlist_window: 6,
            mode: HlsMode::Live,
            basic_auth: None,
            tls_cert: None,
            tls_key: None,
        }
    }
}

impl HlsConfig {
    /// Overlay URL-derived values on top of an existing config.
    #[cfg(feature = "serve")]
    pub fn merge_from_url(&mut self, url: &HlsUrl) {
        self.bind = SocketAddr::new(url.addr, url.port);
        if let Some(dir) = &url.output_dir {
            self.output_dir = PathBuf::from(dir);
        }
        if let Some(d) = url.segment_duration {
            self.segment_duration = d;
        }
        if let Some(n) = url.playlist_window {
            self.playlist_window = n;
        }
        if let Some(m) = url.mode {
            self.mode = m;
        }
        if let (Some(u), Some(p)) = (&url.auth_user, &url.auth_pass) {
            self.basic_auth = Some((u.clone(), p.clone()));
        }
        if let Some(c) = &url.cert {
            self.tls_cert = Some(PathBuf::from(c));
        }
        if let Some(k) = &url.key {
            self.tls_key = Some(PathBuf::from(k));
        }
    }

    /// Validate the config.  Returns an error message describing the first
    /// problem found (or `None` if valid).
    pub fn validate(&self) -> Option<String> {
        if self.segment_duration.is_zero() {
            return Some("segment_duration must be > 0".into());
        }
        if matches!(self.mode, HlsMode::Live) {
            if self.playlist_window == 0 {
                return Some("playlist_window must be > 0 in Live mode".into());
            }
            // RFC 8216 §6.2.2: a live playlist must be able to hold ≥ 3 target
            // durations. target = ceil(segment_duration); reject windows too
            // small to ever reach 3× target with target-sized segments. The
            // comparison is done in integer nanoseconds (u128) so a config
            // exactly on the boundary classifies exactly — floats are used only
            // to format the error message.
            let target_secs = (self.segment_duration.as_secs()
                + u64::from(self.segment_duration.subsec_nanos() > 0))
            .max(1);
            let window_nanos = self.playlist_window as u128 * self.segment_duration.as_nanos();
            let min_nanos = 3u128 * u128::from(target_secs) * 1_000_000_000u128;
            if window_nanos < min_nanos {
                return Some(format!(
                    "playlist_window ({}) too small: {} × {:.3}s = {:.3}s cannot hold \
                     3 × target duration ({}s) (RFC 8216 §6.2.2)",
                    self.playlist_window,
                    self.playlist_window,
                    self.segment_duration.as_secs_f64(),
                    self.playlist_window as f64 * self.segment_duration.as_secs_f64(),
                    3 * target_secs,
                ));
            }
        }
        match (&self.tls_cert, &self.tls_key) {
            (Some(_), None) => Some("tls_cert set but tls_key missing".into()),
            (None, Some(_)) => Some("tls_key set but tls_cert missing".into()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bind_is_loopback() {
        let cfg = HlsConfig::default();
        assert!(
            cfg.bind.ip().is_loopback(),
            "default bind must be loopback, got {}",
            cfg.bind
        );
    }

    #[test]
    fn default_is_valid() {
        let cfg = HlsConfig::default();
        assert_eq!(cfg.validate(), None);
    }

    #[test]
    fn zero_duration_invalid() {
        let cfg = HlsConfig {
            segment_duration: Duration::from_secs(0),
            ..HlsConfig::default()
        };
        assert!(cfg.validate().is_some());
    }

    #[test]
    fn live_zero_window_invalid() {
        let cfg = HlsConfig {
            playlist_window: 0,
            ..HlsConfig::default()
        };
        assert!(cfg.validate().is_some());
    }

    #[test]
    fn live_window_too_small_for_3x_target_invalid() {
        // target = ceil(4) = 4 → need ≥ 12 s; 2 × 4 = 8 s is too small.
        let cfg = HlsConfig {
            segment_duration: Duration::from_secs(4),
            playlist_window: 2,
            mode: HlsMode::Live,
            ..HlsConfig::default()
        };
        assert!(cfg.validate().is_some());
    }

    #[test]
    fn default_live_config_satisfies_3x_target() {
        // default: window 6 × 4 s = 24 s ≥ 3 × 4 = 12 s.
        assert_eq!(HlsConfig::default().validate(), None);
    }

    #[test]
    fn event_mode_small_window_is_valid() {
        // The 3×-target rule is Live-only.
        let cfg = HlsConfig {
            segment_duration: Duration::from_secs(4),
            playlist_window: 1,
            mode: HlsMode::Event,
            ..HlsConfig::default()
        };
        assert_eq!(cfg.validate(), None);
    }

    #[cfg(feature = "serve")]
    #[test]
    fn merge_overlay() {
        let mut cfg = HlsConfig::default();
        let u =
            HlsUrl::parse("hls://127.0.0.1:9000?segment_duration=6&playlist_window=10&mode=vod")
                .unwrap();
        cfg.merge_from_url(&u);
        assert_eq!(cfg.segment_duration, Duration::from_secs(6));
        assert_eq!(cfg.playlist_window, 10);
        assert_eq!(cfg.mode, HlsMode::Vod);
    }
}
