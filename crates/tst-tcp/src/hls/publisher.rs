//! [`HlsPublisher`] — implements [`tst_core::publisher::Publisher`].

use std::sync::{Arc, Mutex};

use tst_core::publisher::{Publisher, PublisherStats};

use crate::hls::config::HlsConfig;
use crate::hls::error::HlsError;
use crate::hls::playlist;
use crate::hls::segmenter::Segmenter;
use crate::hls::stats::HlsStats;

/// HLS publisher. Owns a [`Segmenter`] that writes `.ts` segments to disk +
/// a playlist renderer. Phase 10 (HTTP server) lands separately.
///
/// Build via [`HlsPublisher::with_config`] or
/// [`crate::hls::HlsPublisherBuilder`].
pub struct HlsPublisher {
    pub(crate) state: Arc<Mutex<State>>,
    pub(crate) finished: bool,
    // server: Option<crate::hls::http_server::ServerHandle>,  // wired in Phase 10
}

/// Shared mutable state between Publisher impl + HTTP server reader.
pub(crate) struct State {
    pub(crate) segmenter: Segmenter,
    pub(crate) bytes_pushed_total: u64,
}

impl HlsPublisher {
    /// Build a publisher with an explicit config.  Does NOT yet bind the
    /// HTTP server (Phase 10 wires that in).
    pub fn with_config(config: HlsConfig) -> Result<Self, HlsError> {
        if let Some(msg) = config.validate() {
            return Err(HlsError::InvalidConfig(msg));
        }
        let segmenter = Segmenter::new(config)?;
        Ok(Self {
            state: Arc::new(Mutex::new(State {
                segmenter,
                bytes_pushed_total: 0,
            })),
            finished: false,
        })
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
            return Err(HlsError::UnalignedPushTs { len: ts_bytes.len() });
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
    use std::path::PathBuf;

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
        let cfg = HlsConfig { output_dir: dir, ..HlsConfig::default() };
        let mut p = HlsPublisher::with_config(cfg).unwrap();
        assert!(matches!(
            p.push_ts(&[0u8; 187]),
            Err(HlsError::UnalignedPushTs { len: 187 })
        ));
    }
}
