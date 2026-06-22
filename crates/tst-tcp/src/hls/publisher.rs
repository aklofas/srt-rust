//! [`HlsPublisher`] — implements [`tst_core::publisher::Publisher`].

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tst_core::publisher::{Publisher, PublisherStats};

use crate::hls::config::HlsConfig;
use crate::hls::error::HlsError;
use crate::hls::playlist;
use crate::hls::segmenter::Segmenter;
use crate::hls::stats::HlsStats;

/// HLS publisher.
pub struct HlsPublisher {
    pub(crate) state: Arc<Mutex<State>>,
    pub(crate) finished: bool,
    pub(crate) server: Option<crate::hls::http_server::ServerHandle>,
}

pub(crate) struct State {
    pub(crate) segmenter: Segmenter,
    pub(crate) bytes_pushed_total: u64,
}

impl HlsPublisher {
    pub fn with_config(config: HlsConfig) -> Result<Self, HlsError> {
        if let Some(msg) = config.validate() {
            return Err(HlsError::InvalidConfig(msg));
        }
        let bind = config.bind;
        let basic_auth = config.basic_auth.clone();

        #[cfg(feature = "tls")]
        let tls_config = match (&config.tls_cert, &config.tls_key) {
            (Some(cert), Some(key)) => Some(crate::hls::tls::load_server_config(cert, key)?),
            _ => None,
        };
        #[cfg(not(feature = "tls"))]
        if config.tls_cert.is_some() || config.tls_key.is_some() {
            return Err(HlsError::TlsDisabled);
        }

        let segmenter = Segmenter::new(config)?;
        let state = Arc::new(Mutex::new(State {
            segmenter,
            bytes_pushed_total: 0,
        }));

        let server = crate::hls::http_server::ServerHandle::start(
            state.clone(),
            bind,
            basic_auth,
            #[cfg(feature = "tls")]
            tls_config,
        )?;

        Ok(Self {
            state,
            finished: false,
            server: Some(server),
        })
    }

    /// Local socket address the HTTP server bound to.
    pub fn local_addr(&self) -> Option<std::net::SocketAddr> {
        self.server.as_ref().map(|s| s.local_addr())
    }

    /// Snapshot of richer per-impl stats.
    pub fn hls_stats(&self) -> HlsStats {
        let s = self.state.lock().expect("HlsPublisher poisoned");
        HlsStats {
            segments_written: s.segmenter.segments_written(),
            bytes_pushed_total: s.bytes_pushed_total,
            open_segment_bytes: s.segmenter.open_segment_bytes(),
        }
    }

    /// Manually render the current playlist (for tests / introspection).
    pub fn render_playlist(&self, is_final: bool) -> String {
        let s = self.state.lock().expect("HlsPublisher poisoned");
        playlist::render(&s.segmenter, is_final)
    }
}

impl Publisher for HlsPublisher {
    type Error = HlsError;

    fn push_ts(&mut self, ts_bytes: &[u8]) -> Result<(), HlsError> {
        if self.finished {
            return Err(HlsError::Finished);
        }
        if ts_bytes.len() % 188 != 0 {
            return Err(HlsError::UnalignedPushTs {
                len: ts_bytes.len(),
            });
        }
        let mut s = self.state.lock().expect("HlsPublisher poisoned");
        s.segmenter.tick()?;
        s.segmenter.push_ts(ts_bytes)?;
        s.bytes_pushed_total = s.bytes_pushed_total.saturating_add(ts_bytes.len() as u64);
        Ok(())
    }

    fn cut_segment(&mut self) -> Result<(), HlsError> {
        if self.finished {
            return Err(HlsError::Finished);
        }
        let mut s = self.state.lock().expect("HlsPublisher poisoned");
        s.segmenter.cut()
    }

    fn cut_segment_with_duration(&mut self, media_duration: Duration) -> Result<(), HlsError> {
        if self.finished {
            return Err(HlsError::Finished);
        }
        let mut s = self.state.lock().expect("HlsPublisher poisoned");
        s.segmenter.cut_with_duration(Some(media_duration))
    }

    fn finish(mut self) -> Result<(), HlsError> {
        if self.finished {
            return Err(HlsError::Finished);
        }
        self.finished = true;
        let (output_dir, final_pl) = {
            let mut s = self.state.lock().expect("HlsPublisher poisoned");
            s.segmenter.finalize()?;
            let pl = playlist::render(&s.segmenter, true);
            (s.segmenter.output_dir().to_path_buf(), pl)
        };
        std::fs::write(output_dir.join("playlist.m3u8"), &final_pl).map_err(HlsError::Io)?;
        if let Some(server) = self.server.take() {
            server.shutdown();
        }
        Ok(())
    }

    fn stats(&self) -> PublisherStats {
        let s = self.state.lock().expect("HlsPublisher poisoned");
        let mut out = PublisherStats::default();
        out.segments_written = s.segmenter.segments_written();
        out.bytes_written = s.bytes_pushed_total;
        out.current_segment_age = s.segmenter.current_segment_age();
        out.last_segment_duration = s.segmenter.last_segment_duration();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hls::config::HlsMode;
    use std::path::PathBuf;
    use std::time::Duration;

    fn tmpdir(label: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "hls-pub-test-{}-{}-{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn push_segments_then_finish_writes_playlist() {
        let dir = tmpdir("ok");
        let cfg = HlsConfig {
            output_dir: dir.clone(),
            bind: "127.0.0.1:0".parse().unwrap(),
            ..HlsConfig::default()
        };
        let mut p = HlsPublisher::with_config(cfg).unwrap();
        p.push_ts(&[0x47u8; 376]).unwrap();
        p.cut_segment().unwrap();
        p.push_ts(&[0x47u8; 376]).unwrap();
        p.cut_segment().unwrap();

        let stats = p.stats();
        assert_eq!(stats.segments_written, 2);
        assert_eq!(stats.bytes_written, 752);

        p.finish().unwrap();
        let pl = std::fs::read_to_string(dir.join("playlist.m3u8")).unwrap();
        assert!(pl.contains("#EXTM3U"));
        assert!(pl.contains("segment_00000.ts"));
    }

    #[test]
    fn unaligned_push_rejected() {
        let dir = tmpdir("unalign");
        let cfg = HlsConfig {
            output_dir: dir,
            bind: "127.0.0.1:0".parse().unwrap(),
            ..HlsConfig::default()
        };
        let mut p = HlsPublisher::with_config(cfg).unwrap();
        assert!(matches!(
            p.push_ts(&[0u8; 187]),
            Err(HlsError::UnalignedPushTs { len: 187 })
        ));
    }

    #[test]
    fn http_serves_playlist_and_segment() {
        let dir = tmpdir("http");
        let cfg = HlsConfig {
            output_dir: dir.clone(),
            bind: "127.0.0.1:0".parse().unwrap(),
            ..HlsConfig::default()
        };
        let mut p = HlsPublisher::with_config(cfg).unwrap();
        p.push_ts(&[0x47u8; 376]).unwrap();
        p.cut_segment().unwrap();
        let addr = p.local_addr().unwrap();

        // Blocking HTTP GET using std::net.
        use std::io::{Read, Write};
        let mut sock = std::net::TcpStream::connect(addr).unwrap();
        sock.write_all(b"GET /playlist.m3u8 HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut resp = String::new();
        sock.read_to_string(&mut resp).unwrap();
        assert!(resp.contains("200 OK"));
        assert!(resp.contains("#EXTM3U"));
        assert!(resp.contains("segment_00000.ts"));

        let mut sock = std::net::TcpStream::connect(addr).unwrap();
        sock.write_all(b"GET /segment_00000.ts HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut resp = Vec::new();
        sock.read_to_end(&mut resp).unwrap();
        let s = String::from_utf8_lossy(&resp);
        assert!(s.contains("200 OK"));
        assert!(s.contains("video/mp2t"));

        p.finish().unwrap();
    }

    #[test]
    fn cut_with_duration_sets_media_extinf() {
        let dir = tmpdir("mediadur");
        let cfg = HlsConfig {
            output_dir: dir,
            bind: "127.0.0.1:0".parse().unwrap(),
            mode: HlsMode::Event,
            ..HlsConfig::default()
        };
        let mut p = HlsPublisher::with_config(cfg).unwrap();
        p.push_ts(&[0x47u8; 188]).unwrap();
        p.cut_segment_with_duration(Duration::from_millis(3200))
            .unwrap();
        p.push_ts(&[0x47u8; 188]).unwrap();
        p.cut_segment_with_duration(Duration::from_millis(4100))
            .unwrap();
        let pl = p.render_playlist(false);
        assert!(pl.contains("#EXTINF:3.200,"), "playlist:\n{pl}");
        assert!(pl.contains("#EXTINF:4.100,"), "playlist:\n{pl}");
        p.finish().unwrap();
    }

    #[test]
    fn basic_auth_rejects_unauthorized() {
        let dir = tmpdir("auth");
        let cfg = HlsConfig {
            output_dir: dir,
            bind: "127.0.0.1:0".parse().unwrap(),
            basic_auth: Some(("alice".into(), "s3cret".into())),
            ..HlsConfig::default()
        };
        let p = HlsPublisher::with_config(cfg).unwrap();
        let addr = p.local_addr().unwrap();

        use std::io::{Read, Write};
        let mut sock = std::net::TcpStream::connect(addr).unwrap();
        sock.write_all(b"GET /playlist.m3u8 HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut resp = String::new();
        sock.read_to_string(&mut resp).unwrap();
        assert!(resp.contains("401 Unauthorized"));
    }
}
