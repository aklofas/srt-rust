//! Sender-side MPEG-TS muxer.
//!
//! See `docs/specs/2026-05-01-srt-core-mpegts-mux-design.md` for the full
//! design. The public surface is `Muxer`, `Config`, `VideoCodec`,
//! `KlvStreamType`. Internal helpers live in `ts`, `psi`, `pes` submodules.
//!
//! Re-export note: `Muxer`, `VideoCodec`, and `KlvStreamType` are re-exported
//! at the crate root (`srt_core::Muxer` etc.). `Config` deliberately is not —
//! callers reach it via `mpegts::mux::Config` so the construction site is
//! visually distinct from the SRT `SocketConfig` / `ListenerConfig` already
//! at the crate root. Don't "symmetry-fix" this.

pub(crate) mod pes;
pub(crate) mod psi;
pub(crate) mod ts;

use crate::error::MuxError;
use crate::mpegts::common::pid;

/// Video codec carried by the muxer's video PID.
///
/// Drives the PMT `stream_type` byte: 0x1B for H.264 / AVC,
/// 0x24 for H.265 / HEVC. Both supported; mid-stream codec change is
/// out of scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    H265,
}

/// Transport-stream type for the KLV PID.
///
/// `PrivateData` (PMT stream_type 0x06) is the broadly-recognized form;
/// `SynchronousMetadata` (0x15) is strict ST 1402 sync.
///
/// Whether the KLV PES carries a PTS is controlled separately via the
/// `carries_pts` field in `StreamSpec::Klv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KlvStreamType {
    PrivateData,
    SynchronousMetadata,
}

/// One elementary stream in the muxer's output TS.
///
/// [`Config::validate`] caps at 16 video + 16 KLV streams, with at least
/// one of either kind required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamSpec {
    Video {
        /// PID for the video PES stream. Must be in `0x0010..=0x1FFE`.
        pid: u16,
        /// Video codec — drives PMT stream_type (0x1B for H.264, 0x24 for H.265).
        codec: VideoCodec,
    },
    Klv {
        /// PID for the KLV metadata stream. Must be in `0x0010..=0x1FFE`,
        /// distinct from any video PID.
        pid: u16,
        /// Transport-stream type — drives the PMT stream_type byte
        /// (0x06 PrivateData / 0x15 SynchronousMetadata).
        stream_type: KlvStreamType,
        /// Whether the KLV PES carries a PTS in its header.
        /// `false` = ST 1402 async (no PTS).
        /// `true`  = sync KLV (PTS aligns with video).
        /// `SynchronousMetadata` + `false` is invalid.
        carries_pts: bool,
    },
}

impl StreamSpec {
    pub(crate) fn pid(&self) -> u16 {
        match self {
            StreamSpec::Video { pid, .. } => *pid,
            StreamSpec::Klv { pid, .. } => *pid,
        }
    }
}

/// Opaque handle to a configured video stream on a `Muxer`.
///
/// Obtained from [`Muxer::video_handles`] / [`Muxer::video_stream_handle`].
/// Handles are valid only on the muxer that produced them; passing a handle
/// to a different muxer is rejected with [`MuxError::InvalidStreamHandle`].
///
/// The internal index is the ordinal position among video streams in
/// [`Config::streams`] (filtered to `StreamSpec::Video` only). Callers can
/// rely on the handles being assigned in the order video streams were
/// added to the builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoStreamHandle(usize);

/// Opaque handle to a configured KLV stream on a `Muxer`.
///
/// Same semantics as [`VideoStreamHandle`] but for KLV streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KlvStreamHandle(usize);

impl VideoStreamHandle {
    pub fn new(index: usize) -> Self {
        Self(index)
    }
    pub fn index(self) -> usize {
        self.0
    }
    #[cfg(test)]
    pub(crate) fn for_test(index: usize) -> Self {
        Self(index)
    }
}

impl KlvStreamHandle {
    pub fn new(index: usize) -> Self {
        Self(index)
    }
    pub fn index(self) -> usize {
        self.0
    }
    #[cfg(test)]
    pub(crate) fn for_test(index: usize) -> Self {
        Self(index)
    }
}

/// Muxer construction parameters.
///
/// **Multi-stream.** [`Config::validate`] enforces at most 16 Video and
/// 16 Klv streams, with at least one of either kind required.
///
/// Construct with [`Config::builder()`] for ergonomic chaining, or directly
/// with field updates over [`Config::default()`] for the canonical
/// single-video-plus-single-KLV case.
#[derive(Debug, Clone)]
pub struct Config {
    /// Elementary streams the muxer carries. ≤16 Video, ≤16 Klv, ≥1 of either.
    pub streams: Vec<StreamSpec>,

    /// PID carrying the PCR. `None` = use the first video stream's PID, or
    /// the first KLV stream's PID if no video stream is configured.
    pub pcr_pid: Option<u16>,

    /// PCR re-emission interval, in milliseconds. Default 40. Validation 1..=100.
    pub pcr_interval_ms: u32,

    /// PAT/PMT re-emission interval, in milliseconds. Default 100. Validation >= 10.
    pub psi_interval_ms: u32,

    /// Maximum buffered TS packets before push returns `BufferFull`.
    /// Default 10000 (~1.88 MB, ~600 ms at 25 Mbps). Validation: >= 10.
    pub buffer_packets: usize,

    /// Per-stream caller-supplied PMT descriptors. Outer Vec indexed by
    /// `streams[i]`; inner Vec is the descriptor list for that stream
    /// (each inner Vec<u8> is one complete descriptor TLV — tag + length
    /// byte + body, as built by the helpers in [`crate::mpegts::descriptors`]).
    /// Empty by default — populate via
    /// [`ConfigBuilder::stream_descriptors_for_video`] /
    /// `_for_klv` / `_for_stream`.
    ///
    /// Hand-built `Config` callers must keep `stream_descriptors.len()
    /// == streams.len()`; [`Config::validate`] enforces this. Outer-Vec
    /// growth happens in `ConfigBuilder::build` to match the final
    /// stream set.
    pub stream_descriptors: Vec<Vec<Vec<u8>>>,
}

impl Default for Config {
    fn default() -> Self {
        // Defaults: H.264 video at 0x1011, KLV PrivateData at 0x1031,
        // async KLV (no PTS), PCR pinned to video.
        Self {
            streams: vec![
                StreamSpec::Video {
                    pid: 0x1011,
                    codec: VideoCodec::H264,
                },
                StreamSpec::Klv {
                    pid: 0x1031,
                    stream_type: KlvStreamType::PrivateData,
                    carries_pts: false,
                },
            ],
            pcr_pid: None,
            pcr_interval_ms: 40,
            psi_interval_ms: 100,
            buffer_packets: 10_000,
            stream_descriptors: vec![Vec::new(), Vec::new()],
        }
    }
}

impl Config {
    /// Start a new builder. Equivalent to `ConfigBuilder::default()`.
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }

    /// Validate the configuration. Returns `Err(MuxError::InvalidConfig)`
    /// with a static message describing the failed rule.
    pub fn validate(&self) -> Result<(), MuxError> {
        // ≥1 stream of either kind.
        if self.streams.is_empty() {
            return Err(MuxError::InvalidConfig(
                "at least one stream (video or klv) is required",
            ));
        }

        // Cap at 16 video + 16 KLV streams. Well above realistic
        // gimbaled-platform topologies (EO + IR + maybe IR-narrow + a
        // depth channel = 4 in the wild today). Lift trivially if asked.
        const VIDEO_CAP: usize = 16;
        const KLV_CAP: usize = 16;
        let mut video_count = 0;
        let mut klv_count = 0;
        for s in &self.streams {
            match s {
                StreamSpec::Video { .. } => video_count += 1,
                StreamSpec::Klv { .. } => klv_count += 1,
            }
        }
        if video_count > VIDEO_CAP {
            return Err(MuxError::TooManyVideoStreams {
                count: video_count,
                cap: VIDEO_CAP,
            });
        }
        if klv_count > KLV_CAP {
            return Err(MuxError::TooManyKlvStreams {
                count: klv_count,
                cap: KLV_CAP,
            });
        }

        // Per-stream validation.
        for s in &self.streams {
            match s {
                StreamSpec::Video { pid, .. } => {
                    if !pid::is_user_pid(*pid) {
                        return Err(MuxError::InvalidConfig(
                            "video pid must be in 0x0010..=0x1FFE",
                        ));
                    }
                }
                StreamSpec::Klv {
                    pid,
                    stream_type,
                    carries_pts,
                } => {
                    if !pid::is_user_pid(*pid) {
                        return Err(MuxError::InvalidConfig(
                            "klv pid must be in 0x0010..=0x1FFE",
                        ));
                    }
                    if *stream_type == KlvStreamType::SynchronousMetadata && !*carries_pts {
                        return Err(MuxError::InvalidConfig(
                            "klv stream_type=SynchronousMetadata requires carries_pts=true",
                        ));
                    }
                }
            }
        }

        // PIDs must be distinct.
        for (i, s1) in self.streams.iter().enumerate() {
            for s2 in &self.streams[i + 1..] {
                if s1.pid() == s2.pid() {
                    return Err(MuxError::InvalidConfig("stream PIDs must all be distinct"));
                }
            }
        }

        // pcr_pid (if specified) must equal a configured stream's PID.
        if let Some(pcr) = self.pcr_pid {
            if !self.streams.iter().any(|s| s.pid() == pcr) {
                return Err(MuxError::InvalidConfig(
                    "pcr_pid must equal a configured stream PID",
                ));
            }
        }

        if !(1..=100).contains(&self.pcr_interval_ms) {
            return Err(MuxError::InvalidConfig(
                "pcr_interval_ms must be in 1..=100",
            ));
        }
        if self.psi_interval_ms < 10 {
            return Err(MuxError::InvalidConfig("psi_interval_ms must be >= 10"));
        }
        if self.buffer_packets < 10 {
            return Err(MuxError::InvalidConfig("buffer_packets must be >= 10"));
        }

        // stream_descriptors must be the same length as streams. Hand-built
        // Config callers must keep them in sync; ConfigBuilder::build does
        // this automatically.
        if self.stream_descriptors.len() != self.streams.len() {
            return Err(MuxError::InvalidConfig(
                "stream_descriptors.len() must equal streams.len()",
            ));
        }

        // Per-stream descriptor well-formedness: each TLV must declare a
        // length that matches its slice length, and the length byte itself
        // must be ≤ 253 (so total descriptor bytes ≤ 255).
        for (si, descs) in self.stream_descriptors.iter().enumerate() {
            for (di, tlv) in descs.iter().enumerate() {
                if tlv.len() < 2 {
                    return Err(MuxError::MalformedDescriptor {
                        stream_index: si,
                        descriptor_index: di,
                        reason: "descriptor TLV must be at least 2 bytes (tag + length)",
                    });
                }
                let declared = tlv[1] as usize;
                if declared != tlv.len() - 2 {
                    return Err(MuxError::MalformedDescriptor {
                        stream_index: si,
                        descriptor_index: di,
                        reason: "length byte does not match payload length",
                    });
                }
                if declared > 253 {
                    return Err(MuxError::MalformedDescriptor {
                        stream_index: si,
                        descriptor_index: di,
                        reason: "descriptor body length must fit in u8 (max 253 useful bytes)",
                    });
                }
            }
        }

        // PMT must fit in one TS packet. Sum 5 ES-header bytes per stream
        // + auto-emitted descriptor bytes (KLVA Registration on KLV PIDs)
        // + caller bytes. The 17-byte fixed PMT overhead (header + CRC +
        // PCR/info length) leaves 166 bytes for the ES loop.
        let mut total_es_loop_bytes = 0usize;
        for (i, spec) in self.streams.iter().enumerate() {
            let auto_bytes = match spec {
                StreamSpec::Klv {
                    stream_type: KlvStreamType::PrivateData,
                    ..
                } => 6, // KLVA Registration descriptor body
                _ => 0,
            };
            // If caller supplied a Registration descriptor on a KLV PID,
            // auto-emit is suppressed (Task 7). Detect that here so the
            // validation budget matches what the muxer actually emits.
            let caller_overrides_klva = matches!(spec, StreamSpec::Klv { .. })
                && self.stream_descriptors[i]
                    .iter()
                    .any(|tlv| !tlv.is_empty() && tlv[0] == 0x05);
            let effective_auto = if caller_overrides_klva { 0 } else { auto_bytes };
            let caller_bytes: usize = self.stream_descriptors[i].iter().map(Vec::len).sum();
            total_es_loop_bytes += 5 + effective_auto + caller_bytes;
        }
        if total_es_loop_bytes > crate::mpegts::descriptors::MAX_DESCRIPTOR_LOOP_PER_PMT {
            return Err(MuxError::PmtTooLarge {
                used_bytes: total_es_loop_bytes,
                max_bytes: crate::mpegts::descriptors::MAX_DESCRIPTOR_LOOP_PER_PMT,
            });
        }

        Ok(())
    }

    /// Resolve the PCR PID. If `pcr_pid` is `None`:
    /// - prefer the first video stream's PID;
    /// - if no video stream, use the first KLV stream's PID.
    ///
    /// Caller MUST have called `validate()` first; this helper assumes ≥1 stream.
    pub(crate) fn resolved_pcr_pid(&self) -> u16 {
        if let Some(pid) = self.pcr_pid {
            return pid;
        }
        if let Some(pid) = self.streams.iter().find_map(|s| match s {
            StreamSpec::Video { pid, .. } => Some(*pid),
            _ => None,
        }) {
            return pid;
        }
        // No video — use the first KLV stream. validate() guarantees ≥1.
        self.streams
            .iter()
            .find_map(|s| match s {
                StreamSpec::Klv { pid, .. } => Some(*pid),
                _ => None,
            })
            .expect("validate() guarantees at least one stream")
    }

    /// Iterate over video streams. Convenience accessor for the muxer's
    /// internals; callers shouldn't need this directly.
    pub(crate) fn video_streams(&self) -> impl Iterator<Item = (u16, VideoCodec)> + '_ {
        self.streams.iter().filter_map(|s| match s {
            StreamSpec::Video { pid, codec } => Some((*pid, *codec)),
            _ => None,
        })
    }

    /// Iterate over klv streams. Convenience accessor for the muxer's
    /// internals.
    #[allow(clippy::type_complexity)]
    pub(crate) fn klv_streams(&self) -> impl Iterator<Item = (u16, KlvStreamType, bool)> + '_ {
        self.streams.iter().filter_map(|s| match s {
            StreamSpec::Klv {
                pid,
                stream_type,
                carries_pts,
            } => Some((*pid, *stream_type, *carries_pts)),
            _ => None,
        })
    }

    /// First (and currently, only) video stream's PID, if configured.
    pub fn primary_video_pid(&self) -> Option<u16> {
        self.video_streams().next().map(|(pid, _)| pid)
    }

    /// First (and currently, only) KLV stream's PID, if configured.
    pub fn primary_klv_pid(&self) -> Option<u16> {
        self.klv_streams().next().map(|(pid, _, _)| pid)
    }
}

/// How a `descriptors_for_*` call was scoped — preserved verbatim until
/// `build()` resolves it against the final stream set.
#[derive(Debug, Clone)]
enum DescriptorTarget {
    Video(usize),
    Klv(usize),
    Absolute(usize),
}

/// Ergonomic construction of [`Config`] with one chain of method calls.
///
/// Mirrors the C-side builder shape (`srtc_mux_config_*`). Build then
/// call [`ConfigBuilder::build`] to get a validated [`Config`].
#[derive(Debug, Clone, Default)]
pub struct ConfigBuilder {
    streams: Vec<StreamSpec>,
    pcr_pid: Option<u16>,
    pcr_interval_ms: Option<u32>,
    psi_interval_ms: Option<u32>,
    buffer_packets: Option<usize>,
    /// Append-order list of (target, descriptors) pairs. `build()` walks
    /// this and materializes `Config::stream_descriptors`. A later entry
    /// for the same resolved absolute index overwrites earlier entries
    /// (caller's last word wins) — same behavior as overwriting any
    /// builder field.
    pending_descriptors: Vec<(DescriptorTarget, Vec<Vec<u8>>)>,
}

impl ConfigBuilder {
    pub fn add_stream(mut self, spec: StreamSpec) -> Self {
        self.streams.push(spec);
        self
    }

    pub fn add_video(self, pid: u16, codec: VideoCodec) -> Self {
        self.add_stream(StreamSpec::Video { pid, codec })
    }

    pub fn add_klv(self, pid: u16, stream_type: KlvStreamType, carries_pts: bool) -> Self {
        self.add_stream(StreamSpec::Klv {
            pid,
            stream_type,
            carries_pts,
        })
    }

    pub fn pcr_pid(mut self, pid: u16) -> Self {
        self.pcr_pid = Some(pid);
        self
    }

    pub fn pcr_interval_ms(mut self, ms: u32) -> Self {
        self.pcr_interval_ms = Some(ms);
        self
    }

    pub fn psi_interval_ms(mut self, ms: u32) -> Self {
        self.psi_interval_ms = Some(ms);
        self
    }

    pub fn buffer_packets(mut self, n: usize) -> Self {
        self.buffer_packets = Some(n);
        self
    }

    /// Set descriptor list for the n-th video stream (zero-indexed
    /// among `StreamSpec::Video` entries in add-order). Replaces any
    /// previously set list for that stream. `video_index` out of range
    /// surfaces as [`MuxError::InvalidStreamHandle`] from
    /// [`ConfigBuilder::build`] (resolved at build time so chain-order
    /// doesn't matter).
    ///
    /// Each `Vec<u8>` is one complete descriptor TLV — build them with
    /// the helpers in [`crate::mpegts::descriptors`].
    pub fn stream_descriptors_for_video(
        mut self,
        video_index: usize,
        descriptors: Vec<Vec<u8>>,
    ) -> Self {
        self.pending_descriptors
            .push((DescriptorTarget::Video(video_index), descriptors));
        self
    }

    /// Set descriptor list for the n-th KLV stream (zero-indexed among
    /// `StreamSpec::Klv` entries in add-order).
    pub fn stream_descriptors_for_klv(
        mut self,
        klv_index: usize,
        descriptors: Vec<Vec<u8>>,
    ) -> Self {
        self.pending_descriptors
            .push((DescriptorTarget::Klv(klv_index), descriptors));
        self
    }

    /// Set descriptor list by absolute stream index in `streams`
    /// (across both kinds). Lower-level escape hatch for callers that
    /// already track absolute indices.
    pub fn stream_descriptors_for_stream(
        mut self,
        stream_index: usize,
        descriptors: Vec<Vec<u8>>,
    ) -> Self {
        self.pending_descriptors
            .push((DescriptorTarget::Absolute(stream_index), descriptors));
        self
    }

    /// Finalize. Returns a validated [`Config`] or an error describing the
    /// failed rule.
    pub fn build(self) -> Result<Config, MuxError> {
        let n = self.streams.len();
        let mut stream_descriptors: Vec<Vec<Vec<u8>>> = vec![Vec::new(); n];

        // Resolve each pending target against the final stream set.
        // Iteration is in caller-call order so a later set on the same
        // resolved index overwrites an earlier one.
        for (target, descs) in self.pending_descriptors {
            let absolute = match target {
                DescriptorTarget::Video(vi) => self
                    .streams
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| matches!(s, StreamSpec::Video { .. }))
                    .nth(vi)
                    .map(|(abs, _)| abs)
                    .ok_or(MuxError::InvalidStreamHandle {
                        kind: "video",
                        index: vi,
                    })?,
                DescriptorTarget::Klv(ki) => self
                    .streams
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| matches!(s, StreamSpec::Klv { .. }))
                    .nth(ki)
                    .map(|(abs, _)| abs)
                    .ok_or(MuxError::InvalidStreamHandle {
                        kind: "klv",
                        index: ki,
                    })?,
                DescriptorTarget::Absolute(ai) => {
                    if ai >= n {
                        return Err(MuxError::InvalidStreamHandle {
                            kind: "stream",
                            index: ai,
                        });
                    }
                    ai
                }
            };
            stream_descriptors[absolute] = descs;
        }

        let cfg = Config {
            streams: self.streams,
            pcr_pid: self.pcr_pid,
            pcr_interval_ms: self.pcr_interval_ms.unwrap_or(40),
            psi_interval_ms: self.psi_interval_ms.unwrap_or(100),
            buffer_packets: self.buffer_packets.unwrap_or(10_000),
            stream_descriptors,
        };
        cfg.validate()?;
        Ok(cfg)
    }
}

use crate::mpegts::common::{Pcr27mhz, Pts90khz, StreamType};
use std::collections::{BTreeMap, VecDeque};

/// Stats snapshot for [`Muxer`].
///
/// Returned by [`Muxer::stats`]. All counters are cumulative since
/// construction (or the last [`Muxer::reset_stats`] call).
///
/// `per_stream` is keyed by PID. Entries are created eagerly at
/// [`Muxer::new`] for every configured video and KLV stream so callers
/// can always index by a known PID without first checking for key
/// presence.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MuxerStats {
    /// Total 188-byte TS packets drained via [`Muxer::pull`].
    pub ts_packets_emitted: u64,
    /// Total bytes drained via [`Muxer::pull`] (`ts_packets_emitted * 188`).
    pub ts_bytes_emitted: u64,
    /// Per-stream counters, keyed by PID. One entry per configured
    /// video or KLV stream. `StreamStats::items` = push_video_to /
    /// push_klv_to call count; `StreamStats::bytes` = raw ES bytes pushed
    /// (before PES/TS framing overhead).
    pub per_stream: BTreeMap<u16, crate::mpegts::stats::StreamStats>,
}

use self::pes::{
    MAX_PES_HEADER_SIZE, PesPtsField, STREAM_ID_KLV, STREAM_ID_VIDEO, write_pes_header,
};
use self::psi::{KLVA_REGISTRATION_DESCRIPTOR, PmtStreamEntry, write_pat_packet, write_pmt_packet};
use self::ts::{AdaptationField, ContinuityCounters, write_packet};

/// Sender-side MPEG-TS muxer.
///
/// Construct with `Muxer::new(config)`, push encoded frames via `push_video`
/// and `push_klv`, then drain TS packets with `pull`. The muxer is
/// deterministic — output is a function of inputs only, not wall-clock time.
///
/// See the design doc for full semantics:
/// `docs/specs/2026-05-01-srt-core-mpegts-mux-design.md`.
/// Per-video-stream cached state. Built once at `Muxer::new` time.
struct VideoStreamState {
    pid: u16,
    codec: VideoCodec,
}

/// Per-KLV-stream cached state. Each KLV stream tracks its own `last_pts`
/// for the per-stream-clock decision (locked design point 4).
struct KlvStreamState {
    pid: u16,
    stream_type: KlvStreamType,
    carries_pts: bool,
}

pub struct Muxer {
    config: Config,
    /// Per-stream pre-composed PMT descriptor bytes, indexed parallel to
    /// `config.streams`. Each entry is the concatenation of:
    ///   - auto-emitted bytes (KLVA Registration on PrivateData KLV PIDs,
    ///     suppressed when caller supplies their own Registration), then
    ///   - caller-supplied bytes from `config.stream_descriptors`.
    ///
    /// Borrowed at PMT emission time; never reallocated after construction.
    pmt_descriptor_cache: Vec<Vec<u8>>,
    /// One entry per video stream in `config.streams` (filtered to Video),
    /// in the order they appear. Index = `VideoStreamHandle`.
    video_streams: Vec<VideoStreamState>,
    /// One entry per KLV stream. Index = `KlvStreamHandle`.
    klv_streams: Vec<KlvStreamState>,
    pcr_pid: u16,
    pcr_interval_27mhz: u64,
    psi_interval_90khz: i64,

    queue: VecDeque<[u8; 188]>,
    counters: ContinuityCounters,

    /// Last PSI emission PTS, masked to 33 bits (0..2^33). None until first.
    /// PSI cadence is single-timeline (driven by whichever push_*_to call
    /// passed the most recent PTS) — that matches single-stream semantics
    /// exactly when only one stream is configured.
    last_psi_emission_pts: Option<u64>,
    /// 27 MHz PCR value at the most recent PCR emission. None until first.
    /// PCR rides one PID (`self.pcr_pid`) so a single timeline is correct.
    last_pcr_emission_27mhz: Option<u64>,

    // ── Stats counters ────────────────────────────────────────────────────
    ts_packets_emitted: u64,
    ts_bytes_emitted: u64,
    /// Keyed by PID. Populated eagerly at construction for every configured
    /// stream; only the `items` / `bytes` / `discontinuities` fields change
    /// at runtime. `pid` and `stream_type` are set at construction and
    /// never modified.
    per_stream: BTreeMap<u16, crate::mpegts::stats::StreamStats>,
}

impl Muxer {
    /// Construct and validate.
    pub fn new(config: Config) -> Result<Self, MuxError> {
        config.validate()?;

        // Build per-stream state in declaration order. Either vec may be
        // empty (video-only or KLV-only output is valid as of Path 3);
        // Config::validate enforces ≥1 stream of either kind.
        let video_streams: Vec<VideoStreamState> = config
            .video_streams()
            .map(|(pid, codec)| VideoStreamState { pid, codec })
            .collect();
        let klv_streams: Vec<KlvStreamState> = config
            .klv_streams()
            .map(|(pid, stream_type, carries_pts)| KlvStreamState {
                pid,
                stream_type,
                carries_pts,
            })
            .collect();

        // Pre-compose per-stream descriptor bytes once. The auto-emitted
        // KLVA Registration on PrivateData KLV PIDs is suppressed when the
        // caller supplies their own Registration descriptor on the same
        // PID — TSDuck and ffprobe both flag duplicate Registration
        // descriptors, and the corpus shows real senders never duplicate.
        let mut pmt_descriptor_cache: Vec<Vec<u8>> = Vec::with_capacity(config.streams.len());
        for (i, spec) in config.streams.iter().enumerate() {
            let caller_descs = &config.stream_descriptors[i];
            let caller_has_registration = caller_descs
                .iter()
                .any(|tlv| !tlv.is_empty() && tlv[0] == 0x05);

            // If a caller supplied a Registration descriptor on a KLV PID
            // with a non-KLVA format_identifier, log a warning — they're
            // probably tagging a vendor-specific KLV transport, but it
            // means receivers won't recognize the stream as KLV via the
            // standard registration path.
            if matches!(spec, StreamSpec::Klv { .. }) {
                for tlv in caller_descs {
                    if tlv.len() >= 6 && tlv[0] == 0x05 && &tlv[2..6] != b"KLVA" {
                        tracing::warn!(
                            "caller-supplied Registration descriptor on KLV PID has \
                             non-KLVA format_identifier ({:?}); receivers may not \
                             recognize the stream as KLV",
                            std::str::from_utf8(&tlv[2..6]).unwrap_or("?")
                        );
                    }
                }
            }

            let mut bytes = Vec::new();
            if let StreamSpec::Klv {
                stream_type: KlvStreamType::PrivateData,
                ..
            } = spec
            {
                if !caller_has_registration {
                    bytes.extend_from_slice(KLVA_REGISTRATION_DESCRIPTOR);
                }
            }
            for tlv in caller_descs {
                bytes.extend_from_slice(tlv);
            }
            pmt_descriptor_cache.push(bytes);
        }

        let pcr_pid = config.resolved_pcr_pid();
        let pcr_interval_27mhz = (config.pcr_interval_ms as u64) * 27_000;
        let psi_interval_90khz = (config.psi_interval_ms as i64) * 90;

        // Eagerly create per-stream stats entries for every configured stream.
        // Identity fields (pid, stream_type) are set here and never change;
        // flow counters (items, bytes, discontinuities) start at zero.
        let mut per_stream: BTreeMap<u16, crate::mpegts::stats::StreamStats> = BTreeMap::new();
        for v in &video_streams {
            let stream_type_byte = match v.codec {
                VideoCodec::H264 => StreamType::H264.as_u8(),
                VideoCodec::H265 => StreamType::H265.as_u8(),
            };
            per_stream.insert(
                v.pid,
                crate::mpegts::stats::StreamStats {
                    pid: v.pid,
                    stream_type: stream_type_byte,
                    ..Default::default()
                },
            );
        }
        for k in &klv_streams {
            let stream_type_byte = match k.stream_type {
                KlvStreamType::PrivateData => StreamType::KlvPrivate.as_u8(),
                KlvStreamType::SynchronousMetadata => StreamType::KlvSyncMetadata.as_u8(),
            };
            per_stream.insert(
                k.pid,
                crate::mpegts::stats::StreamStats {
                    pid: k.pid,
                    stream_type: stream_type_byte,
                    ..Default::default()
                },
            );
        }

        Ok(Self {
            config,
            pmt_descriptor_cache,
            video_streams,
            klv_streams,
            pcr_pid,
            pcr_interval_27mhz,
            psi_interval_90khz,
            queue: VecDeque::with_capacity(64),
            counters: ContinuityCounters::new(),
            last_psi_emission_pts: None,
            last_pcr_emission_27mhz: None,
            ts_packets_emitted: 0,
            ts_bytes_emitted: 0,
            per_stream,
        })
    }

    /// Push one H.264 / H.265 access unit in Annex-B framing.
    ///
    /// `key_frame=true` causes the first TS packet of the resulting PES to
    /// carry an adaptation field with `random_access_indicator` set.
    ///
    /// Returns `Err(MuxError::InvalidNal)` if `nal` doesn't begin with an
    /// Annex-B start code.
    /// Returns `Err(MuxError::BufferFull)` if the resulting TS packets would
    /// exceed `Config::buffer_packets`. State is unchanged in either error
    /// case.
    pub fn push_video(
        &mut self,
        nal: &[u8],
        pts_90khz: i64,
        key_frame: bool,
    ) -> Result<(), MuxError> {
        // The single-target API only resolves when exactly one video stream
        // is configured. N=0 and N>1 are both ambiguous.
        if self.video_streams.len() != 1 {
            return Err(MuxError::AmbiguousTarget {
                kind: "video",
                count: self.video_streams.len(),
            });
        }
        let handle = VideoStreamHandle::new(0);
        self.push_video_to(handle, nal, pts_90khz, key_frame)
    }

    /// Push one KLV metadata blob.
    ///
    /// `pts_90khz` becomes the PES PTS when the KLV stream was configured with
    /// `carries_pts: true` in [`StreamSpec::Klv`]; ignored otherwise.
    /// Returns `Err(MuxError::BufferFull)` like `push_video`.
    pub fn push_klv(&mut self, klv: &[u8], pts_90khz: i64) -> Result<(), MuxError> {
        if self.klv_streams.len() != 1 {
            return Err(MuxError::AmbiguousTarget {
                kind: "klv",
                count: self.klv_streams.len(),
            });
        }
        let handle = KlvStreamHandle::new(0);
        self.push_klv_to(handle, klv, pts_90khz)
    }

    /// Drain ready TS packets into `out`.
    ///
    /// Returns the number of bytes written: 0 or a positive multiple of 188.
    /// `0` indicates either an empty queue or `out.len() < 188`. Pull is
    /// infallible — there are no failure modes that don't already surface
    /// at `push_video` / `push_klv` time (buffer-full, validation).
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

    /// All `VideoStreamHandle`s for this muxer, in declaration order.
    /// Returns one handle per `StreamSpec::Video` in the original config.
    pub fn video_handles(&self) -> Vec<VideoStreamHandle> {
        (0..self.video_streams.len())
            .map(VideoStreamHandle::new)
            .collect()
    }

    /// All `KlvStreamHandle`s for this muxer, in declaration order.
    pub fn klv_handles(&self) -> Vec<KlvStreamHandle> {
        (0..self.klv_streams.len())
            .map(KlvStreamHandle::new)
            .collect()
    }

    /// Handle for the i-th video stream, or `None` if out of range.
    /// Convenience for callers who add streams in known order.
    pub fn video_stream_handle(&self, index: usize) -> Option<VideoStreamHandle> {
        if index < self.video_streams.len() {
            Some(VideoStreamHandle::new(index))
        } else {
            None
        }
    }

    /// Handle for the i-th KLV stream, or `None` if out of range.
    pub fn klv_stream_handle(&self, index: usize) -> Option<KlvStreamHandle> {
        if index < self.klv_streams.len() {
            Some(KlvStreamHandle::new(index))
        } else {
            None
        }
    }

    /// Push one H.264 / H.265 access unit on a specific video stream.
    ///
    /// `pts_90khz` and `key_frame` carry the same semantics as
    /// [`Self::push_video`]. The caller selects the destination stream
    /// via the [`VideoStreamHandle`] obtained from
    /// [`Self::video_handles`] / [`Self::video_stream_handle`].
    ///
    /// Returns [`MuxError::InvalidStreamHandle`] if the handle's index
    /// is out of range for this muxer's configured video stream count.
    pub fn push_video_to(
        &mut self,
        handle: VideoStreamHandle,
        nal: &[u8],
        pts_90khz: i64,
        key_frame: bool,
    ) -> Result<(), MuxError> {
        validate_annex_b(nal)?;

        let idx = handle.index();
        if idx >= self.video_streams.len() {
            return Err(MuxError::InvalidStreamHandle {
                kind: "video",
                index: idx,
            });
        }
        let video_pid = self.video_streams[idx].pid;

        let mut header = [0u8; MAX_PES_HEADER_SIZE];
        let header_len = write_pes_header(
            &mut header,
            STREAM_ID_VIDEO,
            PesPtsField::PtsOnly(Pts90khz(pts_90khz)),
            None,
        );

        let total = header_len + nal.len();
        let video_packets = ts_packets_for(total);
        let psi_packets = if self.psi_due(pts_90khz) { 2 } else { 0 };

        if self.queue.len() + psi_packets + video_packets > self.config.buffer_packets {
            return Err(MuxError::BufferFull {
                capacity_packets: self.config.buffer_packets,
            });
        }

        self.maybe_emit_psi(pts_90khz);

        let mut pes_buf = Vec::with_capacity(total);
        pes_buf.extend_from_slice(&header[..header_len]);
        pes_buf.extend_from_slice(nal);

        let mut cursor = 0;
        let mut first = true;
        while cursor < pes_buf.len() {
            let mut adaptation = AdaptationField::default();
            if first {
                if key_frame {
                    adaptation.random_access = true;
                }
                if self.pcr_pid == video_pid && self.pcr_due(pts_90khz) {
                    let pcr = Pcr27mhz::from_pts(Pts90khz(pts_90khz));
                    adaptation.pcr = Some(pcr);
                    self.last_pcr_emission_27mhz = Some(pcr.0);
                }
            }
            let mut pkt = [0u8; 188];
            let result = write_packet(
                &mut pkt,
                video_pid,
                first,
                adaptation,
                &pes_buf[cursor..],
                &mut self.counters,
            );
            cursor += result.payload_consumed;
            self.queue.push_back(pkt);
            first = false;
        }

        // Count on the Ok path only — after all early-returns above.
        if let Some(s) = self.per_stream.get_mut(&video_pid) {
            s.items += 1;
            s.bytes += nal.len() as u64;
        }

        Ok(())
    }

    /// Push one KLV metadata blob on a specific KLV stream.
    ///
    /// `pts_90khz` carries the same semantics as [`Self::push_klv`] —
    /// used as the PES PTS only when the targeted KLV stream was
    /// configured with `carries_pts: true`; ignored otherwise.
    ///
    /// Returns [`MuxError::InvalidStreamHandle`] if the handle's index
    /// is out of range.
    pub fn push_klv_to(
        &mut self,
        handle: KlvStreamHandle,
        klv: &[u8],
        pts_90khz: i64,
    ) -> Result<(), MuxError> {
        let idx = handle.index();
        if idx >= self.klv_streams.len() {
            return Err(MuxError::InvalidStreamHandle {
                kind: "klv",
                index: idx,
            });
        }
        let k = &self.klv_streams[idx];
        let klv_pid = k.pid;
        let klv_carries_pts = k.carries_pts;

        let pts_field = if klv_carries_pts {
            PesPtsField::PtsOnly(Pts90khz(pts_90khz))
        } else {
            PesPtsField::None
        };

        let pes_overhead = 3usize + if klv_carries_pts { 5 } else { 0 };
        let max_klv = (u16::MAX as usize) - pes_overhead;
        if klv.len() > max_klv {
            return Err(MuxError::KlvTooLarge {
                size: klv.len(),
                max: max_klv,
            });
        }

        let mut header = [0u8; MAX_PES_HEADER_SIZE];
        let header_len = write_pes_header(
            &mut header,
            STREAM_ID_KLV,
            pts_field,
            Some(klv.len() as u16),
        );

        let total = header_len + klv.len();
        let klv_packets = ts_packets_for(total);
        let psi_packets = if self.psi_due(pts_90khz) { 2 } else { 0 };

        if self.queue.len() + psi_packets + klv_packets > self.config.buffer_packets {
            return Err(MuxError::BufferFull {
                capacity_packets: self.config.buffer_packets,
            });
        }

        self.maybe_emit_psi(pts_90khz);

        let mut pes_buf = Vec::with_capacity(total);
        pes_buf.extend_from_slice(&header[..header_len]);
        pes_buf.extend_from_slice(klv);

        let mut cursor = 0;
        let mut first = true;
        while cursor < pes_buf.len() {
            let mut adaptation = AdaptationField::default();
            if first && self.pcr_pid == klv_pid && self.pcr_due(pts_90khz) {
                let pcr = Pcr27mhz::from_pts(Pts90khz(pts_90khz));
                adaptation.pcr = Some(pcr);
                self.last_pcr_emission_27mhz = Some(pcr.0);
            }
            let mut pkt = [0u8; 188];
            let result = write_packet(
                &mut pkt,
                klv_pid,
                first,
                adaptation,
                &pes_buf[cursor..],
                &mut self.counters,
            );
            cursor += result.payload_consumed;
            self.queue.push_back(pkt);
            first = false;
        }

        // Count on the Ok path only — after all early-returns above.
        if let Some(s) = self.per_stream.get_mut(&klv_pid) {
            s.items += 1;
            s.bytes += klv.len() as u64;
        }

        Ok(())
    }

    /// Return a snapshot of the current stats counters.
    ///
    /// All per-stream entries are present regardless of whether any data has
    /// been pushed to that stream yet.
    pub fn stats(&self) -> MuxerStats {
        MuxerStats {
            ts_packets_emitted: self.ts_packets_emitted,
            ts_bytes_emitted: self.ts_bytes_emitted,
            per_stream: self.per_stream.clone(),
        }
    }

    /// Zero all flow counters.
    ///
    /// Per-stream entries are preserved (their `pid` and `stream_type`
    /// identity fields remain set); only the flow counters (`items`,
    /// `bytes`, `discontinuities`) are zeroed.
    pub fn reset_stats(&mut self) {
        self.ts_packets_emitted = 0;
        self.ts_bytes_emitted = 0;
        for s in self.per_stream.values_mut() {
            s.items = 0;
            s.bytes = 0;
            s.discontinuities = 0;
        }
    }

    fn psi_due(&self, pts_90khz: i64) -> bool {
        match self.last_psi_emission_pts {
            None => true,
            Some(last_masked) => {
                let now_masked = Pts90khz(pts_90khz).masked_33bit();
                let delta = crate::mpegts::common::pts_diff_33bit(now_masked, last_masked);
                delta >= self.psi_interval_90khz
            }
        }
    }

    fn pcr_due(&self, pts_90khz: i64) -> bool {
        match self.last_pcr_emission_27mhz {
            None => true,
            Some(last) => {
                // PCR is at 27 MHz; the 33-bit base wraps at 2^33 base ticks.
                // Convert both to 33-bit base and use the same modular helper,
                // then compare in 90 kHz units.
                let now_base_masked = Pts90khz(pts_90khz).masked_33bit();
                let last_base_masked = (last / 300) & ((1u64 << 33) - 1);
                let delta_90khz =
                    crate::mpegts::common::pts_diff_33bit(now_base_masked, last_base_masked);
                let threshold_90khz = (self.pcr_interval_27mhz / 300) as i64;
                delta_90khz >= threshold_90khz
            }
        }
    }

    fn maybe_emit_psi(&mut self, pts_90khz: i64) {
        if !self.psi_due(pts_90khz) {
            return;
        }
        let mut pat = [0u8; 188];
        write_pat_packet(&mut pat, &mut self.counters);
        self.queue.push_back(pat);

        // Walk config.streams in order so descriptor-cache indices align.
        // Single-stream output preserves the existing video-then-klv
        // ordering — no test bytes change for that case.
        let mut entries: Vec<PmtStreamEntry> = Vec::with_capacity(self.config.streams.len());
        for (i, spec) in self.config.streams.iter().enumerate() {
            let stream_type = match spec {
                StreamSpec::Video {
                    codec: VideoCodec::H264,
                    ..
                } => StreamType::H264,
                StreamSpec::Video {
                    codec: VideoCodec::H265,
                    ..
                } => StreamType::H265,
                StreamSpec::Klv {
                    stream_type: KlvStreamType::PrivateData,
                    ..
                } => StreamType::KlvPrivate,
                StreamSpec::Klv {
                    stream_type: KlvStreamType::SynchronousMetadata,
                    ..
                } => StreamType::KlvSyncMetadata,
            };
            entries.push(PmtStreamEntry {
                stream_type,
                elementary_pid: spec.pid(),
                descriptors: &self.pmt_descriptor_cache[i],
            });
        }

        let mut pmt = [0u8; 188];
        write_pmt_packet(&mut pmt, self.pcr_pid, &entries, &mut self.counters)
            .expect("validated Config must produce single-section PMT");
        self.queue.push_back(pmt);

        self.last_psi_emission_pts = Some(Pts90khz(pts_90khz).masked_33bit());
    }
}

fn validate_annex_b(nal: &[u8]) -> Result<(), MuxError> {
    if nal.starts_with(&[0x00, 0x00, 0x00, 0x01]) || nal.starts_with(&[0x00, 0x00, 0x01]) {
        Ok(())
    } else {
        Err(MuxError::InvalidNal)
    }
}

/// Number of 188-byte TS packets needed to carry `payload_size` bytes of
/// PES (header + ES). 184 = 188 - 4 byte TS header. Adaptation field eats
/// further capacity but for sizing purposes the worst case is no AF (gives
/// the smallest packet count). The orchestrator may emit one more packet
/// than this if AF stuffing pushes a byte over; we allow a 1-packet slop
/// in the buffer reservation.
fn ts_packets_for(payload_size: usize) -> usize {
    payload_size.div_ceil(184).max(1) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_validates() {
        Config::default().validate().expect("default is valid");
    }

    #[test]
    fn rejects_video_pid_zero() {
        let mut cfg = Config::default();
        if let Some(StreamSpec::Video { pid, .. }) = cfg
            .streams
            .iter_mut()
            .find(|s| matches!(s, StreamSpec::Video { .. }))
        {
            *pid = 0x0000;
        }
        assert!(matches!(
            cfg.validate(),
            Err(MuxError::InvalidConfig(
                "video pid must be in 0x0010..=0x1FFE"
            ))
        ));
    }

    #[test]
    fn rejects_klv_pid_null() {
        let mut cfg = Config::default();
        if let Some(StreamSpec::Klv { pid, .. }) = cfg
            .streams
            .iter_mut()
            .find(|s| matches!(s, StreamSpec::Klv { .. }))
        {
            *pid = 0x1FFF;
        }
        assert!(matches!(
            cfg.validate(),
            Err(MuxError::InvalidConfig(
                "klv pid must be in 0x0010..=0x1FFE"
            ))
        ));
    }

    #[test]
    fn rejects_pid_collision() {
        let cfg = Config::builder()
            .add_video(0x1234, VideoCodec::H264)
            .add_klv(0x1234, KlvStreamType::PrivateData, false)
            .build();
        assert!(matches!(
            cfg,
            Err(MuxError::InvalidConfig("stream PIDs must all be distinct"))
        ));
    }

    #[test]
    fn rejects_unrelated_pcr_pid() {
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .pcr_pid(0x0500)
            .build();
        assert!(matches!(
            cfg,
            Err(MuxError::InvalidConfig(
                "pcr_pid must equal a configured stream PID"
            ))
        ));
    }

    #[test]
    fn accepts_pcr_pid_on_klv() {
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .pcr_pid(0x1031)
            .build();
        cfg.expect("pcr_pid on klv is allowed");
    }

    #[test]
    fn rejects_pcr_interval_zero() {
        let cfg = Config {
            pcr_interval_ms: 0,
            ..Config::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_pcr_interval_over_100() {
        let cfg = Config {
            pcr_interval_ms: 150,
            ..Config::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(MuxError::InvalidConfig(
                "pcr_interval_ms must be in 1..=100"
            ))
        ));
    }

    #[test]
    fn rejects_psi_interval_too_small() {
        let cfg = Config {
            psi_interval_ms: 5,
            ..Config::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(MuxError::InvalidConfig("psi_interval_ms must be >= 10"))
        ));
    }

    #[test]
    fn rejects_buffer_too_small() {
        let cfg = Config {
            buffer_packets: 5,
            ..Config::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(MuxError::InvalidConfig("buffer_packets must be >= 10"))
        ));
    }

    #[test]
    fn rejects_sync_without_pts() {
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::SynchronousMetadata, false)
            .build();
        assert!(cfg.is_err());
    }

    #[test]
    fn accepts_async_with_pts_combo() {
        // 0x06 + PTS — the common-practice "sync KLV everyone recognizes"
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, true)
            .build();
        cfg.expect("0x06 + PTS is valid");
    }

    #[test]
    fn resolved_pcr_pid_default() {
        let cfg = Config::default();
        assert_eq!(cfg.resolved_pcr_pid(), cfg.primary_video_pid().unwrap());
    }

    #[test]
    fn resolved_pcr_pid_explicit() {
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .pcr_pid(0x1031)
            .build()
            .unwrap();
        assert_eq!(cfg.resolved_pcr_pid(), 0x1031);
    }

    #[test]
    fn muxer_constructs_with_valid_config() {
        let mux = Muxer::new(Config::default());
        assert!(mux.is_ok());
    }

    #[test]
    fn muxer_rejects_invalid_config() {
        let mut cfg = Config::default();
        if let Some(StreamSpec::Video { pid, .. }) = cfg
            .streams
            .iter_mut()
            .find(|s| matches!(s, StreamSpec::Video { .. }))
        {
            *pid = 0;
        }
        let res = Muxer::new(cfg);
        assert!(res.is_err());
    }

    #[test]
    fn pull_returns_zero_on_empty_queue() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        let mut buf = [0u8; 1316];
        assert_eq!(mux.pull(&mut buf), 0);
    }

    #[test]
    fn pull_returns_zero_on_short_buffer() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        let nal = [0x00, 0x00, 0x00, 0x01, 0x09, 0x10];
        mux.push_video(&nal, 0, true).unwrap();
        let mut buf = [0u8; 100];
        assert_eq!(mux.pull(&mut buf), 0);
    }

    #[test]
    fn push_video_rejects_non_annex_b() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        let bad = [0x12, 0x34, 0x56];
        assert!(matches!(
            mux.push_video(&bad, 0, false),
            Err(MuxError::InvalidNal)
        ));
    }

    #[test]
    fn push_video_accepts_3byte_start_code() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        let nal = [0x00, 0x00, 0x01, 0x09, 0x10];
        assert!(mux.push_video(&nal, 0, true).is_ok());
    }

    #[test]
    fn first_pull_includes_pat_pmt() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        let nal = [0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x99];
        mux.push_video(&nal, 0, true).unwrap();
        let mut buf = [0u8; 4096];
        let n = mux.pull(&mut buf);
        assert!(n >= 188 * 3, "expected at least PAT + PMT + 1 video packet");
        // First packet should be PAT (PID 0)
        let pid = (((buf[1] as u16) & 0x1F) << 8) | buf[2] as u16;
        assert_eq!(pid, 0x0000);
        // Second packet should be PMT (PID 0x1000 from psi.rs)
        let pid_2 = (((buf[188 + 1] as u16) & 0x1F) << 8) | buf[188 + 2] as u16;
        assert_eq!(pid_2, 0x1000);
    }

    #[test]
    fn buffer_full_returned_when_overcommitted() {
        let cfg = Config {
            buffer_packets: 10,
            ..Config::default()
        };
        let mut mux = Muxer::new(cfg).unwrap();
        // A 50KB IDR is much larger than 10 packets can hold.
        let big_nal = {
            let mut v = vec![0u8; 50_000];
            v[0] = 0;
            v[1] = 0;
            v[2] = 0;
            v[3] = 1;
            v[4] = 0x65; // IDR slice NAL type
            v
        };
        let res = mux.push_video(&big_nal, 0, true);
        assert!(matches!(
            res,
            Err(MuxError::BufferFull {
                capacity_packets: 10
            })
        ));
    }

    #[test]
    fn buffer_full_does_not_modify_state() {
        let cfg = Config {
            buffer_packets: 10,
            ..Config::default()
        };
        let mut mux = Muxer::new(cfg).unwrap();
        let nal = vec![0u8; 50_000];
        let nal = {
            let mut v = nal;
            v[..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            v
        };
        let _ = mux.push_video(&nal, 0, true);
        // Queue should be empty (push didn't commit).
        let mut buf = [0u8; 1316];
        assert_eq!(mux.pull(&mut buf), 0);
    }

    #[test]
    fn psi_emission_survives_pts_rollover() {
        // Push a video AU just before 33-bit rollover, then another well past.
        // True modular delta is +9590 ticks (~106ms), greater than psi_interval
        // default of 9000 ticks (100ms), so PSI MUST re-emit. Buggy raw i64
        // subtraction yields a huge negative and wrongly suppresses PSI.
        let mut mux = Muxer::new(Config::default()).unwrap();
        let nal = vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x00];
        let just_before_wrap = (1i64 << 33) - 90;
        let well_past_wrap = 9_500;
        mux.push_video(&nal, just_before_wrap, true).unwrap();
        let mut buf = vec![0u8; 188 * 64];
        while mux.pull(&mut buf) > 0 {}
        mux.push_video(&nal, well_past_wrap, false).unwrap();
        let n = mux.pull(&mut buf);
        assert!(n > 0);
        // First packet should be PAT (PID 0x0000) since PSI is due.
        let first_pid = (((buf[1] as u16) & 0x1F) << 8) | buf[2] as u16;
        assert_eq!(
            first_pid, 0x0000,
            "PSI suppressed across rollover; got first PID 0x{:04X}",
            first_pid
        );
    }

    #[test]
    fn psi_not_due_on_backward_pts() {
        // B-frame display-order: PTS may zigzag backward by a few frames. PSI
        // cadence must NOT trigger on a backward step (it would wrongly emit).
        let mut mux = Muxer::new(Config::default()).unwrap();
        let nal = vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x00];
        mux.push_video(&nal, 100_000, true).unwrap();
        let mut buf = vec![0u8; 188 * 64];
        while mux.pull(&mut buf) > 0 {}
        // Now push a backward PTS (display order earlier). Should NOT emit PSI.
        mux.push_video(&nal, 100_000 - 270, false).unwrap(); // -3ms
        let n = mux.pull(&mut buf);
        assert!(n > 0);
        let first_pid = (((buf[1] as u16) & 0x1F) << 8) | buf[2] as u16;
        assert_eq!(
            first_pid, 0x1011,
            "PSI emitted on backward PTS, got first PID 0x{:04X}",
            first_pid
        );
    }

    #[test]
    fn psi_due_after_threshold_forward() {
        // Sanity: forward by exactly psi_interval triggers PSI.
        let mut mux = Muxer::new(Config::default()).unwrap();
        let nal = vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x00];
        mux.push_video(&nal, 0, true).unwrap();
        let mut buf = vec![0u8; 188 * 64];
        while mux.pull(&mut buf) > 0 {}
        // psi_interval default = 100ms = 9000 ticks at 90kHz.
        mux.push_video(&nal, 9_000, false).unwrap();
        let n = mux.pull(&mut buf);
        assert!(n > 0);
        // First packet should be PAT (PID 0x0000) since PSI was due.
        let first_pid = (((buf[1] as u16) & 0x1F) << 8) | buf[2] as u16;
        assert_eq!(first_pid, 0x0000, "expected PAT, got 0x{:04X}", first_pid);
    }

    #[test]
    fn push_klv_rejects_oversized_blob() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        // PES_packet_length is u16; with PTS off, max KLV payload = 65535 - 3 = 65532.
        let too_big = vec![0u8; 65_533];
        let err = mux.push_klv(&too_big, 0).unwrap_err();
        match err {
            MuxError::KlvTooLarge { size, max } => {
                assert_eq!(size, 65_533);
                assert_eq!(max, 65_532);
            }
            other => panic!("expected MuxError::KlvTooLarge, got {:?}", other),
        }
    }

    #[test]
    fn push_klv_accepts_largest_legal_blob() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        // 65532 with no PTS is the spec-imposed ceiling.
        let max_klv = vec![0xAB; 65_532];
        mux.push_klv(&max_klv, 0)
            .expect("max-size KLV must succeed");
    }

    #[test]
    fn push_klv_with_pts_reduces_max() {
        // With klv_carries_pts=true, header_data_length=5, so max payload =
        // 65535 - 3 - 5 = 65527.
        let mut mux = Muxer::new(
            Config::builder()
                .add_video(0x1011, VideoCodec::H264)
                .add_klv(0x1031, KlvStreamType::PrivateData, true)
                .build()
                .unwrap(),
        )
        .unwrap();
        let too_big = vec![0u8; 65_528];
        let err = mux.push_klv(&too_big, 90_000).unwrap_err();
        match err {
            MuxError::KlvTooLarge { size, max } => {
                assert_eq!(size, 65_528);
                assert_eq!(max, 65_527);
            }
            other => panic!("expected MuxError::KlvTooLarge, got {:?}", other),
        }
    }

    #[test]
    fn config_rejects_empty_streams() {
        let cfg = Config {
            streams: vec![],
            ..Config::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, MuxError::InvalidConfig(msg) if msg.contains("at least one stream")));
    }

    #[test]
    fn config_rejects_duplicate_pids() {
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1011, KlvStreamType::PrivateData, false)
            .build();
        let err = cfg.unwrap_err();
        assert!(matches!(err, MuxError::InvalidConfig(msg) if msg.contains("distinct")));
    }

    #[test]
    fn config_pcr_pid_must_match_stream() {
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .pcr_pid(0x1099) // not configured
            .build();
        let err = cfg.unwrap_err();
        assert!(matches!(err, MuxError::InvalidConfig(msg) if msg.contains("pcr_pid")));
    }

    #[test]
    fn handle_types_are_copy_eq_hash() {
        // Compile-time assertion: handles must be Copy + Eq + Hash so
        // consumers can stash them in HashMaps / HashSets and pass them
        // around freely.
        fn assert_copy<T: Copy>() {}
        fn assert_eq_hash<T: Eq + std::hash::Hash>() {}
        assert_copy::<VideoStreamHandle>();
        assert_copy::<KlvStreamHandle>();
        assert_eq_hash::<VideoStreamHandle>();
        assert_eq_hash::<KlvStreamHandle>();
    }

    #[test]
    fn handle_debug_includes_kind_and_index() {
        let v = VideoStreamHandle::for_test(2);
        let k = KlvStreamHandle::for_test(0);
        // Don't lock the exact format, just sanity-check it carries both bits.
        assert!(format!("{v:?}").contains("Video"));
        assert!(format!("{v:?}").contains('2'));
        assert!(format!("{k:?}").contains("Klv"));
        assert!(format!("{k:?}").contains('0'));
    }

    #[test]
    fn handles_single_stream_returns_one_each() {
        let cfg = Config::default();
        let mux = Muxer::new(cfg).unwrap();
        let vs = mux.video_handles();
        let ks = mux.klv_handles();
        assert_eq!(vs.len(), 1);
        assert_eq!(ks.len(), 1);
        assert_eq!(mux.video_stream_handle(0), Some(vs[0]));
        assert_eq!(mux.klv_stream_handle(0), Some(ks[0]));
    }

    #[test]
    fn handles_out_of_range_returns_none() {
        let mux = Muxer::new(Config::default()).unwrap();
        assert_eq!(mux.video_stream_handle(1), None);
        assert_eq!(mux.klv_stream_handle(1), None);
    }

    #[test]
    fn push_video_to_routes_to_correct_pid() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        let h = mux.video_stream_handle(0).unwrap();
        let nal = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42];
        mux.push_video_to(h, &nal, 0, true).unwrap();
        // Drain and inspect: at least one packet should carry video_pid (0x1011).
        let mut buf = vec![0u8; 188 * 16];
        let n = mux.pull(&mut buf);
        assert!(n > 0);
        let mut found = false;
        for chunk in buf[..n].chunks_exact(188) {
            // PID is bits 4..16 of bytes 1..3.
            let pid = ((chunk[1] as u16 & 0x1F) << 8) | chunk[2] as u16;
            if pid == 0x1011 {
                found = true;
                break;
            }
        }
        assert!(found, "expected at least one packet on video PID 0x1011");
    }

    #[test]
    fn push_klv_to_routes_to_correct_pid() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        let h = mux.klv_stream_handle(0).unwrap();
        // Minimal KLV blob — UL + length=0 (16 bytes UL + 1 byte length).
        let mut klv = vec![
            0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00,
            0x00, 0x00,
        ];
        klv.push(0x00);
        mux.push_klv_to(h, &klv, 0).unwrap();
        let mut buf = vec![0u8; 188 * 16];
        let n = mux.pull(&mut buf);
        assert!(n > 0);
        let mut found = false;
        for chunk in buf[..n].chunks_exact(188) {
            let pid = ((chunk[1] as u16 & 0x1F) << 8) | chunk[2] as u16;
            if pid == 0x1031 {
                found = true;
                break;
            }
        }
        assert!(found, "expected at least one packet on KLV PID 0x1031");
    }

    #[test]
    fn push_video_to_invalid_handle_rejects() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        let bogus = VideoStreamHandle::for_test(99);
        let nal = [0x00, 0x00, 0x00, 0x01, 0x67];
        let err = mux.push_video_to(bogus, &nal, 0, true).unwrap_err();
        match err {
            MuxError::InvalidStreamHandle { kind, index } => {
                assert_eq!(kind, "video");
                assert_eq!(index, 99);
            }
            other => panic!("expected InvalidStreamHandle, got {other:?}"),
        }
    }

    #[test]
    fn push_klv_to_invalid_handle_rejects() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        let bogus = KlvStreamHandle::for_test(99);
        let err = mux.push_klv_to(bogus, &[0; 16], 0).unwrap_err();
        match err {
            MuxError::InvalidStreamHandle { kind, index } => {
                assert_eq!(kind, "klv");
                assert_eq!(index, 99);
            }
            other => panic!("expected InvalidStreamHandle, got {other:?}"),
        }
    }

    #[test]
    fn config_validate_accepts_dual_video_plus_klv() {
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264) // EO
            .add_video(0x1021, VideoCodec::H264) // IR
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .build();
        assert!(cfg.is_ok(), "dual-video + KLV must validate");
    }

    #[test]
    fn config_validate_accepts_dual_klv_plus_video() {
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .add_klv(0x1041, KlvStreamType::PrivateData, true)
            .build();
        assert!(cfg.is_ok(), "video + dual-KLV must validate");
    }

    #[test]
    fn config_validate_rejects_seventeen_video_streams() {
        let mut b = Config::builder();
        for i in 0..17u16 {
            b = b.add_video(0x1010 + i, VideoCodec::H264);
        }
        b = b.add_klv(0x1100, KlvStreamType::PrivateData, false);
        let err = b.build().unwrap_err();
        assert!(
            matches!(err, MuxError::TooManyVideoStreams { count: 17, cap: 16 }),
            "expected TooManyVideoStreams {{ 17, 16 }}, got {err:?}",
        );
    }

    #[test]
    fn config_validate_rejects_seventeen_klv_streams() {
        let mut b = Config::builder().add_video(0x1011, VideoCodec::H264);
        for i in 0..17u16 {
            b = b.add_klv(0x1100 + i, KlvStreamType::PrivateData, false);
        }
        let err = b.build().unwrap_err();
        assert!(
            matches!(err, MuxError::TooManyKlvStreams { count: 17, cap: 16 }),
            "expected TooManyKlvStreams {{ 17, 16 }}, got {err:?}",
        );
    }

    #[test]
    fn muxer_new_accepts_video_only() {
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .build()
            .unwrap();
        let mux = Muxer::new(cfg);
        assert!(mux.is_ok(), "video-only muxer must construct");
    }

    #[test]
    fn muxer_new_accepts_klv_only() {
        // KLV-only requires PCR pinned to KLV PID (it carries PCR).
        let cfg = Config::builder()
            .add_klv(0x1031, KlvStreamType::PrivateData, true)
            .pcr_pid(0x1031)
            .build()
            .unwrap();
        let mux = Muxer::new(cfg);
        assert!(mux.is_ok(), "klv-only muxer must construct");
    }

    #[test]
    fn push_video_rejects_when_multiple_video_streams_configured() {
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .add_video(0x1021, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();
        let nal = [0x00, 0x00, 0x00, 0x01, 0x67];
        let err = mux.push_video(&nal, 0, true).unwrap_err();
        assert!(
            matches!(
                err,
                MuxError::AmbiguousTarget {
                    kind: "video",
                    count: 2
                }
            ),
            "expected AmbiguousTarget {{ video, 2 }}, got {err:?}",
        );
    }

    #[test]
    fn push_klv_rejects_when_multiple_klv_streams_configured() {
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .add_klv(0x1041, KlvStreamType::PrivateData, true)
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();
        let err = mux.push_klv(&[0; 16], 0).unwrap_err();
        assert!(
            matches!(
                err,
                MuxError::AmbiguousTarget {
                    kind: "klv",
                    count: 2
                }
            ),
            "expected AmbiguousTarget {{ klv, 2 }}, got {err:?}",
        );
    }

    #[test]
    fn push_video_rejects_when_no_video_streams_configured() {
        // KLV-only muxer — push_video has no possible target.
        let cfg = Config::builder()
            .add_klv(0x1031, KlvStreamType::PrivateData, true)
            .pcr_pid(0x1031)
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();
        let nal = [0x00, 0x00, 0x00, 0x01, 0x67];
        let err = mux.push_video(&nal, 0, true).unwrap_err();
        assert!(
            matches!(
                err,
                MuxError::AmbiguousTarget {
                    kind: "video",
                    count: 0
                }
            ),
            "expected AmbiguousTarget {{ video, 0 }}, got {err:?}",
        );
    }

    #[test]
    fn push_klv_rejects_when_no_klv_streams_configured() {
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();
        let err = mux.push_klv(&[0; 16], 0).unwrap_err();
        assert!(
            matches!(
                err,
                MuxError::AmbiguousTarget {
                    kind: "klv",
                    count: 0
                }
            ),
            "expected AmbiguousTarget {{ klv, 0 }}, got {err:?}",
        );
    }

    #[test]
    fn default_config_has_empty_per_stream_descriptors() {
        let cfg = Config::default();
        assert_eq!(cfg.stream_descriptors.len(), cfg.streams.len());
        for descs in &cfg.stream_descriptors {
            assert!(descs.is_empty());
        }
    }

    #[test]
    fn validate_rejects_descriptor_count_mismatch() {
        let cfg = Config {
            stream_descriptors: vec![Vec::new()], // streams has 2, descs has 1
            ..Config::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, MuxError::InvalidConfig(_)));
    }

    #[test]
    fn validate_rejects_malformed_descriptor() {
        // Length byte claims 5 bytes of body but only 1 follows.
        let bad = vec![0xFF, 0x05, 0x00];
        let cfg = Config {
            stream_descriptors: vec![vec![bad], Vec::new()],
            ..Config::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(
            err,
            MuxError::MalformedDescriptor {
                stream_index: 0,
                descriptor_index: 0,
                ..
            }
        ));
    }

    #[test]
    fn validate_rejects_oversized_pmt() {
        // 4 streams × 100-byte descriptor = ~400 bytes > 166 max.
        let big = crate::mpegts::descriptors::user_private(&[0u8; 100]);
        let cfg = Config::builder()
            .add_video(0x100, VideoCodec::H264)
            .add_video(0x101, VideoCodec::H264)
            .add_klv(0x102, KlvStreamType::PrivateData, false)
            .add_klv(0x103, KlvStreamType::PrivateData, false)
            .stream_descriptors_for_stream(0, vec![big.clone()])
            .stream_descriptors_for_stream(1, vec![big.clone()])
            .stream_descriptors_for_stream(2, vec![big.clone()])
            .stream_descriptors_for_stream(3, vec![big])
            .build();
        assert!(matches!(cfg, Err(MuxError::PmtTooLarge { .. })));
    }

    #[test]
    fn builder_routes_video_descriptors_by_video_index() {
        // 2 video, 1 KLV. Setting video_index=1 should land on absolute index 2.
        let cfg = Config::builder()
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x102, KlvStreamType::PrivateData, false)
            .add_video(0x101, VideoCodec::H264)
            .stream_descriptors_for_video(1, vec![crate::mpegts::descriptors::user_private(b"V2")])
            .build()
            .unwrap();

        assert_eq!(cfg.stream_descriptors[0], Vec::<Vec<u8>>::new());
        assert_eq!(cfg.stream_descriptors[1], Vec::<Vec<u8>>::new());
        assert_eq!(cfg.stream_descriptors[2].len(), 1);
        assert_eq!(cfg.stream_descriptors[2][0][0], 0xFF);
    }

    #[test]
    fn builder_rejects_out_of_range_video_index() {
        let res = Config::builder()
            .add_video(0x100, VideoCodec::H264)
            .stream_descriptors_for_video(7, vec![crate::mpegts::descriptors::user_private(b"X")])
            .build();
        assert!(matches!(res, Err(MuxError::InvalidStreamHandle { .. })));
    }

    #[test]
    fn cache_composes_auto_emit_then_caller_bytes_on_klv_private() {
        let cfg = Config::builder()
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .stream_descriptors_for_klv(
                0,
                vec![crate::mpegts::descriptors::user_private(b"KLV_LBL")],
            )
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();

        // Stream 0 (video) — no auto-emit, no caller — empty cache entry.
        assert!(muxer.pmt_descriptor_cache[0].is_empty());

        // Stream 1 (KLV PrivateData) — KLVA Registration (6 bytes) +
        // user_private("KLV_LBL") (9 bytes) = 15 bytes.
        let entry = &muxer.pmt_descriptor_cache[1];
        assert_eq!(entry.len(), 15);
        assert_eq!(&entry[..6], &[0x05, 0x04, b'K', b'L', b'V', b'A']);
        assert_eq!(entry[6], 0xFF); // user_private tag
        assert_eq!(entry[7], 7); // body length
        assert_eq!(&entry[8..], b"KLV_LBL");
    }

    #[test]
    fn cache_suppresses_klva_auto_emit_when_caller_supplies_registration() {
        let cfg = Config::builder()
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .stream_descriptors_for_klv(
                0,
                vec![crate::mpegts::descriptors::registration(*b"KLVA", &[])],
            )
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();

        // Caller's Registration only — auto-emit suppressed. Total = 6 bytes.
        assert_eq!(muxer.pmt_descriptor_cache[0].len(), 6);
    }

    #[test]
    fn cache_no_auto_emit_on_sync_klv() {
        let cfg = Config::builder()
            .add_klv(0x101, KlvStreamType::SynchronousMetadata, true)
            .stream_descriptors_for_klv(
                0,
                vec![
                    crate::mpegts::descriptors::metadata_klva(0x00),
                    crate::mpegts::descriptors::metadata_std(0, 0, 0),
                ],
            )
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();
        // No KLVA auto-emit on SynchronousMetadata. 11 + 11 = 22 bytes.
        assert_eq!(muxer.pmt_descriptor_cache[0].len(), 22);
        assert_eq!(muxer.pmt_descriptor_cache[0][0], 0x26);
        assert_eq!(muxer.pmt_descriptor_cache[0][11], 0x27);
    }
}

#[cfg(test)]
mod stats_tests {
    use super::*;

    #[test]
    fn stats_starts_with_per_stream_entries_for_configured_streams() {
        let cfg = Config::builder()
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .build()
            .unwrap();
        let m = Muxer::new(cfg).unwrap();
        let st = m.stats();
        assert_eq!(st.ts_packets_emitted, 0);
        assert_eq!(st.ts_bytes_emitted, 0);
        assert_eq!(st.per_stream.len(), 2);
        assert!(st.per_stream.contains_key(&0x100));
        assert!(st.per_stream.contains_key(&0x101));
        assert_eq!(st.per_stream[&0x100].stream_type, 0x1B);
        assert_eq!(st.per_stream[&0x101].stream_type, 0x06);
        assert_eq!(st.per_stream[&0x100].items, 0);
    }

    #[test]
    fn stats_count_pushed_items_and_pulled_packets() {
        let cfg = Config::builder()
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .build()
            .unwrap();
        let mut m = Muxer::new(cfg).unwrap();
        let nal: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67, 0xBB, 0xCC];
        m.push_video(nal, 0, true).unwrap();
        let klv: &[u8] = &[
            0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00,
            0x00, 0x00, 0x00,
        ];
        m.push_klv(klv, 0).unwrap();
        let mut buf = vec![0u8; 64 * 188];
        let n = m.pull(&mut buf);
        let st = m.stats();
        assert_eq!(st.per_stream[&0x100].items, 1);
        assert_eq!(st.per_stream[&0x100].bytes, nal.len() as u64);
        assert_eq!(st.per_stream[&0x101].items, 1);
        assert_eq!(st.per_stream[&0x101].bytes, klv.len() as u64);
        assert_eq!(st.ts_bytes_emitted, n as u64);
        assert_eq!(st.ts_packets_emitted, (n / 188) as u64);
    }

    #[test]
    fn reset_stats_zeros_counters_keeps_entries() {
        let cfg = Config::builder()
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .build()
            .unwrap();
        let mut m = Muxer::new(cfg).unwrap();
        let nal: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67, 0xBB];
        m.push_video(nal, 0, true).unwrap();
        m.reset_stats();
        let st = m.stats();
        assert_eq!(st.ts_packets_emitted, 0);
        assert_eq!(st.per_stream.len(), 2);
        assert_eq!(st.per_stream[&0x100].items, 0);
        assert_eq!(st.per_stream[&0x100].bytes, 0);
    }
}
