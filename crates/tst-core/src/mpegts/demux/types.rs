//! Demuxer supporting types — stats, options, per-program tracker, and
//! builder. Co-resident with the `Demuxer` impl in `demuxer.rs`
//! before the Phase 5 split (audit theme I).

use crate::mpegts::demux::event::{StreamInfo, StreamKind};
use crate::mpegts::demux::strict::StrictMode;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Stats snapshot for [`Demuxer`](crate::mpegts::demux::Demuxer). Used by
/// `tst_pipeline::DemuxReceiver` to compose its own `DemuxReceiverStats`;
/// also exposed publicly for callers using
/// [`Demuxer`](crate::mpegts::demux::Demuxer) directly.
///
/// Per-stream entries are created lazily as events are emitted — the
/// receiver discovers topology rather than configuring it up front. PSI
/// PIDs (PAT 0x0000, active PMT PID) get hardcoded labels "PAT" / "PMT".
#[must_use]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DemuxerStats {
    /// Number of `ProgramMap` events emitted (one per PMT version seen).
    pub program_maps_seen: u64,
    /// Number of distinct PMT version_number values seen, including the
    /// initial sighting. Resets to zero on `reset_stats`, so the next PMT
    /// always increments this counter.
    pub pmt_versions_seen: u64,
    /// Total discontinuity events emitted across all PIDs.
    pub discontinuities: u64,
    /// Total non-conformant events emitted across all PIDs.
    pub nonconformant: u64,
    /// Number of programs currently tracked (entries in the PAT that have
    /// been received). Reflects the live PAT — increases when a PAT version
    /// bump adds a program, decreases when one is removed.
    pub programs_seen: u32,
    /// Number of distinct subtitle PIDs the demuxer has seen at least one
    /// PES sample for. Increments on the first `SamplePayload::Subtitle`
    /// event per PID; resets to zero on `reset_stats`.
    pub subtitle_streams_seen: u32,
    /// Per-PID counters. Keys are PIDs. Entries are created on first event
    /// for a given PID; PSI PIDs (0x0000 for PAT, the PMT PID) are added
    /// with fixed "PAT"/"PMT" labels when a `ProgramMap` event fires.
    pub per_stream: BTreeMap<u16, crate::mpegts::stats::StreamStats>,
}

/// Default per-PID PES reassembly cap. 4 MiB accommodates 4K H.265 IDR
/// keyframes (typically 1–2 MB) with headroom, and matches the order of
/// magnitude of typical per-PID PES sizes in well-formed streams. Breach
/// surfaces as a `Discontinuity { kind: PesOversize }` event and the
/// partial PES on that PID is dropped.
pub(super) const DEFAULT_PES_CAP_PER_PID: usize = 4 * 1024 * 1024;
/// Default aggregate PES reassembly cap across all PIDs. 64 MiB defends
/// against a multi-PID flood scenario where each PID stays under its own
/// per-PID cap but the aggregate explodes. Breach surfaces as a
/// `Discontinuity { kind: PesTotalOversize }` event and all in-flight
/// partial PES on every PID are dropped.
pub(super) const DEFAULT_PES_CAP_TOTAL: usize = 64 * 1024 * 1024;

/// Caller-supplied overrides for the demuxer.
#[must_use]
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct DemuxerConfig {
    pub strict: StrictMode,
    /// Per-PID PES reassembly cap. `None` uses `DEFAULT_PES_CAP_PER_PID`
    /// (4 MiB). Tune up for ultra-high-bitrate streams whose IDR keyframes
    /// exceed the default; tune down for adversarial-input scenarios where
    /// faster failure (and a tighter memory bound) is preferable.
    pub pes_cap_per_pid: Option<usize>,
    /// Aggregate PES reassembly cap across all PIDs. `None` uses
    /// `DEFAULT_PES_CAP_TOTAL` (64 MiB). Tune up for streams with many
    /// concurrent high-bitrate PIDs; tune down to bound multi-PID flood
    /// memory growth in adversarial-input scenarios.
    pub pes_cap_total: Option<usize>,
    pub klv_link_overrides: Vec<(u16, u16)>,
    pub stream_kind_overrides: HashMap<u16, StreamKind>,
    /// When `true`, PSI section reassembly accepts continuation packets
    /// across continuity-counter jumps (today's permissive behavior —
    /// section either passes by luck or fails CRC). Default `false` is
    /// strict-correctness: drop the partial section on jump and emit
    /// `NonConformantIssue::PsiCcDiscontinuity`. Matches ffmpeg
    /// `mpegts.c:3118-3142`.
    pub lenient_psi_reassembly: bool,
    /// AV1 PES carriage mode the demuxer expects. Default
    /// [`crate::mpegts::mux::Av1CarriageMode::Mpeg2TsBinding`]
    /// (spec-conformant per the AV1-in-MPEG-2-TS binding).
    ///
    /// In `Mpeg2TsBinding` mode the demuxer expects PES
    /// `stream_id=0xBD` and `ts_open_bitstream_unit()` framing on each
    /// OBU. Violations surface as
    /// [`crate::mpegts::demux::NonConformantIssue::Av1WrongStreamId`]
    /// and [`crate::mpegts::demux::NonConformantIssue::Av1MissingTsObuFraming`].
    /// The demuxer falls back to raw-OBU parsing in lenient mode so the
    /// sample still surfaces.
    ///
    /// In `InteropRawObu` mode the demuxer accepts raw OBUs without the
    /// `ts_open_bitstream_unit` framing (matches ffmpeg / libaom /
    /// hls.js / mediamtx today) and does not raise the binding issues.
    pub av1_carriage: crate::mpegts::mux::Av1CarriageMode,
}

/// Per-program demuxer state. Crate-private — accessed only by `Demuxer`
/// and its `pub(crate) fn programs_for_test()` accessor for in-crate tests.
#[derive(Debug)]
pub(crate) struct ProgramTracker {
    pub program_number: u16,
    // Redundant with the HashMap key in `Demuxer::programs` (which is keyed
    // by `pmt_pid`); retained for symmetry with `program_number` and to
    // make `Debug` output self-describing.
    #[allow(dead_code)]
    pub pmt_pid: u16,
    pub pmt_version: Option<u8>,
    pub pcr_pid: Option<u16>,
    pub streams: Vec<StreamInfo>,
    /// PIDs that have already had a KLV stream-type-mismatch nonconformant
    /// emitted for the current PMT version. Cleared on PMT version bump.
    pub(crate) klv_mismatch_coalesce: HashSet<u16>,
}

/// Builder for [`Demuxer`](crate::mpegts::demux::Demuxer).
///
/// Construct via [`DemuxerBuilder::new`] or
/// [`DemuxerBuilder::default`], chain option methods, and call
/// [`DemuxerBuilder::build`] to produce a
/// [`Demuxer`](crate::mpegts::demux::Demuxer).
#[must_use]
#[derive(Debug, Default)]
pub struct DemuxerBuilder {
    options: DemuxerConfig,
}

impl DemuxerBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn strict(mut self, mode: StrictMode) -> Self {
        self.options.strict = mode;
        self
    }

    pub fn pes_cap_per_pid(mut self, bytes: usize) -> Self {
        self.options.pes_cap_per_pid = Some(bytes);
        self
    }

    pub fn pes_cap_total(mut self, bytes: usize) -> Self {
        self.options.pes_cap_total = Some(bytes);
        self
    }

    pub fn link_klv(mut self, klv_pid: u16, video_pid: u16) -> Self {
        self.options.klv_link_overrides.push((klv_pid, video_pid));
        self
    }

    pub fn treat_as(mut self, pid: u16, kind: StreamKind) -> Self {
        self.options.stream_kind_overrides.insert(pid, kind);
        self
    }

    /// Set the expected AV1 PES carriage mode. Default
    /// [`crate::mpegts::mux::Av1CarriageMode::Mpeg2TsBinding`] (spec-conformant).
    /// Set to `InteropRawObu` to match ffmpeg/libaom/hls.js senders.
    pub fn av1_carriage(mut self, mode: crate::mpegts::mux::Av1CarriageMode) -> Self {
        self.options.av1_carriage = mode;
        self
    }

    pub fn build(self) -> crate::mpegts::demux::Demuxer {
        crate::mpegts::demux::Demuxer::with_config(self.options)
    }
}

#[cfg(test)]
pub(crate) const fn default_pes_cap_per_pid() -> usize {
    DEFAULT_PES_CAP_PER_PID
}

#[cfg(test)]
pub(crate) const fn default_pes_cap_total() -> usize {
    DEFAULT_PES_CAP_TOTAL
}
