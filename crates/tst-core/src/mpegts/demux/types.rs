//! Demuxer supporting types — stats, options, per-program tracker, and
//! builder. Co-resident with the `Demuxer` impl in `demuxer.rs`
//! before the Phase 5 split (audit theme I).

use crate::mpegts::demux::event::{StreamInfo, StreamKind};
use crate::mpegts::demux::strict::StrictMode;
use hashbrown::HashSet;
// `stream_kind_overrides` is a public field of `DemuxerConfig`; keep it on
// `std::collections::HashMap` so the public API surface is unchanged. A later
// no_std step migrates this public type deliberately (with a baseline update).
use std::collections::{BTreeMap, HashMap};

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

/// Default per-PID AU cell reassembly buffer cap (1 MiB). Comfortable
/// for any realistic MISB sync-metadata AU (ST 0903 VMTI with hundreds
/// of target packs is ~hundreds of KB at most); well below the
/// per-PES default. Configurable via
/// [`DemuxerBuilder::au_cell_cap_per_pid`].
//
// Task 4 wires this into the reassembler; until then the const is
// surfaced only to docs (referenced via `Self::au_cell_cap_per_pid`).
#[allow(dead_code)]
pub(super) const DEFAULT_AU_CELL_CAP_PER_PID: usize = 1024 * 1024;

/// Caller-supplied overrides for the demuxer.
///
/// `Default` is hand-written (not derived) so `cfi_tolerance` can default
/// to `true`. Every other field uses its own `Default` impl.
#[must_use]
#[non_exhaustive]
#[derive(Debug, Clone)]
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
    /// Per-PID cap on the in-flight AU cell reassembly buffer.
    /// `None` uses `DEFAULT_AU_CELL_CAP_PER_PID` (1 MiB). When the
    /// buffered inner-byte total would exceed this, the demuxer drops
    /// the buffer and emits
    /// [`crate::mpegts::demux::NonConformantIssue::MultiCellAu`] with
    /// `reason = MultiCellAuReason::Overflow`. Tune up for streams with
    /// unusually large sync-metadata AUs; tune down for adversarial-input
    /// scenarios where faster failure is preferable.
    pub au_cell_cap_per_pid: Option<usize>,
    /// Tolerate sync-metadata AU cells that arrive with
    /// `cell_fragment_indication` bits set to `0b00` (middle) or `0b01`
    /// (last) when there is no active reassembly buffer for the PID.
    ///
    /// **Default `true`** — pragmatic for real-world STANAG 4609 streams.
    /// Corpus-wide validation across 251 captures (37 GB, multiple
    /// platforms) found ~99% of `NonConformant` events in the field are
    /// `MalformedAuCellCfiTolerated`: industry encoders default-initialize
    /// the field to zero and ship CFI=`0b00` (Middle) on single-cell AUs
    /// that should be `0b11` (Complete). MISB ST 1402.2 Appendix B lists
    /// the four bit patterns without semantic explanation, and no other
    /// public reference decoder (FFmpeg's `mpegtsenc.c`, GStreamer's
    /// `tsdemux.c::parse_pes_metadata_frame`, TSDuck, paretech/klvdata,
    /// jimcavoy/klvp) enforces CFI — the spec-strict reading lost its
    /// last enforcement battery, and producers ship malformed CFI bits
    /// without anything downstream catching it.
    ///
    /// When tolerance is on, the demuxer additionally validates the
    /// orphan cell's inner payload as a single complete KLV unit
    /// (SMPTE 336M UL prefix `06 0e 2b 34` followed by a BER length
    /// that describes exactly the available payload). If it passes, the
    /// cell is emitted as a
    /// [`crate::mpegts::demux::MetadataKind::KlvSyncAuCell`] with
    /// `cell_fragment_indication = Complete` AND a
    /// [`crate::mpegts::demux::NonConformantIssue::CfiTolerated`]
    /// diagnostic so the malformation remains visible to callers (this
    /// is what makes the corpus-wide pattern observable in the first
    /// place — tolerance recovers data without hiding the bug).
    ///
    /// Set to `false` for spec-strict conformance testing per
    /// H.222.0 V9 §2.12.4.2 Table 2-157: orphan Middle/Last cells then
    /// produce
    /// [`crate::mpegts::demux::NonConformantIssue::MultiCellAu`] with
    /// `reason = MultiCellAuReason::Orphan` and no metadata event. Use
    /// when validating a producer against the wire spec rather than
    /// consuming real-world traffic. (Note: this is asymmetric with
    /// `lenient_psi_reassembly`, which still defaults `false`/strict.
    /// The asymmetry is calibrated to corpus evidence — we have
    /// empirical proof the CFI bug is dominant in real traffic; we do
    /// not have equivalent evidence for PSI reassembly violations.)
    pub cfi_tolerance: bool,
}

impl Default for DemuxerConfig {
    fn default() -> Self {
        Self {
            strict: StrictMode::default(),
            pes_cap_per_pid: None,
            pes_cap_total: None,
            klv_link_overrides: Vec::new(),
            stream_kind_overrides: HashMap::new(),
            lenient_psi_reassembly: false,
            av1_carriage: crate::mpegts::mux::Av1CarriageMode::default(),
            au_cell_cap_per_pid: None,
            // Tolerance-by-default — see field rustdoc above.
            cfi_tolerance: true,
        }
    }
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

    /// Set the per-PID AU cell reassembly buffer cap. See
    /// [`DemuxerConfig::au_cell_cap_per_pid`].
    pub fn au_cell_cap_per_pid(mut self, bytes: usize) -> Self {
        self.options.au_cell_cap_per_pid = Some(bytes);
        self
    }

    /// Enable opt-in tolerance for sync-metadata AU cells whose
    /// `cell_fragment_indication` bits are set to `0b00` (middle) or
    /// `0b01` (last) without a prior First cell. See
    /// [`DemuxerConfig::cfi_tolerance`].
    pub fn cfi_tolerance(mut self, enable: bool) -> Self {
        self.options.cfi_tolerance = enable;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cfi_tolerance_is_true() {
        // Tolerance-by-default posture: corpus-wide validation showed
        // the producer-side CFI=00-on-single-cell-AU bug is dominant in
        // real STANAG 4609 traffic (~99% of NonConformant events). The
        // CfiTolerated diagnostic still fires so the malformation stays
        // visible to validators. Receivers can set `cfi_tolerance: false`
        // explicitly for spec-strict conformance testing.
        assert!(DemuxerConfig::default().cfi_tolerance);
    }

    #[test]
    fn builder_sets_cfi_tolerance() {
        let builder = DemuxerBuilder::new().cfi_tolerance(true);
        let config = builder.options;
        assert!(config.cfi_tolerance);
    }

    #[test]
    fn builder_can_toggle_cfi_tolerance_off() {
        let builder = DemuxerBuilder::new()
            .cfi_tolerance(true)
            .cfi_tolerance(false);
        assert!(!builder.options.cfi_tolerance);
    }
}
