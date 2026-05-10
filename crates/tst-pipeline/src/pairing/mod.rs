//! Opt-in KLV ↔ video pairer.
//!
//! Stateful transducer that ingests `DemuxEvent`s and emits typed
//! `PairerOutput`s. Two strategies:
//!
//! * [`Pairer::with_options`] — match each video AU against the KLV
//!   with nearest PTS, within a configured tolerance. Two modes:
//!   [`PairerMode::Realtime`] (zero buffer, eager emission) and
//!   [`PairerMode::Buffered`] (bounded arrival skew, bidirectional
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

pub use types::{KlvSample, PairerMode, PairerOptions, PairerOutput, PairerStats, VideoSample};

use std::time::Duration;
use tst_core::mpegts::demux::DemuxEvent;

/// Convert a `Duration` to MPEG-TS 90 kHz PTS ticks. Saturating —
/// inputs larger than `i64::MAX / 90_000` seconds clamp to `i64::MAX`.
fn duration_to_pts_ticks(d: Duration) -> i64 {
    // 90 kHz ticks: ticks = secs * 90_000. Use as_nanos to preserve
    // sub-second precision down to ~11 µs (one tick).
    let nanos = d.as_nanos();
    // 1 tick = 1/90_000 s = 11_111.111... ns. ticks = nanos * 90_000 / 1e9
    //                                               = nanos * 9 / 100_000.
    let ticks_u128 = nanos.saturating_mul(9) / 100_000;
    if ticks_u128 > i64::MAX as u128 {
        i64::MAX
    } else {
        ticks_u128 as i64
    }
}

/// Stateful KLV ↔ video pairer. Construct with one of the strategy
/// constructors; feed `DemuxEvent`s; collect `PairerOutput`s.
///
/// The pairer holds bounded internal state per its strategy. It is
/// video-driven: each `Sample::Video` event on the configured
/// `video_pid` produces exactly one `Paired` or `UnpairedVideo` output,
/// and each `Metadata` event on the configured `klv_pid` produces
/// exactly one `Paired` or `UnpairedKlv` output. Off-route events
/// surface as `PassThrough`.
///
/// # Closing
///
/// `Pairer` is a passive aggregator — it owns no transport and no OS
/// handles. Drop is the only shutdown and is trivially synchronous.
/// Call [`Self::flush`] before drop at end-of-stream to drain any
/// remaining buffered video AUs (a no-op in `PairerMode::Realtime`,
/// load-bearing in `PairerMode::Buffered`).
///
/// ## Per-language idiom
///
/// | Language | Idiom |
/// |----------|-------|
/// | Rust | `let _ = pairer.flush(); drop(pairer);` (or just let it fall out of scope) |
/// | Java | Drain via `flush()`, then let GC reclaim — no `AutoCloseable` needed |
/// | Kotlin | Drain via `flush()`, then let GC reclaim |
/// | Swift | `deinit` calls drop; explicit `flush()` before exit if `Buffered` mode |
/// | Python | `pairer.flush()` at end-of-stream; let GC reclaim |
/// | C | (deferred to per-binding plan — pairer C ABI not yet shipped) |
pub struct Pairer {
    state: PairerState,
    stats: PairerStats,
}

enum PairerState {
    Nearest(nearest::NearestState),
    LastBefore(last_before::LastBeforeState),
}

impl Pairer {
    /// Construct a nearest-PTS pairer with the given options.
    ///
    /// Replaces the pre-Phase-3 5-positional-arg `Pairer::nearest_pts`
    /// constructor. Field-style construction is unit-explicit
    /// (`Duration` instead of bare ticks) and translates cleanly to
    /// future C ABI / JNI / UniFFI surfaces.
    ///
    /// # Behavior
    ///
    /// Each video AU pairs against the KLV with nearest PTS, within
    /// `opts.tolerance`. The KLV history holds up to
    /// `opts.max_buffered_klv` entries (FIFO eviction on overflow).
    /// In [`PairerMode::Buffered`], up to `opts.max_buffered_video`
    /// video AUs are held while looking ahead for a within-tolerance
    /// match.
    ///
    /// # Panics
    ///
    /// Panics if `opts.max_buffered_klv == 0` or, in
    /// `PairerMode::Buffered`, `opts.max_buffered_video == 0`. A cap of
    /// zero entries is useless; the constructor refuses rather than
    /// emit `UnpairedVideo` for every input silently.
    ///
    /// # Example — realtime nearest-PTS pairer with a 300 ms tolerance
    ///
    /// `PairerOptions` is `#[non_exhaustive]`; construct via
    /// [`Default::default()`] and assign overrides.
    ///
    /// ```
    /// use std::time::Duration;
    /// use tst_pipeline::pairing::{Pairer, PairerMode, PairerOptions};
    ///
    /// let mut opts = PairerOptions::default();
    /// opts.mode = PairerMode::Realtime;
    /// opts.tolerance = Duration::from_millis(300);
    /// opts.max_buffered_klv = 32;
    /// opts.max_buffered_video = 32;
    ///
    /// let mut pairer = Pairer::with_options(
    ///     0x0100, // video PID
    ///     0x0102, // KLV PID
    ///     opts,
    /// );
    ///
    /// // Feed each `DemuxEvent` from a `DemuxReceiver` into `pairer.feed(...)`
    /// // and consume the resulting `PairerOutput`s. Call `pairer.flush()`
    /// // at end-of-stream to drain remaining buffered video AUs (a no-op
    /// // in `Realtime` mode but kept for symmetry with `Buffered`).
    /// let _stats = pairer.stats();
    /// ```
    pub fn with_options(video_pid: u16, klv_pid: u16, opts: PairerOptions) -> Self {
        assert!(
            opts.max_buffered_klv > 0,
            "PairerOptions::max_buffered_klv must be > 0"
        );
        let tolerance_ticks = duration_to_pts_ticks(opts.tolerance);
        let internal_mode = match opts.mode {
            PairerMode::Realtime => nearest::InternalMode::Realtime,
            PairerMode::Buffered { max_lag } => {
                assert!(
                    opts.max_buffered_video > 0,
                    "PairerOptions::max_buffered_video must be > 0 for Buffered mode"
                );
                // `max_lag` is the PTS-skew "wait window": how long a
                // buffered video can sit in the buffer (measured against
                // the newest observed KLV PTS) before forced emit. It is
                // independent of `tolerance` (the match window) but must
                // be at least as large — a max_lag smaller than tolerance
                // is nonsensical (you'd give up before tolerance is even
                // tested). Clamp up to preserve a sane minimum.
                let raw_max_lag_ticks = duration_to_pts_ticks(max_lag);
                let max_lag_ticks = raw_max_lag_ticks.max(tolerance_ticks);
                nearest::InternalMode::Buffered {
                    max_video_buffer: opts.max_buffered_video as usize,
                    max_lag_ticks,
                }
            }
        };
        let max_klv_history = opts.max_buffered_klv as usize;
        // `link_klv_to_video` is reserved on PairerOptions; not yet
        // wired through to the internal NearestState. Tracking for
        // follow-up.
        let _ = opts.link_klv_to_video;
        Self {
            state: PairerState::Nearest(nearest::NearestState::new(
                video_pid,
                klv_pid,
                tolerance_ticks,
                max_klv_history,
                internal_mode,
            )),
            stats: PairerStats::default(),
        }
    }

    /// Sample-and-hold: each video AU pairs with the most recent KLV
    /// where `klv.pts <= video.pts`. If `freshness` is `Some(d)`, emit
    /// `UnpairedVideo` when the held KLV is older than `d` behind the
    /// video; if `None`, attach regardless of staleness. Past-only by
    /// definition; no [`PairerMode`] knob applies.
    ///
    /// # Example
    ///
    /// ```
    /// use std::time::Duration;
    /// use tst_pipeline::pairing::Pairer;
    ///
    /// // 2 s freshness ceiling: drop pairing if held KLV is staler.
    /// let pairer = Pairer::last_before_pts(
    ///     0x0100,
    ///     0x0102,
    ///     Some(Duration::from_secs(2)),
    /// );
    /// let _ = pairer;
    /// ```
    pub fn last_before_pts(video_pid: u16, klv_pid: u16, freshness: Option<Duration>) -> Self {
        let freshness_ticks = freshness.map(duration_to_pts_ticks);
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

#[cfg(test)]
mod tests {
    use super::*;
    use tst_core::mpegts::demux::{
        AudioCodec, DiscontinuityKind, MetadataKind, NonConformantIssue, ProgramMap, SamplePayload,
        StreamId, StreamKind, VideoCodec, VideoPayload,
    };

    const VIDEO_PID: u16 = 0x100;
    const KLV_PID: u16 = 0x102;
    const OTHER_VIDEO_PID: u16 = 0x101;
    const AUDIO_PID: u16 = 0x110;

    fn make_pairer() -> Pairer {
        // 1 ms tolerance maps to 90 ticks @ 90 kHz. Tests below use deltas
        // of 0 (paired) and 10_000 ticks (unpaired) — both well-bracketed
        // by this tolerance.
        Pairer::with_options(
            VIDEO_PID,
            KLV_PID,
            PairerOptions {
                mode: PairerMode::Realtime,
                tolerance: Duration::from_millis(1),
                max_buffered_klv: 4,
                max_buffered_video: 4,
                link_klv_to_video: true,
            },
        )
    }

    #[test]
    fn program_map_passes_through() {
        let mut p = make_pairer();
        let pmt = DemuxEvent::ProgramMap(ProgramMap {
            program_number: 1,
            pcr_pid: VIDEO_PID,
            streams: Vec::new(),
            klv_links: Vec::new(),
        });
        let out = p.feed(pmt.clone());
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0], PairerOutput::PassThrough(e) if e == &pmt));
    }

    #[test]
    fn nonconformant_passes_through() {
        let mut p = make_pairer();
        let nc = DemuxEvent::NonConformant {
            stream: StreamId {
                pid: VIDEO_PID,
                kind: StreamKind::Video(VideoCodec::H264),
            },
            issue: NonConformantIssue::PusiMidPes,
        };
        let out = p.feed(nc.clone());
        assert!(matches!(&out[0], PairerOutput::PassThrough(_)));
    }

    #[test]
    fn discontinuity_passes_through() {
        let mut p = make_pairer();
        let d = DemuxEvent::Discontinuity {
            stream: StreamId {
                pid: VIDEO_PID,
                kind: StreamKind::Video(VideoCodec::H264),
            },
            kind: DiscontinuityKind::ContinuityJump {
                expected: 0x5,
                observed: 0x9,
            },
        };
        let out = p.feed(d);
        assert!(matches!(&out[0], PairerOutput::PassThrough(_)));
    }

    #[test]
    fn audio_sample_on_video_pid_passes_through() {
        let mut p = make_pairer();
        // Caller misconfigured: video_pid actually carries audio. Don't
        // fabricate a VideoSample; pass through.
        let audio = DemuxEvent::Sample {
            stream: StreamId {
                pid: VIDEO_PID,
                kind: StreamKind::Audio(AudioCodec::Aac),
            },
            pts: 0,
            dts: None,
            payload: SamplePayload::Audio {
                codec: AudioCodec::Aac,
                frames: vec![0xFF, 0xF1],
            },
        };
        let out = p.feed(audio);
        assert!(matches!(&out[0], PairerOutput::PassThrough(_)));
    }

    #[test]
    fn sample_on_klv_pid_passes_through() {
        let mut p = make_pairer();
        let v = DemuxEvent::Sample {
            stream: StreamId {
                pid: KLV_PID,
                kind: StreamKind::Video(VideoCodec::H264),
            },
            pts: 0,
            dts: None,
            payload: SamplePayload::Video {
                codec: VideoCodec::H264,
                payload: VideoPayload::Nals(Vec::new()),
            },
        };
        let out = p.feed(v);
        assert!(matches!(&out[0], PairerOutput::PassThrough(_)));
    }

    #[test]
    fn metadata_on_video_pid_passes_through() {
        let mut p = make_pairer();
        let m = DemuxEvent::Metadata {
            stream: StreamId {
                pid: VIDEO_PID,
                kind: StreamKind::KlvAsync,
            },
            pts: 0,
            kind: MetadataKind::KlvAsync,
            payload: Vec::new(),
        };
        let out = p.feed(m);
        assert!(matches!(&out[0], PairerOutput::PassThrough(_)));
    }

    #[test]
    fn video_on_other_pid_passes_through() {
        let mut p = make_pairer();
        let v = DemuxEvent::Sample {
            stream: StreamId {
                pid: OTHER_VIDEO_PID,
                kind: StreamKind::Video(VideoCodec::H265),
            },
            pts: 0,
            dts: None,
            payload: SamplePayload::Video {
                codec: VideoCodec::H265,
                payload: VideoPayload::Nals(Vec::new()),
            },
        };
        let out = p.feed(v);
        assert!(matches!(&out[0], PairerOutput::PassThrough(_)));
    }

    #[test]
    fn metadata_kind_klv_sync_au_cell_pairs_same_as_klv_async() {
        let mut p = make_pairer();
        // Spec §3.6 / cookbook recipe 12: nearest-PTS pairing treats
        // both KlvSyncAuCell and KlvAsync as KLV candidates.
        let sync_klv = DemuxEvent::Metadata {
            stream: StreamId {
                pid: KLV_PID,
                kind: StreamKind::KlvSync {
                    declared_link: None,
                },
            },
            pts: 50,
            kind: MetadataKind::KlvSyncAuCell {
                metadata_service_id: 0,
                sequence_number: 0,
                cell_fragment_indication:
                    tst_core::mpegts::au_cell::CellFragmentIndication::Complete,
                decoder_config_flag: false,
                random_access_indicator: true,
            },
            payload: vec![0xAA],
        };
        let video = DemuxEvent::Sample {
            stream: StreamId {
                pid: VIDEO_PID,
                kind: StreamKind::Video(VideoCodec::H264),
            },
            pts: 50,
            dts: None,
            payload: SamplePayload::Video {
                codec: VideoCodec::H264,
                payload: VideoPayload::Nals(Vec::new()),
            },
        };
        let _ = p.feed(sync_klv);
        let out = p.feed(video);
        assert!(matches!(&out[0], PairerOutput::Paired { .. }));
    }

    #[test]
    fn audio_pid_event_passes_through() {
        let mut p = make_pairer();
        let a = DemuxEvent::Sample {
            stream: StreamId {
                pid: AUDIO_PID,
                kind: StreamKind::Audio(AudioCodec::Aac),
            },
            pts: 0,
            dts: None,
            payload: SamplePayload::Audio {
                codec: AudioCodec::Aac,
                frames: Vec::new(),
            },
        };
        let out = p.feed(a);
        assert!(matches!(&out[0], PairerOutput::PassThrough(_)));
    }

    fn make_pairer_stats() -> Pairer {
        Pairer::with_options(
            VIDEO_PID,
            KLV_PID,
            PairerOptions {
                mode: PairerMode::Realtime,
                tolerance: Duration::from_millis(1),
                max_buffered_klv: 4,
                max_buffered_video: 4,
                link_klv_to_video: true,
            },
        )
    }

    fn klv_async_event(pid: u16, pts: i64) -> DemuxEvent {
        DemuxEvent::Metadata {
            stream: StreamId {
                pid,
                kind: StreamKind::KlvAsync,
            },
            pts,
            kind: MetadataKind::KlvAsync,
            payload: vec![0xAA],
        }
    }

    fn video_event_for_stats(pts: i64) -> DemuxEvent {
        DemuxEvent::Sample {
            stream: StreamId {
                pid: VIDEO_PID,
                kind: StreamKind::Video(VideoCodec::H264),
            },
            pts,
            dts: None,
            payload: SamplePayload::Video {
                codec: VideoCodec::H264,
                payload: VideoPayload::Nals(Vec::new()),
            },
        }
    }

    #[test]
    fn stats_starts_zero() {
        let p = make_pairer_stats();
        let s = p.stats();
        assert_eq!(s, PairerStats::default());
    }

    #[test]
    fn stats_increments_paired_and_unpaired_video() {
        let mut p = make_pairer_stats();
        let _ = p.feed(klv_async_event(KLV_PID, 0));
        let _ = p.feed(video_event_for_stats(0)); // Paired
        let _ = p.feed(video_event_for_stats(10_000)); // UnpairedVideo (no KLV in window)
        let s = p.stats();
        assert_eq!(s.paired, 1);
        assert_eq!(s.unpaired_video, 1);
    }

    #[test]
    fn stats_increments_unpaired_klv_on_eviction_and_flush() {
        let mut p = make_pairer_stats();
        // Fill history with 5 unused KLVs; max=4 so 1 evicts.
        for pts in [0, 10, 20, 30, 40] {
            let _ = p.feed(klv_async_event(KLV_PID, pts));
        }
        let s_after_evict = p.stats();
        assert_eq!(s_after_evict.unpaired_klv, 1);
        // Flush emits the remaining 4 unused.
        let _ = p.flush();
        let s_after_flush = p.stats();
        assert_eq!(s_after_flush.unpaired_klv, 5);
    }

    #[test]
    fn stats_increments_pass_through() {
        let mut p = make_pairer_stats();
        let pmt = DemuxEvent::ProgramMap(ProgramMap {
            program_number: 1,
            pcr_pid: VIDEO_PID,
            streams: Vec::new(),
            klv_links: Vec::new(),
        });
        let _ = p.feed(pmt);
        let s = p.stats();
        assert_eq!(s.pass_through, 1);
    }

    #[test]
    fn reset_stats_zeros_all_counters() {
        let mut p = make_pairer_stats();
        let _ = p.feed(klv_async_event(KLV_PID, 0));
        let _ = p.feed(video_event_for_stats(0));
        p.reset_stats();
        assert_eq!(p.stats(), PairerStats::default());
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;
    use tst_core::mpegts::demux::{
        MetadataKind, SamplePayload, StreamId, StreamKind, VideoCodec, VideoPayload,
    };

    const VIDEO_PID: u16 = 0x100;
    const KLV_PID: u16 = 0x102;

    #[derive(Debug, Clone)]
    enum SyntheticEvent {
        Video(i64),
        Klv(i64),
    }

    fn to_demux_event(e: &SyntheticEvent) -> DemuxEvent {
        match e {
            SyntheticEvent::Video(pts) => DemuxEvent::Sample {
                stream: StreamId {
                    pid: VIDEO_PID,
                    kind: StreamKind::Video(VideoCodec::H264),
                },
                pts: *pts,
                dts: None,
                payload: SamplePayload::Video {
                    codec: VideoCodec::H264,
                    payload: VideoPayload::Nals(Vec::new()),
                },
            },
            SyntheticEvent::Klv(pts) => DemuxEvent::Metadata {
                stream: StreamId {
                    pid: KLV_PID,
                    kind: StreamKind::KlvAsync,
                },
                pts: *pts,
                kind: MetadataKind::KlvAsync,
                payload: vec![0xAA],
            },
        }
    }

    fn arb_event() -> impl Strategy<Value = SyntheticEvent> {
        prop_oneof![
            (-1_000_000i64..1_000_000).prop_map(SyntheticEvent::Video),
            (-1_000_000i64..1_000_000).prop_map(SyntheticEvent::Klv),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn nearest_realtime_conservation(events in proptest::collection::vec(arb_event(), 0..200)) {
            // 1000 ticks @ 90 kHz ≈ 11.1 ms. Use a Duration that comfortably
            // brackets that; tolerance values aren't load-bearing for the
            // conservation-property test (any positive value works).
            let mut p = Pairer::with_options(
                VIDEO_PID,
                KLV_PID,
                PairerOptions {
                    mode: PairerMode::Realtime,
                    tolerance: Duration::from_millis(12),
                    max_buffered_klv: 16,
                    max_buffered_video: 16,
                    link_klv_to_video: true,
                },
            );
            let mut video_count = 0u64;
            let mut klv_count = 0u64;
            let mut paired = 0u64;
            let mut unpaired_video = 0u64;
            let mut unpaired_klv = 0u64;
            for e in &events {
                match e {
                    SyntheticEvent::Video(_) => video_count += 1,
                    SyntheticEvent::Klv(_) => klv_count += 1,
                }
                for o in p.feed(to_demux_event(e)) {
                    match o {
                        PairerOutput::Paired { .. } => paired += 1,
                        PairerOutput::UnpairedVideo(_) => unpaired_video += 1,
                        PairerOutput::UnpairedKlv(_) => unpaired_klv += 1,
                        PairerOutput::PassThrough(_) => prop_assert!(false, "no pass-through expected"),
                    }
                }
            }
            for o in p.flush() {
                match o {
                    PairerOutput::Paired { .. } => paired += 1,
                    PairerOutput::UnpairedVideo(_) => unpaired_video += 1,
                    PairerOutput::UnpairedKlv(_) => unpaired_klv += 1,
                    PairerOutput::PassThrough(_) => prop_assert!(false, "no pass-through expected"),
                }
            }
            // Every video is accounted for: either paired or unpaired.
            prop_assert_eq!(paired + unpaired_video, video_count);
            // A single KLV entry can pair with multiple videos (marked
            // used=true but kept in history). So paired > klv_count is
            // possible; the tighter bound is unpaired_klv <= klv_count.
            prop_assert!(unpaired_klv <= klv_count);
        }

        #[test]
        fn nearest_buffered_conservation(events in proptest::collection::vec(arb_event(), 0..200)) {
            // Pre-Phase-3 the second knob was `max_video_buffer: 8`. Under
            // PairerOptions, the same effect comes from setting
            // `max_buffered_video = 8`. The new `max_lag: Duration` knob
            // is a separate constraint; pick something comfortably large
            // so the count cap is the binding limit (matching the
            // pre-Phase-3 behavior under test).
            let mut p = Pairer::with_options(
                VIDEO_PID,
                KLV_PID,
                PairerOptions {
                    mode: PairerMode::Buffered { max_lag: Duration::from_secs(1) },
                    tolerance: Duration::from_millis(12),
                    max_buffered_klv: 16,
                    max_buffered_video: 8,
                    link_klv_to_video: true,
                },
            );
            let mut video_count = 0u64;
            let mut klv_count = 0u64;
            let mut paired = 0u64;
            let mut unpaired_video = 0u64;
            let mut unpaired_klv = 0u64;
            for e in &events {
                match e {
                    SyntheticEvent::Video(_) => video_count += 1,
                    SyntheticEvent::Klv(_) => klv_count += 1,
                }
                for o in p.feed(to_demux_event(e)) {
                    match o {
                        PairerOutput::Paired { .. } => paired += 1,
                        PairerOutput::UnpairedVideo(_) => unpaired_video += 1,
                        PairerOutput::UnpairedKlv(_) => unpaired_klv += 1,
                        PairerOutput::PassThrough(_) => prop_assert!(false, "no pass-through expected"),
                    }
                }
            }
            for o in p.flush() {
                match o {
                    PairerOutput::Paired { .. } => paired += 1,
                    PairerOutput::UnpairedVideo(_) => unpaired_video += 1,
                    PairerOutput::UnpairedKlv(_) => unpaired_klv += 1,
                    PairerOutput::PassThrough(_) => prop_assert!(false, "no pass-through expected"),
                }
            }
            // Every video is accounted for: either paired or unpaired.
            prop_assert_eq!(paired + unpaired_video, video_count);
            // A single KLV entry can pair with multiple videos (marked
            // used=true but kept in history). So paired > klv_count is
            // possible; the tighter bound is unpaired_klv <= klv_count.
            prop_assert!(unpaired_klv <= klv_count);
        }

        #[test]
        fn last_before_conservation(events in proptest::collection::vec(arb_event(), 0..200)) {
            let mut p = Pairer::last_before_pts(VIDEO_PID, KLV_PID, None);
            let mut video_count = 0u64;
            let mut klv_count = 0u64;
            let mut paired = 0u64;
            let mut unpaired_video = 0u64;
            let mut unpaired_klv = 0u64;
            for e in &events {
                match e {
                    SyntheticEvent::Video(_) => video_count += 1,
                    SyntheticEvent::Klv(_) => klv_count += 1,
                }
                for o in p.feed(to_demux_event(e)) {
                    match o {
                        PairerOutput::Paired { .. } => paired += 1,
                        PairerOutput::UnpairedVideo(_) => unpaired_video += 1,
                        PairerOutput::UnpairedKlv(_) => unpaired_klv += 1,
                        PairerOutput::PassThrough(_) => prop_assert!(false, "no pass-through expected"),
                    }
                }
            }
            for o in p.flush() {
                match o {
                    PairerOutput::Paired { .. } => paired += 1,
                    PairerOutput::UnpairedVideo(_) => unpaired_video += 1,
                    PairerOutput::UnpairedKlv(_) => unpaired_klv += 1,
                    PairerOutput::PassThrough(_) => prop_assert!(false, "no pass-through expected"),
                }
            }
            // last_before paired_count == video_count requires every
            // video to find a usable slot — which depends on KLV
            // ordering. The conservation invariant still holds:
            prop_assert_eq!(paired + unpaired_video, video_count);
            // Note: a single KLV can be marked used by many video
            // pairings before being displaced, so paired + unpaired_klv
            // != klv_count in general (paired counts video events, not
            // KLV events). The KLV-side invariant is tighter:
            prop_assert!(unpaired_klv <= klv_count);
        }
    }
}
