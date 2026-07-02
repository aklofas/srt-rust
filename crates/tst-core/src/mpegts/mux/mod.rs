//! MuxSender-side MPEG-TS muxer.
//!
//! The public surface is `Muxer`, `MuxerConfig`, `VideoCodec`,
//! `KlvStreamType`. Internal helpers live in `ts`, `psi`, `pes` submodules.
//!
//! Re-export note: `Muxer`, `VideoCodec`, and `KlvStreamType` are re-exported
//! at the crate root (`tst_core::Muxer` etc.). `MuxerConfig` deliberately is
//! not — callers reach it via `mpegts::mux::MuxerConfig` so the construction
//! site is visually distinct from the SRT `SocketConfig` / `ListenerConfig`
//! already at the crate root. Don't "symmetry-fix" this.

pub(crate) mod pes;
pub(crate) mod psi;
pub(crate) mod ts;

mod state;
mod scheduling;
mod stats_accounting;
mod push_audio;
mod push_data;
mod push_klv;
mod push_subtitle;
mod push_video;

use crate::error::MuxError;
use alloc::vec::Vec;

mod types;
pub use types::*;

mod config;
pub use config::*;
pub use stats_accounting::MuxerStats;

/// Spec-domain tier of muxer errors.
///
/// The [`crate::error::MuxError`] enum (canonical home at
/// [`crate::error::MuxError`]) is re-exported here under the `_detail`
/// convention to signal "spec-knowledge tier; production binding code
/// should prefer [`crate::error::MuxError::kind()`] for
/// action-discriminating dispatch".
///
/// The underscore prefix follows the workspace convention for re-export
/// modules that signal "opt into the spec-domain tier" at the import site:
///
/// ```
/// use tst_core::mpegts::mux::_detail::MuxError;
///
/// fn diagnose_klv(e: &MuxError) -> Option<String> {
///     match e {
///         MuxError::KlvTooLarge { size, max } => Some(format!(
///             "KLV LS exceeds PES cap: {size} > {max}"
///         )),
///         _ => None,
///     }
/// }
/// ```
///
/// Equivalent to importing from the canonical path [`crate::error::MuxError`];
/// the re-export exists for documentary intent (the underscore signals
/// "I know I'm pattern-matching the spec-domain variants and I accept
/// future variant additions").
///
/// # Stability
///
/// The inner variant set is `#[non_exhaustive]` and may grow.
/// New variants WILL be added without a major version bump;
/// pattern-matching code here should include a wildcard arm. If you
/// only need action-discriminating categorization, prefer the
/// coarse-tier [`crate::error::MuxSenderErrorKind`] enum via
/// [`crate::error::MuxError::kind()`].
pub mod _detail {
    /// The canonical [`crate::error::MuxError`].
    pub use crate::error::MuxError;
}

// Re-exported through `super::` / `super::*` by sibling modules (scheduling.rs
// uses `super::StreamType`; the tests/ files use `super::*` glob). Keep here
// even though mod.rs itself no longer references these directly — the
// `Muxer::new` body that did was extracted into `state.rs`.
#[allow(unused_imports)]
use crate::mpegts::common::{StreamType, StreamTypeCode};
use alloc::collections::{BTreeMap, VecDeque};

use self::pes::MAX_PES_HEADER_SIZE;
use self::state::{
    AudioStreamState, DataStreamState, KlvStreamState, SubtitleStreamState, VideoStreamState,
};
use self::ts::ContinuityCounters;

/// MuxSender-side MPEG-TS muxer.
///
/// Construct with `Muxer::new(config)`, push encoded frames via `push_video`
/// and `push_klv`, then drain TS packets with `pull`. The muxer is
/// deterministic — output is a function of inputs only, not wall-clock time.
///
/// # Closing
///
/// `Muxer` is a passive aggregator — it owns no transport and no OS
/// handles. Drop is the only shutdown and is trivially synchronous.
/// Call [`Self::pull`] in a loop until it returns `0` before drop to
/// drain any TS packets sitting in the internal queue; bytes left in
/// the queue at drop are discarded.
///
/// ## Per-language idiom
///
/// | Language | Idiom |
/// |----------|-------|
/// | Rust | `while muxer.pull(&mut buf) > 0 { /* ... */ } drop(muxer);` (or just let it fall out of scope) |
/// | Java | Drain via `pull()`, then let GC reclaim — no `AutoCloseable` needed |
/// | Kotlin | Drain via `pull()`, then let GC reclaim |
/// | Swift | `deinit` calls drop; explicit drain via `pull()` before exit |
/// | Python | Drain via `pull()` at end-of-stream; let GC reclaim |
/// | C | `tst_muxer_close(muxer)` — releases the muxer; caller is responsible for prior drain |
pub struct Muxer {
    config: MuxerConfig,

    /// Per-program pre-composed PMT descriptor bytes.
    /// `pmt_descriptor_caches[prog_idx][stream_idx]` = concatenated TLVs for
    /// that stream (KLVA auto-emit + caller-supplied). Indexed parallel to
    /// `config.programs[prog_idx].streams`. Borrowed at PMT emission time;
    /// never reallocated after construction.
    pmt_descriptor_caches: Vec<Vec<Vec<u8>>>,

    /// Per-program video stream state. `video_streams[prog_idx]` is the
    /// list of video streams for program `prog_idx`, in declaration order.
    /// `VideoStreamHandle::unpack()` → `(prog_idx, within_idx)` indexes here.
    video_streams: Vec<Vec<VideoStreamState>>,

    /// Per-program KLV stream state. Same indexing as `video_streams`.
    klv_streams: Vec<Vec<KlvStreamState>>,

    /// Per-program audio stream state. Same indexing as `video_streams`.
    /// `AudioStreamHandle::unpack()` → `(prog_idx, within_idx)` indexes here.
    audio_streams: Vec<Vec<AudioStreamState>>,

    /// Per-program subtitle stream state. Same indexing as `video_streams`.
    /// `SubtitleStreamHandle::unpack()` → `(prog_idx, within_idx)` indexes here.
    subtitle_streams: Vec<Vec<SubtitleStreamState>>,

    /// Per-program data stream state. Same indexing as `video_streams`.
    /// `DataStreamHandle::unpack()` → `(prog_idx, within_idx)` indexes here.
    data_streams: Vec<Vec<DataStreamState>>,

    /// Per-program resolved PCR PID. Indexed parallel to `config.programs`.
    pcr_pids: Vec<u16>,

    pcr_interval_27mhz: u64,
    psi_interval_90khz: i64,

    queue: VecDeque<[u8; 188]>,
    counters: ContinuityCounters,

    /// Per-program last PSI emission PTS, masked to 33 bits. None until first.
    /// Indexed parallel to `config.programs`.
    psi_last: Vec<Option<u64>>,

    /// Per-program last PCR emission value at 27 MHz. None until first.
    /// Indexed parallel to `config.programs`.
    pcr_last: Vec<Option<u64>>,

    // ── Stats counters ────────────────────────────────────────────────────
    ts_packets_emitted: u64,
    ts_bytes_emitted: u64,
    /// Keyed by PID. Populated eagerly at construction for every configured
    /// stream; only the `items` / `bytes` / `discontinuities` fields change
    /// at runtime. `pid` and `stream_type` are set at construction and
    /// never modified.
    per_stream: BTreeMap<u16, crate::mpegts::stats::StreamStats>,

    /// Per-PID codec-specific counters. Allocated lazily on first push
    /// for a PID whose StreamSpec falls into a counted family. Subtitle
    /// PIDs and audio PIDs with LATM/AC-3 codecs do NOT get an entry.
    stream_codec_counters: BTreeMap<u16, crate::mpegts::stats::StreamCodecCounters>,

    /// Scratch buffer reused across push_video / push_klv / push_audio /
    /// push_subtitle calls. Cleared and refilled each AU; avoids one
    /// per-AU heap allocation on the hot path. Grows to the largest AU seen
    /// and stays there — typical size is MAX_PES_HEADER_SIZE + payload.
    pes_scratch: Vec<u8>,
}

impl Muxer {
    /// Construct and validate.
    pub fn new(config: MuxerConfig) -> Result<Self, MuxError> {
        config.validate()?;

        let n_programs = config.programs.len();
        let pcr_interval_27mhz = (config.pcr_interval_ms as u64) * 27_000;
        let psi_interval_90khz = (config.psi_interval_ms as i64) * 90;

        // Per-program state vectors built in a single pass over programs.
        // The heavy lifting (stream-state collection, PCR PID resolution,
        // descriptor cache assembly, per-stream stats initialization) lives
        // in `state.rs` as `pub(super)` helpers so this constructor stays a
        // thin coordinator.
        let mut video_streams: Vec<Vec<VideoStreamState>> = Vec::with_capacity(n_programs);
        let mut klv_streams: Vec<Vec<KlvStreamState>> = Vec::with_capacity(n_programs);
        let mut audio_streams: Vec<Vec<AudioStreamState>> = Vec::with_capacity(n_programs);
        let mut subtitle_streams: Vec<Vec<SubtitleStreamState>> = Vec::with_capacity(n_programs);
        let mut data_streams: Vec<Vec<DataStreamState>> = Vec::with_capacity(n_programs);
        let mut pcr_pids: Vec<u16> = Vec::with_capacity(n_programs);
        let mut pmt_descriptor_caches: Vec<Vec<Vec<u8>>> = Vec::with_capacity(n_programs);
        let mut per_stream: BTreeMap<u16, crate::mpegts::stats::StreamStats> = BTreeMap::new();

        for prog in &config.programs {
            let (video, klv, audio, subtitle, data) = state::collect_stream_states(prog);
            let pcr_pid = state::resolve_pcr_pid(prog);
            let cache = state::build_pmt_descriptor_cache(prog);
            state::initialize_stats(
                prog,
                &video,
                &klv,
                &audio,
                &subtitle,
                &data,
                &mut per_stream,
            );

            video_streams.push(video);
            klv_streams.push(klv);
            audio_streams.push(audio);
            subtitle_streams.push(subtitle);
            data_streams.push(data);
            pcr_pids.push(pcr_pid);
            pmt_descriptor_caches.push(cache);
        }

        Ok(Self {
            config,
            pmt_descriptor_caches,
            video_streams,
            klv_streams,
            audio_streams,
            subtitle_streams,
            data_streams,
            pcr_pids,
            pcr_interval_27mhz,
            psi_interval_90khz,
            queue: VecDeque::with_capacity(64),
            counters: ContinuityCounters::new(),
            psi_last: vec![None; n_programs],
            pcr_last: vec![None; n_programs],
            ts_packets_emitted: 0,
            ts_bytes_emitted: 0,
            per_stream,
            stream_codec_counters: BTreeMap::new(),
            // 8 KiB heuristic: covers a typical small AU without reallocation;
            // grows automatically on first oversized frame and stays there.
            pes_scratch: Vec::with_capacity(MAX_PES_HEADER_SIZE + 8192),
        })
    }

    /// Drain ready TS packets into `out`.
    ///
    /// Returns the number of bytes written: 0 or a positive multiple of 188.
    /// `0` indicates either an empty queue or `out.len() < 188`. Pull is
    /// infallible — there are no failure modes that don't already surface
    /// at `push_video` / `push_klv` time (buffer-full, validation).
    ///
    /// # C ABI
    ///
    /// `tst_muxer_pull` — see `bindings/c/include/tstrans.h`.
    pub fn pull(&mut self, out: &mut [u8]) -> usize {
        if out.len() < 188 {
            return 0;
        }
        let max_packets = (out.len() / 188).min(self.queue.len());
        for i in 0..max_packets {
            let pkt = self.queue.pop_front().expect("checked count");
            out[i * 188..(i + 1) * 188].copy_from_slice(&pkt);
        }
        let n = max_packets * 188;
        self.ts_packets_emitted += max_packets as u64;
        self.ts_bytes_emitted += n as u64;
        n
    }

    /// Number of 188-byte TS packets currently queued in the muxer's
    /// internal output buffer awaiting [`Muxer::pull`]. This is a live
    /// gauge — non-zero between a `push_*` call and the subsequent
    /// drain. Compared against [`Muxer::capacity_packets`] this gives
    /// the back-pressure ratio used by `tst_pipeline::MuxSender`'s
    /// observability hooks.
    pub fn pending_packets(&self) -> u64 {
        self.queue.len() as u64
    }

    /// The configured queue capacity in 188-byte TS packets — a snapshot
    /// of `MuxerConfig::buffer_packets`. A `push_*` that would push the
    /// queue past this cap returns [`crate::error::MuxError::BufferFull`].
    #[must_use]
    pub fn capacity_packets(&self) -> u64 {
        self.config.buffer_packets as u64
    }

    /// Return the resolved PCR PID for program at `prog_idx` (0-based index).
    /// Returns `None` if `prog_idx` is out of range.
    #[cfg(test)]
    pub(crate) fn pcr_pid_for_program(&self, prog_idx: usize) -> Option<u16> {
        self.pcr_pids.get(prog_idx).copied()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────
//
// The 2500+ LoC of inline test code is split into focused files under
// `tests/`. Each file is declared as a direct child of this module via
// `#[path]` so that `use super::*` inside each file resolves against
// `mpegts::mux` — the same scope the original inline blocks had.
//
// DO NOT add a `mod tests { ... }` wrapper here. The `#[path]` attribute
// makes each file a DIRECT child of `mux`, preserving the same
// `use super::*` scope that the inline tests relied on (including
// `pub(super)` helpers like `validate_language_code` from `config.rs`).

#[cfg(test)]
#[path = "tests/config.rs"]
mod tests_config;

#[cfg(test)]
#[path = "tests/handles.rs"]
mod tests_handles;

#[cfg(test)]
#[path = "tests/push.rs"]
mod tests_push;

#[cfg(test)]
#[path = "tests/subtitle.rs"]
mod tests_subtitle;

#[cfg(test)]
#[path = "tests/validation.rs"]
mod tests_validation;

#[cfg(test)]
#[path = "tests/stats.rs"]
mod tests_stats;
