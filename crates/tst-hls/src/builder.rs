//! Builder for [`crate::HlsPublisher`].

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use crate::config::{HlsConfig, HlsMode};
use crate::error::HlsError;
use crate::publisher::HlsPublisher;
#[cfg(feature = "serve")]
use crate::url::{HlsUrl, HlsUrlError};

/// Builder for [`HlsPublisher`].
#[must_use]
#[derive(Debug, Clone)]
pub struct HlsPublisherBuilder {
    config: HlsConfig,
}

impl HlsPublisherBuilder {
    /// Start an empty builder using [`HlsConfig::default`].
    pub fn new() -> Self {
        Self {
            config: HlsConfig::default(),
        }
    }

    /// Parse an `hls://` or `hlss://` URL and seed the config from it.
    #[cfg(feature = "serve")]
    pub fn from_url(url: &str) -> Result<Self, HlsUrlError> {
        let parsed = HlsUrl::parse(url)?;
        let mut config = HlsConfig::default();
        config.merge_from_url(&parsed);
        Ok(Self { config })
    }

    /// HTTP server bind address.
    pub fn bind(mut self, addr: SocketAddr) -> Self {
        self.config.bind = addr;
        self
    }

    /// Filesystem directory for `.ts` segments + `playlist.m3u8`.
    pub fn output_dir<P: AsRef<Path>>(mut self, dir: P) -> Self {
        self.config.output_dir = dir.as_ref().to_path_buf();
        self
    }

    /// Target segment duration.
    pub fn segment_duration(mut self, d: Duration) -> Self {
        self.config.segment_duration = d;
        self
    }

    /// Rolling window size in LIVE mode.
    pub fn playlist_window(mut self, n: usize) -> Self {
        self.config.playlist_window = n;
        self
    }

    /// Playlist mode (LIVE / EVENT / VOD).
    pub fn mode(mut self, mode: HlsMode) -> Self {
        self.config.mode = mode;
        self
    }

    /// Enable HTTP Basic auth.
    pub fn basic_auth<S: Into<String>>(mut self, user: S, pass: S) -> Self {
        self.config.basic_auth = Some((user.into(), pass.into()));
        self
    }

    /// Enable HTTPS by supplying cert + key paths (PEM).  Requires the `tls`
    /// cargo feature; without it [`HlsPublisherBuilder::build`] returns
    /// [`HlsError::TlsDisabled`].
    pub fn enable_tls<P: AsRef<Path>>(mut self, cert: P, key: P) -> Self {
        self.config.tls_cert = Some(cert.as_ref().to_path_buf());
        self.config.tls_key = Some(key.as_ref().to_path_buf());
        self
    }

    /// Construct the publisher (binds the HTTP server immediately).
    pub fn build(self) -> Result<HlsPublisher, HlsError> {
        #[cfg(not(feature = "tls"))]
        if self.config.tls_cert.is_some() || self.config.tls_key.is_some() {
            return Err(HlsError::TlsDisabled);
        }
        HlsPublisher::with_config(self.config)
    }
}

impl Default for HlsPublisherBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tst_core::publisher::Publisher;

    #[test]
    fn fluent_chain_compiles() {
        let dir = std::env::temp_dir().join(format!("hls-builder-chain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _b = HlsPublisherBuilder::new()
            .bind("127.0.0.1:0".parse().unwrap())
            .output_dir(&dir)
            .segment_duration(Duration::from_secs(2))
            .playlist_window(3)
            .mode(HlsMode::Event)
            .basic_auth("u", "p");
    }

    #[cfg(feature = "serve")]
    #[test]
    fn from_url_seeds_config() {
        let b = HlsPublisherBuilder::from_url("hls://127.0.0.1:9100?mode=vod&playlist_window=10")
            .unwrap();
        assert_eq!(b.config.mode, HlsMode::Vod);
        assert_eq!(b.config.playlist_window, 10);
    }

    #[cfg(feature = "serve")]
    #[test]
    fn builder_build_smoke() {
        let dir = std::env::temp_dir().join(format!(
            "hls-builder-smoke-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let p = HlsPublisherBuilder::new()
            .bind("127.0.0.1:0".parse().unwrap())
            .output_dir(&dir)
            .build()
            .unwrap();
        let addr = p.local_addr().unwrap();
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        p.finish().unwrap();
    }
}
