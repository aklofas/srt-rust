//! Opt-in KLV ↔ video pairer.
//!
//! Stateful transducer that ingests `DemuxEvent`s and emits typed
//! `PairerOutput`s. Two strategies:
//!
//! * [`Pairer::nearest_pts`] — match each video AU against the KLV with
//!   nearest PTS, within a configured tolerance. Two modes:
//!   [`MatchMode::Realtime`] (zero buffer, eager emission) and
//!   [`MatchMode::Buffered`] (bounded video buffer, bidirectional
//!   matching).
//! * [`Pairer::last_before_pts`] — sample-and-hold: each video AU pairs
//!   with the most recent KLV where `klv.pts <= video.pts`, optionally
//!   bounded by a freshness ceiling.
//!
//! The pairer is **opt-in** — callers construct it explicitly;
//! [`crate::DemuxReceiver`] does not reach for it by default, preserving
//! the demux module's decoupled-pairing posture.
//!
//! # PTS handling
//!
//! All time values are in 90 kHz ticks per ISO/IEC 13818-1. The demuxer
//! absorbs 33-bit PTS rollover into stream-monotonic `i64` (see
//! `mpegts::common::pts_diff_33bit`), so the pairer subtracts directly
//! without rollover handling. Use
//! [`tst_core::mpegts::demux::pts_to_duration`] for diagnostic
//! conversion.
//!
//! # Cross-language wrappers
//!
//! C ABI / JNI / UniFFI exposure is deferred to the future
//! receiver-surface plan. The Rust types in this module are designed
//! to translate cleanly when that plan lands.
//!
//! # Cookbook
//!
//! See `docs/cookbook.md` recipes 24–27 for canonical realtime,
//! batch-ingest, async sample-and-hold, and EO+IR composition patterns.

mod last_before;
mod nearest;
mod types;

pub use types::{KlvSample, MatchMode, PairerOutput, PairerStats, VideoSample};

use tst_core::mpegts::demux::DemuxEvent;

/// Stateful KLV ↔ video pairer. Construct with one of the strategy
/// constructors; feed `DemuxEvent`s; collect `PairerOutput`s.
///
/// The pairer holds bounded internal state per its strategy. It is
/// video-driven: each `Sample::Video` event on the configured
/// `video_pid` produces exactly one `Paired` or `UnpairedVideo` output,
/// and each `Metadata` event on the configured `klv_pid` produces
/// exactly one `Paired` or `UnpairedKlv` output. Off-route events
/// surface as `PassThrough`.
pub struct Pairer {
    state: PairerState,
    stats: PairerStats,
}

enum PairerState {
    Nearest(nearest::NearestState),
    LastBefore(last_before::LastBeforeState),
}

impl Pairer {
    /// Match each video AU against the nearest-PTS KLV in history,
    /// within `tolerance_ticks` (90 kHz). KLV history holds up to
    /// `max_klv_history` entries (FIFO eviction on overflow).
    ///
    /// Suggested values (caller picks based on stream characteristics):
    /// `tolerance_ticks ≈ 27_000` (0.3 s) for sync KLV at video frame
    /// rate; `max_klv_history ≈ 32` covers ~1 s at 30 fps + 1:1 KLV.
    ///
    /// # Panics
    ///
    /// Panics if `max_klv_history == 0`. A history of zero entries is
    /// useless; the constructor refuses rather than emit `UnpairedVideo`
    /// for every input silently. Same goes for
    /// `MatchMode::Buffered { max_video_buffer: 0 }`.
    pub fn nearest_pts(
        video_pid: u16,
        klv_pid: u16,
        tolerance_ticks: i64,
        max_klv_history: usize,
        mode: MatchMode,
    ) -> Self {
        assert!(max_klv_history > 0, "max_klv_history must be > 0");
        if let MatchMode::Buffered { max_video_buffer } = mode {
            assert!(max_video_buffer > 0, "max_video_buffer must be > 0");
        }
        Self {
            state: PairerState::Nearest(nearest::NearestState::new(
                video_pid,
                klv_pid,
                tolerance_ticks,
                max_klv_history,
                mode,
            )),
            stats: PairerStats::default(),
        }
    }

    /// Sample-and-hold: each video AU pairs with the most recent KLV
    /// where `klv.pts <= video.pts`. If `freshness_ticks` is `Some(n)`,
    /// emit `UnpairedVideo` when the held KLV is older than `n` ticks
    /// behind the video; if `None`, attach regardless of staleness.
    /// Past-only by definition; no `MatchMode` knob applies.
    pub fn last_before_pts(
        video_pid: u16,
        klv_pid: u16,
        freshness_ticks: Option<i64>,
    ) -> Self {
        Self {
            state: PairerState::LastBefore(last_before::LastBeforeState::new(
                video_pid,
                klv_pid,
                freshness_ticks,
            )),
            stats: PairerStats::default(),
        }
    }

    /// Feed one demux event. Returns 0+ outputs in feed-time order.
    pub fn feed(&mut self, event: DemuxEvent) -> Vec<PairerOutput> {
        let outputs = match &mut self.state {
            PairerState::Nearest(s) => s.feed(event),
            PairerState::LastBefore(s) => s.feed(event),
        };
        for o in &outputs {
            match o {
                PairerOutput::Paired { .. } => self.stats.paired += 1,
                PairerOutput::UnpairedVideo(_) => self.stats.unpaired_video += 1,
                PairerOutput::UnpairedKlv(_) => self.stats.unpaired_klv += 1,
                PairerOutput::PassThrough(_) => self.stats.pass_through += 1,
            }
        }
        outputs
    }

    /// Drain remaining state at end-of-stream. Idempotent; subsequent
    /// `feed` calls work normally with no carryover.
    pub fn flush(&mut self) -> Vec<PairerOutput> {
        let outputs = match &mut self.state {
            PairerState::Nearest(s) => s.flush(),
            PairerState::LastBefore(s) => s.flush(),
        };
        for o in &outputs {
            match o {
                PairerOutput::Paired { .. } => self.stats.paired += 1,
                PairerOutput::UnpairedVideo(_) => self.stats.unpaired_video += 1,
                PairerOutput::UnpairedKlv(_) => self.stats.unpaired_klv += 1,
                PairerOutput::PassThrough(_) => self.stats.pass_through += 1,
            }
        }
        outputs
    }

    /// Snapshot the current counters.
    pub fn stats(&self) -> PairerStats {
        self.stats.clone()
    }

    /// Reset all counters to zero.
    pub fn reset_stats(&mut self) {
        self.stats = PairerStats::default();
    }
}
