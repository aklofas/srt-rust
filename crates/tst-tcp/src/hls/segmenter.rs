//! Internal rolling segmenter — IDR-aligned cuts + duration-driven fallback.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::hls::config::{HlsConfig, HlsMode};
use crate::hls::error::HlsError;

/// One completed segment on disk.
#[derive(Debug, Clone)]
pub(crate) struct Segment {
    /// Monotonically-increasing sequence number (starts at 0).
    pub seq: u64,
    /// Filename relative to [`HlsConfig::output_dir`].
    pub filename: String,
    /// Wall-clock duration of this segment.
    pub duration: Duration,
}

pub(crate) struct Segmenter {
    config: HlsConfig,
    next_seq: u64,
    history: VecDeque<Segment>,
    current: Option<OpenSegment>,
}

struct OpenSegment {
    seq: u64,
    filename: String,
    file: File,
    opened_at: Instant,
    bytes_written: u64,
}

impl Segmenter {
    pub(crate) fn new(config: HlsConfig) -> Result<Self, HlsError> {
        // Ensure output_dir exists.
        std::fs::create_dir_all(&config.output_dir).map_err(HlsError::Io)?;

        // Clean up stale segments + playlist from prior runs (best-effort).
        if let Ok(entries) = std::fs::read_dir(&config.output_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if (name_str.starts_with("segment_") && name_str.ends_with(".ts"))
                    || name_str == "playlist.m3u8"
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }

        Ok(Self {
            config,
            next_seq: 0,
            history: VecDeque::new(),
            current: None,
        })
    }

    /// Append TS bytes to the current segment (opens one if none).
    pub(crate) fn push_ts(&mut self, ts_bytes: &[u8]) -> Result<(), HlsError> {
        if ts_bytes.len() % 188 != 0 {
            return Err(HlsError::UnalignedPushTs { len: ts_bytes.len() });
        }
        if self.current.is_none() {
            self.open_new()?;
        }
        let open = self.current.as_mut().expect("just opened");
        open.file.write_all(ts_bytes).map_err(HlsError::Io)?;
        open.bytes_written = open.bytes_written.saturating_add(ts_bytes.len() as u64);
        Ok(())
    }

    /// Explicitly cut the current segment (called on keyframe by MuxPublisher).
    /// No-op if no segment is currently open.
    pub(crate) fn cut(&mut self) -> Result<(), HlsError> {
        if let Some(open) = self.current.take() {
            self.close_segment(open)?;
        }
        Ok(())
    }

    /// Check duration cap and cut if exceeded.
    pub(crate) fn tick(&mut self) -> Result<(), HlsError> {
        let should_cut = self
            .current
            .as_ref()
            .map(|o| o.opened_at.elapsed() >= self.config.segment_duration)
            .unwrap_or(false);
        if should_cut {
            self.cut()?;
        }
        Ok(())
    }

    /// Visible segments for the playlist.
    pub(crate) fn visible_segments(&self) -> Vec<Segment> {
        match self.config.mode {
            HlsMode::Live => {
                let n = self.history.len().min(self.config.playlist_window);
                self.history.iter().rev().take(n).cloned().rev().collect()
            }
            HlsMode::Event | HlsMode::Vod => self.history.iter().cloned().collect(),
        }
    }

    /// First sequence number in `visible_segments`.
    pub(crate) fn media_sequence(&self) -> u64 {
        self.visible_segments()
            .first()
            .map(|s| s.seq)
            .unwrap_or(0)
    }

    /// Bytes in the segment currently being written (zero between cuts).
    pub(crate) fn open_segment_bytes(&self) -> u64 {
        self.current.as_ref().map(|o| o.bytes_written).unwrap_or(0)
    }

    /// Number of segments completed in this run.
    pub(crate) fn segments_written(&self) -> u64 {
        self.next_seq
    }

    pub(crate) fn current_segment_age(&self) -> Option<Duration> {
        self.current.as_ref().map(|o| o.opened_at.elapsed())
    }

    pub(crate) fn last_segment_duration(&self) -> Option<Duration> {
        self.history.back().map(|s| s.duration)
    }

    /// Cut any open segment (called on finish).
    pub(crate) fn finalize(&mut self) -> Result<(), HlsError> {
        self.cut()
    }

    /// Read a segment file's bytes from disk (called by the HTTP server).
    pub(crate) fn read_segment(&self, filename: &str) -> Result<Vec<u8>, HlsError> {
        let path = self.config.output_dir.join(filename);
        std::fs::read(&path).map_err(HlsError::Io)
    }

    pub(crate) fn output_dir(&self) -> &Path {
        &self.config.output_dir
    }

    pub(crate) fn mode(&self) -> HlsMode {
        self.config.mode
    }

    /// Target duration for `#EXT-X-TARGETDURATION`.
    pub(crate) fn target_duration_secs(&self) -> u64 {
        let from_cfg = self.config.segment_duration.as_secs().max(1);
        let max_observed = self
            .history
            .iter()
            .map(|s| s.duration.as_secs())
            .max()
            .unwrap_or(0);
        from_cfg.max(max_observed)
    }

    fn open_new(&mut self) -> Result<(), HlsError> {
        let seq = self.next_seq;
        self.next_seq += 1;
        let filename = format!("segment_{seq:05}.ts");
        let path: PathBuf = self.config.output_dir.join(&filename);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(HlsError::Io)?;
        self.current = Some(OpenSegment {
            seq,
            filename,
            file,
            opened_at: Instant::now(),
            bytes_written: 0,
        });
        Ok(())
    }

    fn close_segment(&mut self, mut open: OpenSegment) -> Result<(), HlsError> {
        open.file.flush().map_err(HlsError::Io)?;
        drop(open.file);
        let duration = open.opened_at.elapsed();
        let segment = Segment {
            seq: open.seq,
            filename: open.filename.clone(),
            duration,
        };
        self.history.push_back(segment);
        self.evict_if_needed()?;
        Ok(())
    }

    fn evict_if_needed(&mut self) -> Result<(), HlsError> {
        if !matches!(self.config.mode, HlsMode::Live) {
            return Ok(());
        }
        while self.history.len() > self.config.playlist_window {
            if let Some(evict) = self.history.pop_front() {
                let path = self.config.output_dir.join(&evict.filename);
                let _ = std::fs::remove_file(&path);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "hls-seg-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn unaligned_push_rejected() {
        let cfg = HlsConfig {
            output_dir: tmpdir(),
            ..HlsConfig::default()
        };
        let mut s = Segmenter::new(cfg).unwrap();
        assert!(matches!(
            s.push_ts(&[0u8; 187]),
            Err(HlsError::UnalignedPushTs { len: 187 })
        ));
    }

    #[test]
    fn cut_creates_segment_file() {
        let dir = tmpdir();
        let cfg = HlsConfig {
            output_dir: dir.clone(),
            ..HlsConfig::default()
        };
        let mut s = Segmenter::new(cfg).unwrap();
        s.push_ts(&[0x47u8; 376]).unwrap();
        s.cut().unwrap();
        assert!(dir.join("segment_00000.ts").exists());
        assert_eq!(s.segments_written(), 1);
    }

    #[test]
    fn live_window_evicts_old_segments() {
        let dir = tmpdir();
        let cfg = HlsConfig {
            output_dir: dir.clone(),
            mode: HlsMode::Live,
            playlist_window: 2,
            ..HlsConfig::default()
        };
        let mut s = Segmenter::new(cfg).unwrap();
        for _ in 0..4 {
            s.push_ts(&[0x47u8; 188]).unwrap();
            s.cut().unwrap();
        }
        assert!(!dir.join("segment_00000.ts").exists());
        assert!(!dir.join("segment_00001.ts").exists());
        assert!(dir.join("segment_00002.ts").exists());
        assert!(dir.join("segment_00003.ts").exists());
        assert_eq!(s.visible_segments().len(), 2);
        assert_eq!(s.media_sequence(), 2);
    }

    #[test]
    fn event_mode_keeps_all() {
        let dir = tmpdir();
        let cfg = HlsConfig {
            output_dir: dir.clone(),
            mode: HlsMode::Event,
            playlist_window: 2,
            ..HlsConfig::default()
        };
        let mut s = Segmenter::new(cfg).unwrap();
        for _ in 0..4 {
            s.push_ts(&[0x47u8; 188]).unwrap();
            s.cut().unwrap();
        }
        assert_eq!(s.visible_segments().len(), 4);
        assert!(dir.join("segment_00000.ts").exists());
    }
}
