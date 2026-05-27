//! [`HlsConfig`] — segment duration, playlist window, mode, auth, TLS, output dir.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use crate::hls::url::HlsUrl;

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
/// [`crate::hls::HlsPublisherBuilder`].
///
/// [`HlsPublisher`]: crate::hls::HlsPublisher
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HlsConfig {
    /// Where the HTTP server binds.
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
    pub basic_auth: Option<(String, String)>,
    /// Optional TLS server cert path (PEM).  Required if [`Self::tls_key`] set.
    pub tls_cert: Option<PathBuf>,
    /// Optional TLS server key path (PEM).  Required if [`Self::tls_cert`] set.
    pub tls_key: Option<PathBuf>,
}

impl Default for HlsConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:8080".parse().expect("hard-coded default"),
            output_dir: PathBuf::from("/tmp/hls"),
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
        if matches!(self.mode, HlsMode::Live) && self.playlist_window == 0 {
            return Some("playlist_window must be > 0 in Live mode".into());
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
    fn default_is_valid() {
        let cfg = HlsConfig::default();
        assert_eq!(cfg.validate(), None);
    }

    #[test]
    fn zero_duration_invalid() {
        let mut cfg = HlsConfig::default();
        cfg.segment_duration = Duration::from_secs(0);
        assert!(cfg.validate().is_some());
    }

    #[test]
    fn live_zero_window_invalid() {
        let mut cfg = HlsConfig::default();
        cfg.playlist_window = 0;
        assert!(cfg.validate().is_some());
    }

    #[test]
    fn merge_overlay() {
        let mut cfg = HlsConfig::default();
        let u = HlsUrl::parse("hls://127.0.0.1:9000?segment_duration=6&playlist_window=10&mode=vod").unwrap();
        cfg.merge_from_url(&u);
        assert_eq!(cfg.segment_duration, Duration::from_secs(6));
        assert_eq!(cfg.playlist_window, 10);
        assert_eq!(cfg.mode, HlsMode::Vod);
    }
}
