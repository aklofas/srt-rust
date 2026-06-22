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

/// A segment evicted from the live playlist but retained on disk until its
/// RFC 8216 §6.2.2 availability window elapses (so clients mid-download of
/// an older playlist don't get a 404).
struct GraceEntry {
    filename: String,
    removed_at: Instant,
    /// Removed-segment duration + the longest playlist that referenced it.
    availability: Duration,
}

pub(crate) struct Segmenter {
    config: HlsConfig,
    /// Immutable `#EXT-X-TARGETDURATION` (seconds), chosen once at
    /// construction as the ceiling of the configured segment duration.
    /// RFC 8216 §6.2.1 forbids the value from changing once the playlist is
    /// published, and §4.3.3.1 requires every EXTINF (rounded to the nearest
    /// integer) to be ≤ this value — so we ceil the configured cap, never
    /// floor it and never recompute it from observed history.
    target_duration_secs: u64,
    next_seq: u64,
    history: VecDeque<Segment>,
    grace: VecDeque<GraceEntry>,
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

        let target_duration_secs = config.segment_duration.as_secs_f64().ceil().max(1.0) as u64;

        Ok(Self {
            config,
            target_duration_secs,
            next_seq: 0,
            history: VecDeque::new(),
            grace: VecDeque::new(),
            current: None,
        })
    }

    /// Append TS bytes to the current segment (opens one if none).
    pub(crate) fn push_ts(&mut self, ts_bytes: &[u8]) -> Result<(), HlsError> {
        if ts_bytes.len() % 188 != 0 {
            return Err(HlsError::UnalignedPushTs {
                len: ts_bytes.len(),
            });
        }
        if self.current.is_none() {
            self.open_new()?;
        }
        let open = self.current.as_mut().expect("just opened");
        open.file.write_all(ts_bytes).map_err(HlsError::Io)?;
        open.bytes_written = open.bytes_written.saturating_add(ts_bytes.len() as u64);
        Ok(())
    }

    /// Explicitly cut the current segment (called on keyframe by
    /// MuxPublisher). No-op if no segment is currently open.
    pub(crate) fn cut(&mut self) -> Result<(), HlsError> {
        self.cut_with_duration(None)
    }

    /// Cut the current segment, recording `media_duration` (from PTS) as the
    /// segment's duration. `None`, or a zero media duration (a degenerate
    /// single-AU segment), falls back to wall-clock elapsed so `#EXTINF` is
    /// never exactly zero. No-op if no segment is open.
    pub(crate) fn cut_with_duration(
        &mut self,
        media_duration: Option<Duration>,
    ) -> Result<(), HlsError> {
        if let Some(open) = self.current.take() {
            self.close_segment(open, media_duration)?;
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

    /// Visible segments for the playlist. Live history is already pruned by
    /// eviction (older segments live in the grace queue until their
    /// availability window elapses); Event/Vod retain everything — so all
    /// modes render the full history.
    pub(crate) fn visible_segments(&self) -> Vec<Segment> {
        self.history.iter().cloned().collect()
    }

    /// First sequence number in `visible_segments`.
    pub(crate) fn media_sequence(&self) -> u64 {
        self.visible_segments().first().map(|s| s.seq).unwrap_or(0)
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

    /// Immutable target duration for `#EXT-X-TARGETDURATION` (seconds).
    ///
    /// Chosen once at construction (the ceiling of the configured segment
    /// duration) and never recomputed — RFC 8216 §6.2.1 forbids the value
    /// from changing once the playlist is published, and §4.3.3.1 requires
    /// every EXTINF, rounded to the nearest integer, to be ≤ this value.
    pub(crate) fn target_duration_secs(&self) -> u64 {
        self.target_duration_secs
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

    fn close_segment(
        &mut self,
        mut open: OpenSegment,
        media_duration: Option<Duration>,
    ) -> Result<(), HlsError> {
        open.file.flush().map_err(HlsError::Io)?;
        drop(open.file);
        let duration = media_duration
            .filter(|d| !d.is_zero())
            .unwrap_or_else(|| open.opened_at.elapsed());
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
        let now = Instant::now();
        let target = Duration::from_secs(self.target_duration_secs);
        // RFC 8216 §6.2.2: never let the live playlist fall below 3× target.
        let min_duration = target * 3;
        // Conservative bound on "the longest Playlist file" that referenced a
        // segment: the full window at target duration.
        let longest_playlist = target * self.config.playlist_window as u32;

        let mut total: Duration = self.history.iter().map(|s| s.duration).sum();
        while self.history.len() > self.config.playlist_window {
            let front_dur = self.history.front().map(|s| s.duration).unwrap_or_default();
            // Stop if evicting the oldest would drop the playlist below 3×
            // target (this is what makes eviction duration-aware, not count-
            // only).
            if total.saturating_sub(front_dur) < min_duration {
                break;
            }
            if let Some(evict) = self.history.pop_front() {
                total = total.saturating_sub(evict.duration);
                self.grace.push_back(GraceEntry {
                    filename: evict.filename,
                    removed_at: now,
                    availability: evict.duration + longest_playlist,
                });
            }
        }
        self.purge_grace(now);
        Ok(())
    }

    /// Delete grace-queued files whose RFC 8216 §6.2.2 availability window
    /// (removed-segment duration + longest referencing playlist) has elapsed.
    /// `now` is a parameter so tests can advance time deterministically.
    pub(crate) fn purge_grace(&mut self, now: Instant) {
        while let Some(front) = self.grace.front() {
            if now.duration_since(front.removed_at) >= front.availability {
                let entry = self.grace.pop_front().expect("front exists");
                let path = self.config.output_dir.join(&entry.filename);
                let _ = std::fs::remove_file(&path);
            } else {
                break;
            }
        }
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
    fn live_evicts_beyond_window_into_grace_respecting_3x_target() {
        let dir = tmpdir();
        let cfg = HlsConfig {
            output_dir: dir.clone(),
            mode: HlsMode::Live,
            segment_duration: Duration::from_secs(2), // target 2 → 3× = 6 s
            playlist_window: 3,
            ..HlsConfig::default()
        };
        let mut s = Segmenter::new(cfg).unwrap();
        for _ in 0..6 {
            s.push_ts(&[0x47u8; 188]).unwrap();
            s.cut_with_duration(Some(Duration::from_secs(2))).unwrap();
        }
        // 3 newest visible; 3 oldest moved to grace (files still on disk).
        assert_eq!(s.visible_segments().len(), 3);
        assert_eq!(s.media_sequence(), 3);
        for seq in 0..3u64 {
            assert!(
                dir.join(format!("segment_{seq:05}.ts")).exists(),
                "grace-retained file {seq} must still exist"
            );
        }
        // Advance past the availability window and purge — files deleted.
        let far_future = Instant::now() + Duration::from_secs(3600);
        s.purge_grace(far_future);
        for seq in 0..3u64 {
            assert!(
                !dir.join(format!("segment_{seq:05}.ts")).exists(),
                "file {seq} must be deleted after its availability window"
            );
        }
        assert!(dir.join("segment_00005.ts").exists());
    }

    #[test]
    fn live_keeps_more_than_window_to_satisfy_3x_target() {
        let dir = tmpdir();
        let cfg = HlsConfig {
            output_dir: dir,
            mode: HlsMode::Live,
            segment_duration: Duration::from_secs(4), // target 4 → 3× = 12 s
            playlist_window: 3,
            ..HlsConfig::default()
        };
        let mut s = Segmenter::new(cfg).unwrap();
        // Actual segments are 1 s each; 5 s total never reaches 12 s, so the
        // duration floor blocks all eviction even though count > window.
        for _ in 0..5 {
            s.push_ts(&[0x47u8; 188]).unwrap();
            s.cut_with_duration(Some(Duration::from_secs(1))).unwrap();
        }
        assert_eq!(s.visible_segments().len(), 5, "duration floor keeps all 5");
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

    #[test]
    fn target_duration_is_ceiling_not_floor() {
        // A 4.6 s cap must advertise target 5 (a 4.6 s EXTINF rounds to 5),
        // never 4 — RFC 8216 §4.3.3.1.
        let cfg = HlsConfig {
            output_dir: tmpdir(),
            segment_duration: Duration::from_millis(4600),
            ..HlsConfig::default()
        };
        let s = Segmenter::new(cfg).unwrap();
        assert_eq!(s.target_duration_secs(), 5);
    }

    #[test]
    fn target_duration_ignores_long_history_segments() {
        // RFC 8216 §6.2.1: the target MUST NOT change once published. The old
        // code recomputed max(config, longest-observed); inject a 9 s segment
        // and prove the target stays at the ceiling of the config (4).
        let cfg = HlsConfig {
            output_dir: tmpdir(),
            segment_duration: Duration::from_secs(4),
            mode: HlsMode::Event,
            ..HlsConfig::default()
        };
        let mut s = Segmenter::new(cfg).unwrap();
        s.history.push_back(Segment {
            seq: 0,
            filename: "segment_00000.ts".into(),
            duration: Duration::from_secs(9),
        });
        assert_eq!(
            s.target_duration_secs(),
            4,
            "target must be immutable, not max-of-history"
        );
    }

    #[test]
    fn media_duration_recorded_verbatim() {
        let cfg = HlsConfig {
            output_dir: tmpdir(),
            mode: HlsMode::Event,
            ..HlsConfig::default()
        };
        let mut s = Segmenter::new(cfg).unwrap();
        s.push_ts(&[0x47u8; 188]).unwrap();
        s.cut_with_duration(Some(Duration::from_millis(2500)))
            .unwrap();
        assert_eq!(
            s.history.back().unwrap().duration,
            Duration::from_millis(2500)
        );
    }

    #[test]
    fn zero_media_duration_falls_back_to_wall_clock() {
        let cfg = HlsConfig {
            output_dir: tmpdir(),
            mode: HlsMode::Event,
            ..HlsConfig::default()
        };
        let mut s = Segmenter::new(cfg).unwrap();
        s.push_ts(&[0x47u8; 188]).unwrap();
        // A degenerate single-AU segment reports zero media duration; the
        // segmenter must fall back to wall-clock so EXTINF is never 0.000.
        s.cut_with_duration(Some(Duration::ZERO)).unwrap();
        assert_eq!(s.history.len(), 1);
    }
}
