//! MuxSender-side MPEG-TS muxer.
//!
//! The public surface is `Muxer`, `Config`, `VideoCodec`,
//! `KlvStreamType`. Internal helpers live in `ts`, `psi`, `pes` submodules.
//!
//! Re-export note: `Muxer`, `VideoCodec`, and `KlvStreamType` are re-exported
//! at the crate root (`tst_core::Muxer` etc.). `Config` deliberately is not —
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
/// 0x24 for H.265 / HEVC, 0x33 for H.266 / VVC. AV1 sits on
/// `stream_type 0x06` with an auto-emitted AV01 `registration_descriptor`.
/// Mid-stream codec change is out of scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    H265,
    /// H.266 / VVC. Drives PMT stream_type 0x33.
    H266,
    /// AV1. Drives PMT stream_type 0x06 with auto-emitted AV01
    /// `registration_descriptor`.
    Av1,
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

/// Audio codec carried by an audio elementary stream.
///
/// Drives the PMT `stream_type` byte:
/// - `Mp2` → 0x03 (ISO/IEC 11172-3 Audio — covers MPEG-1 Layer I/II/III)
/// - `Aac` → 0x0F (ISO/IEC 13818-7 ADTS Audio)
/// - `AacLatm` → 0x11 (ISO/IEC 14496-3 LATM Audio)
/// - `Ac3` → 0x81 (ATSC AC-3)
///
/// E-AC-3, DVB-shaped AC-3 (`stream_type 0x06` + AC-3 registration),
/// MP3 on user-private stream_types: not classified automatically;
/// callers route via `DemuxerOptions::treat_as` on the demux side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    Mp2,
    Aac,
    AacLatm,
    Ac3,
}

/// Subtitle / caption codec carried by a subtitle elementary stream.
///
/// All four variants emit PMT `stream_type = 0x06` (PES private data);
/// disambiguation happens via the auto-emitted PMT descriptor at PSI
/// generation time. See `mpegts::descriptors` for the descriptor
/// encoders this enum drives.
///
/// `Clone` but not `Copy` — a deliberate asymmetry vs. `VideoCodec` /
/// `AudioCodec` (which are both `Copy`). Subtitle codec parameters are
/// structurally part of the codec value here (vs. siblings, where the
/// enum is purely a tag), so forward-compatible variants that may carry
/// non-`Copy` payloads (e.g. variable-length DVB ancillary blobs) won't
/// require a breaking change to drop `Copy` later.
///
/// CEA-608/708 in SEI (the typical "captions in H.264/H.265") is NOT
/// in scope for this enum — that's the deferred SEI parsing plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubtitleCodec {
    /// DVB subtitling (bitmap-shaped). Per ETSI EN 300 468 §6.2.41 +
    /// ETSI EN 300 743.
    DvbSubtitling {
        /// ISO 639-2 language code, lowercase ASCII (e.g. *b"eng").
        language: [u8; 3],
        /// ETSI EN 300 468 Table 26. Common values: 0x10 (DVB sub,
        /// no AR signalling), 0x14 (DVB sub for 4:3 aspect-ratio).
        subtitling_type: u8,
        /// Composition page identifier (0..=0xFFFF).
        composition_page_id: u16,
        /// Ancillary page identifier (0..=0xFFFF).
        ancillary_page_id: u16,
    },
    /// DVB teletext. Per ETSI EN 300 468 §6.2.43 + ETSI EN 300 706.
    DvbTeletext {
        /// ISO 639-2 language code, lowercase ASCII.
        language: [u8; 3],
        /// 5-bit teletext_type. Common values: 0x01 (initial page),
        /// 0x02 (subtitle page), 0x05 (programme schedule).
        teletext_type: u8,
        /// Magazine number, 0..=7. (3-bit field.)
        magazine_number: u8,
        /// BCD-encoded page number, 0x00..=0x99. The convention for
        /// subtitles is magazine 8 page 88 (= magazine_number=0,
        /// page_number=0x88 in this representation since
        /// magazine "8" wraps to 0 in the 3-bit field).
        page_number: u8,
    },
    /// CEA-708 caption data carried as a separate elementary stream
    /// (rather than embedded in H.264 / H.265 SEI). **Informal industry
    /// convention** — ATSC A/53 Part 4 §6.2.3 defines `"GA94"` as the
    /// `user_data_identifier` for caption data **embedded in MPEG-2
    /// video user_data**, not as a stream-level marker for a standalone
    /// CEA-708 elementary stream. No published spec defines this carriage
    /// form; the auto-emitted `registration_descriptor` with
    /// `format_identifier = "GA94"` is interop-with-ATSC-ecosystem-tooling
    /// best-effort.
    Cea708Standalone,
    /// WebVTT cues carried inside MPEG-TS PES. **Informal industry
    /// convention** — neither RFC 8216 nor draft-pantos-hls-rfc8216bis
    /// nor any published spec defines WebVTT-in-MPEG-TS PES carriage.
    /// The `"VTTC"` `format_identifier` is a ffmpeg `mpegtsenc.c`
    /// convention recognized by hls.js v1.7+ and mediamtx, not a
    /// normatively-defined codepoint. Auto-emits `registration_descriptor`
    /// with `format_identifier = "VTTC"`.
    WebVttInTs,
}

/// Classifier for the four supported stream classes carried in an MPEG-TS
/// program. Used by [`MuxError`] variants whose semantics are
/// stream-kind-specific (e.g., [`MuxError::AmbiguousTarget`],
/// [`MuxError::InvalidStreamHandle`], [`MuxError::DescriptorIndexOutOfRange`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StreamKind {
    Video,
    Audio,
    Klv,
    Subtitle,
}

impl core::fmt::Display for StreamKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            StreamKind::Video => "video",
            StreamKind::Audio => "audio",
            StreamKind::Klv => "klv",
            StreamKind::Subtitle => "subtitle",
        })
    }
}

/// Field-name discriminator inside a teletext-stream configuration block;
/// used by [`MuxError::InvalidTeletextField`] in place of `&'static str`
/// tagging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TeletextField {
    MagazineNumber,
    TeletextType,
}

impl core::fmt::Display for TeletextField {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            TeletextField::MagazineNumber => "magazine_number",
            TeletextField::TeletextType => "teletext_type",
        })
    }
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
    Audio {
        /// PID for the audio PES stream. Must be in `0x0010..=0x1FFE`.
        pid: u16,
        /// Audio codec — drives PMT stream_type (0x03 MP2, 0x0F AAC, 0x11 LATM, 0x81 AC-3).
        codec: AudioCodec,
        /// Optional ISO 639-2 language code (3 lowercase ASCII bytes, e.g. `*b"eng"`).
        ///
        /// When `Some`, the muxer auto-emits an `iso_639_language_descriptor`
        /// (tag `0x0A`, ISO/IEC 13818-1 §2.6.18-19) with `audio_type=0x00`
        /// (undefined / clean main). When `None`, no descriptor is emitted.
        ///
        /// Suppressed when the caller has already supplied a tag-`0x0A`
        /// descriptor via `stream_descriptors_for_audio` — same posture
        /// as KLVA / AV01 / AC-3 registration auto-emit. The auto-emit
        /// itself is wired in the PMT descriptor writer; this field exists,
        /// defaults to `None`, and is plumbed through from the builder helpers.
        language: Option<[u8; 3]>,
    },
    Subtitle {
        /// PID for the subtitle PES stream. Must be in `0x0010..=0x1FFE`.
        pid: u16,
        /// Subtitle codec — all variants emit PMT `stream_type = 0x06`;
        /// the auto-emitted PMT descriptor disambiguates.
        codec: SubtitleCodec,
    },
}

impl StreamSpec {
    pub(crate) fn pid(&self) -> u16 {
        match self {
            StreamSpec::Video { pid, .. } => *pid,
            StreamSpec::Klv { pid, .. } => *pid,
            StreamSpec::Audio { pid, .. } => *pid,
            StreamSpec::Subtitle { pid, .. } => *pid,
        }
    }
}

/// Opaque handle to a configured video stream on a `Muxer`.
///
/// Obtained from [`Muxer::video_handles`] / [`Muxer::video_stream_handle`] /
/// [`Muxer::video_handles_for_program`].
/// Handles are valid only on the muxer that produced them; passing a handle
/// to a different muxer is rejected with [`MuxError::InvalidStreamHandle`].
///
/// The internal representation encodes `(program_index, within_program_index)`
/// in a packed `u32`. Callers treat this as an opaque token.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoStreamHandle(u32);

impl std::fmt::Debug for VideoStreamHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (prog, within) = self.unpack();
        write!(f, "VideoStreamHandle(prog={prog}, stream={within})")
    }
}

/// Opaque handle to a configured KLV stream on a `Muxer`.
///
/// Same semantics as [`VideoStreamHandle`] but for KLV streams.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct KlvStreamHandle(u32);

impl std::fmt::Debug for KlvStreamHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (prog, within) = self.unpack();
        write!(f, "KlvStreamHandle(prog={prog}, stream={within})")
    }
}

impl VideoStreamHandle {
    /// Pack `(program_index, within_program_index)` into the opaque u32.
    ///
    /// Bit layout: bits 0..=3 = within_program_index (0..=15),
    /// bits 4..=7 = program_index (0..=15), upper bits zero.
    ///
    /// Public so `srt-c` can construct handles at the FFI boundary. Single-
    /// program callers pass `program_index = 0`.
    pub fn pack(program_index: usize, within_index: usize) -> Self {
        debug_assert!(program_index < MAX_PROGRAMS);
        debug_assert!(within_index < 16);
        Self(((program_index as u32) << 4) | (within_index as u32))
    }

    /// Unpack the opaque u32 into `(program_index, within_program_index)`.
    pub fn unpack(self) -> (usize, usize) {
        let prog = ((self.0 >> 4) & 0x0F) as usize;
        let within = (self.0 & 0x0F) as usize;
        (prog, within)
    }

    /// Return the packed `u32` representation. Used at the FFI boundary when
    /// `srt-c` needs to return a handle to a C caller as a bare integer.
    pub fn raw(self) -> u32 {
        self.0
    }

    /// Wrap a raw packed `u32` handle that was previously produced by
    /// [`pack`](Self::pack) and returned to a C caller. Use this at FFI
    /// push-time entry points where the handle is already packed — calling
    /// `pack(0, raw)` would be wrong because it re-encodes `raw` as a
    /// `within_index`, which trips the `within_index < 16` debug-assert for
    /// any out-of-range value the C caller passes (e.g. an invalid-handle
    /// test fixture with value 99).
    pub fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    #[cfg(test)]
    pub(crate) fn for_test(raw: usize) -> Self {
        // Bypass packing — store raw as opaque u32 so out-of-range test values
        // (e.g. 99) survive the debug_assert in pack() without triggering it.
        Self(raw as u32)
    }
}

impl KlvStreamHandle {
    /// Pack `(program_index, within_program_index)` into the opaque u32.
    ///
    /// Same bit layout as [`VideoStreamHandle::pack`]. Public so `srt-c`
    /// can construct handles at the FFI boundary.
    pub fn pack(program_index: usize, within_index: usize) -> Self {
        debug_assert!(program_index < MAX_PROGRAMS);
        debug_assert!(within_index < 16);
        Self(((program_index as u32) << 4) | (within_index as u32))
    }

    /// Unpack the opaque u32 into `(program_index, within_program_index)`.
    pub fn unpack(self) -> (usize, usize) {
        let prog = ((self.0 >> 4) & 0x0F) as usize;
        let within = (self.0 & 0x0F) as usize;
        (prog, within)
    }

    /// Return the packed `u32` representation. Used at the FFI boundary when
    /// `srt-c` needs to return a handle to a C caller as a bare integer.
    pub fn raw(self) -> u32 {
        self.0
    }

    /// Wrap a raw packed `u32` handle that was previously produced by
    /// [`pack`](Self::pack) and returned to a C caller. Same semantics as
    /// [`VideoStreamHandle::from_raw`].
    pub fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    #[cfg(test)]
    pub(crate) fn for_test(raw: usize) -> Self {
        // Bypass packing — store raw as opaque u32 so out-of-range test values
        // survive the debug_assert in pack() without triggering it.
        Self(raw as u32)
    }
}

/// Opaque handle to a configured audio stream on a `Muxer`.
///
/// Obtained from [`Muxer::audio_handles`] / [`Muxer::audio_handles_for_program`].
/// Handles are valid only on the muxer that produced them; passing a handle
/// to a different muxer is rejected with [`MuxError::InvalidStreamHandle`].
///
/// The internal representation encodes `(program_index, within_program_index)`
/// in a packed `u32`. Callers treat this as an opaque token.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioStreamHandle(u32);

impl std::fmt::Debug for AudioStreamHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (prog, within) = self.unpack();
        write!(f, "AudioStreamHandle(prog={prog}, stream={within})")
    }
}

impl AudioStreamHandle {
    /// Pack `(program_index, within_program_index)` into the opaque u32.
    /// Both inputs are bounded by `MAX_PROGRAMS` and 16 respectively;
    /// out-of-range arguments trip a `debug_assert!`.
    pub fn pack(program_index: usize, within_index: usize) -> Self {
        debug_assert!(program_index < MAX_PROGRAMS);
        debug_assert!(within_index < 16);
        Self(((program_index as u32) << 4) | (within_index as u32 & 0x0F))
    }

    /// Inverse of `pack`. Returns `(program_index, within_index)`.
    pub fn unpack(self) -> (usize, usize) {
        let prog = ((self.0 >> 4) & 0x0F) as usize;
        let within = (self.0 & 0x0F) as usize;
        (prog, within)
    }

    /// Wrap an already-packed `u32` (used at the C ABI boundary in `srt-c`
    /// when handles arrive from the C caller).
    pub fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

/// Per-program upper bound on subtitle streams. Total program-stream
/// cap with all kinds saturated: ≤16 video + ≤16 KLV + ≤16 audio +
/// ≤16 subtitle = ≤64; well within the PMT single-section limit.
pub const MAX_SUBTITLE_STREAMS_PER_PROGRAM: usize = 16;

/// Opaque handle to a configured subtitle stream on a `Muxer`.
///
/// Obtained from [`Muxer::subtitle_handles`] /
/// [`Muxer::subtitle_handles_for_program`]. Handles are valid only on
/// the muxer that produced them; passing a handle to a different
/// muxer is rejected with [`MuxError::InvalidStreamHandle`].
///
/// The internal representation encodes `(program_index,
/// within_program_index)` in a packed `u32`. Callers treat this as an
/// opaque token.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubtitleStreamHandle(u32);

impl std::fmt::Debug for SubtitleStreamHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (prog, within) = self.unpack();
        write!(f, "SubtitleStreamHandle(prog={prog}, stream={within})")
    }
}

impl SubtitleStreamHandle {
    /// Pack `(program_index, within_program_index)` into the opaque u32.
    ///
    /// Bit layout: bits 0..=3 = within_program_index
    /// (0..=`MAX_SUBTITLE_STREAMS_PER_PROGRAM`-1), bits 4..=7 =
    /// program_index (0..=`MAX_PROGRAMS`-1), upper bits zero.
    ///
    /// Public so `srt-c` can construct handles at the FFI boundary.
    /// Single-program callers pass `program_index = 0`.
    pub fn pack(program_index: usize, within_index: usize) -> Self {
        debug_assert!(program_index < MAX_PROGRAMS);
        debug_assert!(within_index < MAX_SUBTITLE_STREAMS_PER_PROGRAM);
        Self(((program_index as u32) << 4) | (within_index as u32 & 0x0F))
    }

    /// Unpack the opaque u32 into `(program_index, within_program_index)`.
    pub fn unpack(self) -> (usize, usize) {
        let prog = ((self.0 >> 4) & 0x0F) as usize;
        let within = (self.0 & 0x0F) as usize;
        (prog, within)
    }

    /// Return the packed `u32` representation. Used at the FFI boundary when
    /// `srt-c` needs to return a handle to a C caller as a bare integer.
    pub fn raw(self) -> u32 {
        self.0
    }

    /// Wrap a raw packed `u32` handle that was previously produced by
    /// [`pack`](Self::pack) and returned to a C caller. Use this at FFI
    /// push-time entry points where the handle is already packed — calling
    /// `pack(0, raw)` would be wrong because it re-encodes `raw` as a
    /// `within_index`, which trips the `within_index <
    /// MAX_SUBTITLE_STREAMS_PER_PROGRAM` debug-assert for any out-of-range
    /// value the C caller passes (e.g. an invalid-handle test fixture with
    /// value 99).
    pub fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

/// Maximum number of programs in one transport stream multiplex.
/// Mirrors the per-program 16-video + 16-KLV stream caps; far above any
/// realistic gimbaled-platform aggregation use case.
pub const MAX_PROGRAMS: usize = 16;

/// One program in a multi-program TS multiplex. Each program has its own
/// PMT (carried on `pmt_pid`), its own PCR (driven by `pcr_pid` or
/// auto-falling-back to the first video stream's PID), and its own
/// elementary stream set.
#[derive(Debug, Clone)]
pub struct ProgramConfig {
    /// Program number (PAT entry). Must be > 0 (program 0 is reserved for
    /// network information). Must be unique across all programs in the Config.
    pub program_number: u16,

    /// PID carrying this program's PMT. PAT lists `(program_number, pmt_pid)`
    /// tuples. Must not collide with any stream PID in any program, and must
    /// be unique across all programs in the Config.
    pub pmt_pid: u16,

    /// Elementary streams in this program. ≤16 video, ≤16 KLV, ≥1 of either.
    pub streams: Vec<StreamSpec>,

    /// PID carrying this program's PCR. `None` = first video stream's PID,
    /// or first KLV stream's PID if the program is KLV-only.
    pub pcr_pid: Option<u16>,

    /// Caller-supplied descriptors at the program (PMT-level) loop, before
    /// the per-stream descriptor loops. Each `Vec<u8>` is one complete TLV
    /// (tag + length + body).
    pub program_descriptors: Vec<Vec<u8>>,

    /// Per-stream descriptors. Outer Vec indexed parallel to `streams`;
    /// inner is the descriptor list for that stream. Hand-built `ProgramConfig`
    /// callers must keep `stream_descriptors.len() == streams.len()`;
    /// `ConfigBuilder::build()` enforces this.
    pub stream_descriptors: Vec<Vec<Vec<u8>>>,
}

impl ProgramConfig {
    /// Returns the PID of the first video stream in this program, if any.
    pub(crate) fn first_video_pid(&self) -> Option<u16> {
        self.streams.iter().find_map(|s| match s {
            StreamSpec::Video { pid, .. } => Some(*pid),
            _ => None,
        })
    }

    /// Returns the PID of the first KLV stream in this program, if any.
    pub(crate) fn first_klv_pid(&self) -> Option<u16> {
        self.streams.iter().find_map(|s| match s {
            StreamSpec::Klv { pid, .. } => Some(*pid),
            _ => None,
        })
    }

    /// Returns the PID of the first audio stream in this program, if any.
    pub(crate) fn first_audio_pid(&self) -> Option<u16> {
        self.streams.iter().find_map(|s| match s {
            StreamSpec::Audio { pid, .. } => Some(*pid),
            _ => None,
        })
    }
}

/// Muxer construction parameters.
///
/// Contains one or more [`ProgramConfig`]s. Multi-program transport streams
/// carry a PAT that lists all programs; each program has its own PMT.
///
/// Construct with [`Config::builder()`] for ergonomic chaining, or directly
/// with field updates over [`Config::default()`] for the canonical
/// single-program single-video-plus-single-KLV case.
#[derive(Debug, Clone)]
pub struct Config {
    /// Programs in this multiplex. ≤ `MAX_PROGRAMS`, ≥ 1.
    pub programs: Vec<ProgramConfig>,

    /// PCR re-emission interval, in milliseconds. Default 40. Validation 1..=100.
    /// Applied per-program (each program's PCR PID re-emits independently).
    pub pcr_interval_ms: u32,

    /// PAT/PMT re-emission interval, in milliseconds. Default 100. Validation >= 10.
    /// One PAT + N PMTs emitted per tick.
    pub psi_interval_ms: u32,

    /// Maximum buffered TS packets before push returns `BufferFull`.
    /// Default 10000 (~1.88 MB, ~600 ms at 25 Mbps). Validation: >= 10.
    pub buffer_packets: usize,
}

impl Default for Config {
    fn default() -> Self {
        // Single program: H.264 video at 0x1011, KLV PrivateData at 0x1031,
        // async KLV (no PTS), PCR auto-resolved to first video stream.
        Self {
            programs: vec![ProgramConfig {
                program_number: 1,
                pmt_pid: 0x1000,
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
                program_descriptors: Vec::new(),
                stream_descriptors: vec![Vec::new(), Vec::new()],
            }],
            pcr_interval_ms: 40,
            psi_interval_ms: 100,
            buffer_packets: 10_000,
        }
    }
}

impl Config {
    /// Start a new builder. Equivalent to `ConfigBuilder::default()`.
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }

    /// Validate the configuration. Returns a `MuxError` describing the first
    /// failed rule.
    pub fn validate(&self) -> Result<(), MuxError> {
        if self.programs.is_empty() {
            return Err(MuxError::InvalidConfig("at least one program is required"));
        }
        if self.programs.len() > MAX_PROGRAMS {
            return Err(MuxError::TooManyPrograms {
                count: self.programs.len(),
                cap: MAX_PROGRAMS,
            });
        }

        // Per-program validation.
        for prog in &self.programs {
            if prog.program_number == 0 {
                return Err(MuxError::InvalidConfig(
                    "program_number 0 is reserved for network information",
                ));
            }
            if !pid::is_user_pid(prog.pmt_pid) {
                return Err(MuxError::InvalidConfig(
                    "pmt_pid must be in 0x0010..=0x1FFE",
                ));
            }
            if prog.streams.is_empty() {
                return Err(MuxError::EmptyProgram {
                    program_number: prog.program_number,
                });
            }

            // Per-program stream cap (mirrors plan #14).
            const VIDEO_CAP: usize = 16;
            const KLV_CAP: usize = 16;
            const AUDIO_CAP: usize = 16;
            const SUBTITLE_CAP: usize = MAX_SUBTITLE_STREAMS_PER_PROGRAM;
            let mut video_count = 0;
            let mut klv_count = 0;
            let mut audio_count = 0;
            let mut subtitle_count = 0;
            for s in &prog.streams {
                match s {
                    StreamSpec::Video { .. } => video_count += 1,
                    StreamSpec::Klv { .. } => klv_count += 1,
                    StreamSpec::Audio { .. } => audio_count += 1,
                    StreamSpec::Subtitle { .. } => subtitle_count += 1,
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
            if audio_count > AUDIO_CAP {
                return Err(MuxError::TooManyAudioStreams {
                    count: audio_count,
                    cap: AUDIO_CAP,
                });
            }
            if subtitle_count > SUBTITLE_CAP {
                return Err(MuxError::TooManySubtitleStreams {
                    count: subtitle_count,
                    cap: SUBTITLE_CAP,
                });
            }

            // Subtitle-only programs cannot resolve a PCR PID. Subtitles
            // must NOT carry PCR per ETSI EN 300 472 §4.0 + EN 300 743 §6.1,
            // and the PCR fallback chain in `Muxer::new` (caller-pinned >
            // video > KLV > audio) excludes subtitles deliberately. Reject
            // at validate-time rather than panicking at runtime.
            if video_count == 0 && klv_count == 0 && audio_count == 0 {
                return Err(MuxError::SubtitleOnlyProgram {
                    program_number: prog.program_number,
                });
            }

            // Per-stream validation (PID range, KLV invariant).
            for s in &prog.streams {
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
                    StreamSpec::Audio { pid, .. } => {
                        if !pid::is_user_pid(*pid) {
                            return Err(MuxError::InvalidConfig(
                                "audio pid must be in 0x0010..=0x1FFE",
                            ));
                        }
                    }
                    StreamSpec::Subtitle { pid, .. } => {
                        if !pid::is_user_pid(*pid) {
                            return Err(MuxError::InvalidConfig(
                                "subtitle pid must be in 0x0010..=0x1FFE",
                            ));
                        }
                    }
                }
            }

            // Within-program PID uniqueness.
            for (i, s1) in prog.streams.iter().enumerate() {
                for s2 in &prog.streams[i + 1..] {
                    if s1.pid() == s2.pid() {
                        return Err(MuxError::InvalidConfig(
                            "stream PIDs within a program must be distinct",
                        ));
                    }
                }
            }

            // pmt_pid must not collide with any stream PID in this program.
            if prog.streams.iter().any(|s| s.pid() == prog.pmt_pid) {
                return Err(MuxError::PmtPidConflictsWithStream {
                    pmt_pid: prog.pmt_pid,
                    program_number: prog.program_number,
                });
            }

            // pcr_pid (if specified) must equal a configured stream's PID.
            if let Some(pcr) = prog.pcr_pid {
                if !prog.streams.iter().any(|s| s.pid() == pcr) {
                    return Err(MuxError::InvalidConfig(
                        "pcr_pid must equal a configured stream PID in the same program",
                    ));
                }
            }

            // Resolve effective PCR PID (caller-pinned or fallback) and
            // reject when it lands on a KLV stream — KLV cadence is too
            // sparse for ETSI TR 101 290 §5.6.1's 100 ms ceiling and
            // today's deterministic muxer can't emit standalone PCR-only
            // TS packets between push events.
            let effective_pcr_pid = prog.pcr_pid.or_else(|| {
                prog.first_video_pid()
                    .or_else(|| prog.first_klv_pid())
                    .or_else(|| prog.first_audio_pid())
            });
            if let Some(pcr) = effective_pcr_pid {
                let lands_on_klv = prog
                    .streams
                    .iter()
                    .any(|s| matches!(s, StreamSpec::Klv { pid, .. } if *pid == pcr));
                if lands_on_klv {
                    return Err(MuxError::KlvPidUsedAsPcrPid { pid: pcr });
                }
            }

            // Subtitle-specific validation: reject PCR-PID pinning to a
            // subtitle PID (subtitles are too sparse for PCR pacing) and
            // validate per-codec parameter ranges (language code shape,
            // teletext field bit-widths).
            for s in &prog.streams {
                if let StreamSpec::Subtitle { pid, codec } = s {
                    if prog.pcr_pid == Some(*pid) {
                        return Err(MuxError::SubtitlePidUsedAsPcrPid { pid: *pid });
                    }
                    match codec {
                        SubtitleCodec::DvbSubtitling { language, .. } => {
                            validate_language_code(*language)?;
                        }
                        SubtitleCodec::DvbTeletext {
                            language,
                            magazine_number,
                            teletext_type,
                            ..
                        } => {
                            validate_language_code(*language)?;
                            if *magazine_number > 7 {
                                return Err(MuxError::InvalidTeletextField {
                                    field: "magazine_number",
                                    value: *magazine_number,
                                    max: 7,
                                });
                            }
                            if *teletext_type > 0x1F {
                                return Err(MuxError::InvalidTeletextField {
                                    field: "teletext_type",
                                    value: *teletext_type,
                                    max: 0x1F,
                                });
                            }
                        }
                        SubtitleCodec::Cea708Standalone | SubtitleCodec::WebVttInTs => {}
                    }
                }
            }

            // stream_descriptors length match + per-TLV well-formedness.
            if prog.stream_descriptors.len() != prog.streams.len() {
                return Err(MuxError::InvalidConfig(
                    "stream_descriptors.len() must equal streams.len()",
                ));
            }
            for (si, descs) in prog.stream_descriptors.iter().enumerate() {
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

            // Per-program PMT size budget. Reject configs that would produce a
            // PMT section body larger than 183 bytes (one TS packet payload).
            let pmt_size = crate::mpegts::mux::psi::estimate_pmt_section_size(prog);
            if pmt_size > crate::mpegts::mux::psi::MAX_PMT_SECTION_BYTES {
                return Err(MuxError::PmtTooLarge {
                    used_bytes: pmt_size,
                    max_bytes: crate::mpegts::mux::psi::MAX_PMT_SECTION_BYTES,
                });
            }
        }

        // Cross-program checks: program_number unique, pmt_pid unique, all
        // stream PIDs unique across programs.
        for (i, p1) in self.programs.iter().enumerate() {
            for p2 in &self.programs[i + 1..] {
                if p1.program_number == p2.program_number {
                    return Err(MuxError::DuplicateProgramNumber {
                        program_number: p1.program_number,
                    });
                }
                if p1.pmt_pid == p2.pmt_pid {
                    return Err(MuxError::DuplicatePmtPid {
                        pid: p1.pmt_pid,
                        programs: [p1.program_number, p2.program_number],
                    });
                }
                for s1 in &p1.streams {
                    if p2.pmt_pid == s1.pid() {
                        return Err(MuxError::DuplicatePidAcrossPrograms {
                            pid: s1.pid(),
                            programs: [p1.program_number, p2.program_number],
                        });
                    }
                    for s2 in &p2.streams {
                        if s1.pid() == s2.pid() {
                            return Err(MuxError::DuplicatePidAcrossPrograms {
                                pid: s1.pid(),
                                programs: [p1.program_number, p2.program_number],
                            });
                        }
                    }
                }
                for s2 in &p2.streams {
                    if p1.pmt_pid == s2.pid() {
                        return Err(MuxError::DuplicatePidAcrossPrograms {
                            pid: s2.pid(),
                            programs: [p1.program_number, p2.program_number],
                        });
                    }
                }
            }
        }

        // Cadence + buffer.
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

        Ok(())
    }
}

/// Ergonomic construction of [`Config`] with nested `add_program` blocks.
///
/// Use [`Config::builder()`] to obtain a `ConfigBuilder`, then open each
/// program with [`ConfigBuilder::add_program`] (returns a [`ProgramBuilder`]),
/// add streams and descriptors on the `ProgramBuilder`, then close the block
/// with [`ProgramBuilder::end_program`] (returns back to the `ConfigBuilder`).
/// Finish with [`ConfigBuilder::build`].
///
/// ```
/// use tst_core::mpegts::mux::{Config, KlvStreamType, VideoCodec};
///
/// let config = Config::builder()
///     .add_program(1, 0x1000)
///         .add_video(0x1011, VideoCodec::H264)
///         .add_klv(0x1031, KlvStreamType::PrivateData, false)
///         .end_program()
///     .build()
///     .unwrap();
/// ```
#[derive(Default, Debug)]
pub struct ConfigBuilder {
    programs: Vec<ProgramConfig>,
    pcr_interval_ms: Option<u32>,
    psi_interval_ms: Option<u32>,
    buffer_packets: Option<usize>,
}

impl ConfigBuilder {
    /// Begin a new program block. Returns a [`ProgramBuilder`] that owns
    /// `self` (consume-by-value); close the block with
    /// [`ProgramBuilder::end_program`] to recover the `ConfigBuilder`.
    pub fn add_program(mut self, program_number: u16, pmt_pid: u16) -> ProgramBuilder {
        self.programs.push(ProgramConfig {
            program_number,
            pmt_pid,
            streams: Vec::new(),
            pcr_pid: None,
            program_descriptors: Vec::new(),
            stream_descriptors: Vec::new(),
        });
        let idx = self.programs.len() - 1;
        ProgramBuilder { parent: self, idx }
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

    /// Finalize. Returns a validated [`Config`] or an error describing the
    /// failed rule.
    pub fn build(self) -> Result<Config, MuxError> {
        let cfg = Config {
            programs: self.programs,
            pcr_interval_ms: self.pcr_interval_ms.unwrap_or(40),
            psi_interval_ms: self.psi_interval_ms.unwrap_or(100),
            buffer_packets: self.buffer_packets.unwrap_or(10_000),
        };
        cfg.validate()?;
        Ok(cfg)
    }
}

/// Sub-builder for one [`ProgramConfig`]. Returned by
/// [`ConfigBuilder::add_program`]; close with [`ProgramBuilder::end_program`]
/// to return to the outer [`ConfigBuilder`] for additional `.add_program(...)`
/// calls or `.build()`.
///
/// Every method consumes `self` and returns `Self` so calls can be chained.
#[derive(Debug)]
pub struct ProgramBuilder {
    parent: ConfigBuilder,
    /// Index into `parent.programs` for the program under construction.
    idx: usize,
}

impl ProgramBuilder {
    /// Add a video elementary stream to this program.
    pub fn add_video(mut self, pid: u16, codec: VideoCodec) -> Self {
        let prog = &mut self.parent.programs[self.idx];
        prog.streams.push(StreamSpec::Video { pid, codec });
        prog.stream_descriptors.push(Vec::new());
        self
    }

    /// Add a KLV metadata elementary stream to this program.
    pub fn add_klv(mut self, pid: u16, stream_type: KlvStreamType, carries_pts: bool) -> Self {
        let prog = &mut self.parent.programs[self.idx];
        prog.streams.push(StreamSpec::Klv {
            pid,
            stream_type,
            carries_pts,
        });
        prog.stream_descriptors.push(Vec::new());
        self
    }

    /// Add an audio elementary stream to this program.
    ///
    /// `pid` must be in `0x0010..=0x1FFE` and distinct from all other PIDs in
    /// this program. `codec` drives the PMT `stream_type` byte.
    pub fn add_audio(mut self, pid: u16, codec: AudioCodec) -> Self {
        let prog = &mut self.parent.programs[self.idx];
        prog.streams.push(StreamSpec::Audio {
            pid,
            codec,
            language: None,
        });
        prog.stream_descriptors.push(Vec::new());
        self
    }

    /// Like [`add_audio`] but emits an `iso_639_language_descriptor`
    /// (ISO/IEC 13818-1 §2.6.18) on the PMT entry. Three-byte ISO 639-2
    /// language code, lowercase ASCII (e.g. `*b"eng"`, `*b"deu"`,
    /// `*b"jpn"`). `audio_type` is set to `0x00` (undefined / clean main)
    /// per §2.6.19 Table 2-83.
    ///
    /// For richer audio_type semantics or multi-language tracks, supply
    /// the descriptor manually via [`stream_descriptors_for_audio`].
    pub fn add_audio_with_language(
        mut self,
        pid: u16,
        codec: AudioCodec,
        language: [u8; 3],
    ) -> Self {
        let prog = &mut self.parent.programs[self.idx];
        prog.streams.push(StreamSpec::Audio {
            pid,
            codec,
            language: Some(language),
        });
        prog.stream_descriptors.push(Vec::new());
        self
    }

    /// Add a subtitle elementary stream to this program.
    ///
    /// `pid` must be in `0x0010..=0x1FFE` and distinct from all other PIDs
    /// in this program. All four `SubtitleCodec` variants emit PMT
    /// `stream_type = 0x06` (PrivateData); the per-stream PMT descriptor
    /// disambiguates the codec at the wire level.
    pub fn add_subtitle(mut self, pid: u16, codec: SubtitleCodec) -> Self {
        let prog = &mut self.parent.programs[self.idx];
        prog.streams.push(StreamSpec::Subtitle { pid, codec });
        prog.stream_descriptors.push(Vec::new());
        self
    }

    /// Pin this program's PCR to a specific PID. Default: first video stream's
    /// PID (or first KLV PID for KLV-only programs).
    pub fn pcr_pid(mut self, pid: u16) -> Self {
        self.parent.programs[self.idx].pcr_pid = Some(pid);
        self
    }

    /// Set program-level descriptors (PMT program info loop, before per-stream
    /// entries). Each `Vec<u8>` is one complete descriptor TLV.
    pub fn program_descriptors(mut self, descs: Vec<Vec<u8>>) -> Self {
        self.parent.programs[self.idx].program_descriptors = descs;
        self
    }

    /// Set the descriptor list for the `video_idx`-th video stream in this
    /// program (zero-indexed among `StreamSpec::Video` entries in add-order).
    ///
    /// # Panics
    /// Panics if `video_idx` is out of range relative to the number of video
    /// streams added so far. Call after the corresponding [`add_video`][Self::add_video].
    pub fn stream_descriptors_for_video(mut self, video_idx: usize, descs: Vec<Vec<u8>>) -> Self {
        let prog = &mut self.parent.programs[self.idx];
        let abs_idx = prog
            .streams
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, StreamSpec::Video { .. }))
            .nth(video_idx)
            .map(|(i, _)| i)
            .unwrap_or_else(|| {
                panic!(
                    "video_idx {video_idx} out of range — call after add_video (program {})",
                    prog.program_number
                )
            });
        prog.stream_descriptors[abs_idx] = descs;
        self
    }

    /// Set the descriptor list for the `klv_idx`-th KLV stream in this
    /// program (zero-indexed among `StreamSpec::Klv` entries in add-order).
    ///
    /// # Panics
    /// Panics if `klv_idx` is out of range. Call after the corresponding
    /// [`add_klv`][Self::add_klv].
    pub fn stream_descriptors_for_klv(mut self, klv_idx: usize, descs: Vec<Vec<u8>>) -> Self {
        let prog = &mut self.parent.programs[self.idx];
        let abs_idx = prog
            .streams
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, StreamSpec::Klv { .. }))
            .nth(klv_idx)
            .map(|(i, _)| i)
            .unwrap_or_else(|| {
                panic!(
                    "klv_idx {klv_idx} out of range — call after add_klv (program {})",
                    prog.program_number
                )
            });
        prog.stream_descriptors[abs_idx] = descs;
        self
    }

    /// Set the descriptor list for the `audio_idx`-th audio stream in this
    /// program (zero-indexed among `StreamSpec::Audio` entries in add-order).
    ///
    /// # Panics
    /// Panics if `audio_idx` is out of range. Call after the corresponding
    /// [`add_audio`][Self::add_audio].
    pub fn stream_descriptors_for_audio(mut self, audio_idx: usize, descs: Vec<Vec<u8>>) -> Self {
        let prog = &mut self.parent.programs[self.idx];
        let abs_idx = prog
            .streams
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, StreamSpec::Audio { .. }))
            .nth(audio_idx)
            .map(|(i, _)| i)
            .unwrap_or_else(|| {
                panic!(
                    "audio_idx {audio_idx} out of range — call after add_audio (program {})",
                    prog.program_number
                )
            });
        prog.stream_descriptors[abs_idx] = descs;
        self
    }

    /// Set the descriptor list for the `subtitle_idx`-th subtitle stream in
    /// this program (zero-indexed among `StreamSpec::Subtitle` entries in
    /// add-order).
    ///
    /// Caller-supplied descriptors append to the auto-emitted codec-
    /// disambiguating descriptor; they do not suppress it (contrast with
    /// KLV's KLVA-suppression rule — for subtitles, the auto-emit IS the
    /// codec marker for receiver classification).
    ///
    /// # Panics
    /// Panics if `subtitle_idx` is out of range. Call after the corresponding
    /// [`add_subtitle`][Self::add_subtitle].
    pub fn stream_descriptors_for_subtitle(
        mut self,
        subtitle_idx: usize,
        descs: Vec<Vec<u8>>,
    ) -> Self {
        let prog = &mut self.parent.programs[self.idx];
        let abs_idx = prog
            .streams
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, StreamSpec::Subtitle { .. }))
            .nth(subtitle_idx)
            .map(|(i, _)| i)
            .unwrap_or_else(|| {
                panic!(
                    "subtitle_idx {subtitle_idx} out of range — call after add_subtitle (program {})",
                    prog.program_number
                )
            });
        prog.stream_descriptors[abs_idx] = descs;
        self
    }

    /// Set the descriptor list for a stream by absolute index within this
    /// program (across both video and KLV streams in add-order).
    ///
    /// # Panics
    /// Panics if `abs_idx` is out of range.
    pub fn stream_descriptors_for_stream(mut self, abs_idx: usize, descs: Vec<Vec<u8>>) -> Self {
        let prog = &mut self.parent.programs[self.idx];
        assert!(
            abs_idx < prog.streams.len(),
            "abs_idx {abs_idx} out of range (program {} has {} streams)",
            prog.program_number,
            prog.streams.len()
        );
        prog.stream_descriptors[abs_idx] = descs;
        self
    }

    /// Close this program block and return to the outer [`ConfigBuilder`].
    pub fn end_program(self) -> ConfigBuilder {
        self.parent
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
    /// Number of programs (PAT entries) in this muxer's configuration.
    pub programs_configured: u32,
    /// Number of subtitle streams configured across all programs in this
    /// muxer. Counts the `StreamSpec::Subtitle` entries from
    /// `Config::programs`.
    pub subtitle_streams_configured: u32,
    /// Per-stream counters, keyed by PID. One entry per configured
    /// video or KLV stream. `StreamStats::items` = push_video_to /
    /// push_klv_to call count; `StreamStats::bytes` = raw ES bytes pushed
    /// (before PES/TS framing overhead).
    pub per_stream: BTreeMap<u16, crate::mpegts::stats::StreamStats>,
}

use self::pes::{
    MAX_PES_HEADER_SIZE, PesFlags, PesPtsField, STREAM_ID_KLV, STREAM_ID_PRIVATE_STREAM_1,
    STREAM_ID_VIDEO, SubtitlePesShape, write_audio_pes, write_pes_header, write_subtitle_pes,
};
use self::psi::{KLVA_REGISTRATION_DESCRIPTOR, PmtStreamEntry, write_pat_packet, write_pmt_packet};
use self::ts::{AdaptationField, ContinuityCounters, write_packet};

/// Per-video-stream cached state. Built once at `Muxer::new` time.
struct VideoStreamState {
    pid: u16,
    codec: VideoCodec,
}

/// Per-KLV-stream cached state.
struct KlvStreamState {
    pid: u16,
    stream_type: KlvStreamType,
    carries_pts: bool,
    /// For `SynchronousMetadata` streams: incrementing AU cell `sequence_number`,
    /// wraps modulo 256 per H.222.0 §2.12.4.2 Table 2-156 semantics. Unused
    /// for `PrivateData` streams.
    au_cell_sequence_number: u8,
}

/// Per-audio-stream cached state.
struct AudioStreamState {
    pid: u16,
    codec: AudioCodec,
}

/// Per-subtitle-stream cached state. `codec` is `Clone` (not `Copy`) so we
/// store it owned per-stream — same shape as `SubtitleCodec` itself.
struct SubtitleStreamState {
    pid: u16,
    codec: SubtitleCodec,
}

/// MuxSender-side MPEG-TS muxer.
///
/// Construct with `Muxer::new(config)`, push encoded frames via `push_video`
/// and `push_klv`, then drain TS packets with `pull`. The muxer is
/// deterministic — output is a function of inputs only, not wall-clock time.
pub struct Muxer {
    config: Config,

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
}

impl Muxer {
    /// Construct and validate.
    pub fn new(config: Config) -> Result<Self, MuxError> {
        config.validate()?;

        let n_programs = config.programs.len();
        let pcr_interval_27mhz = (config.pcr_interval_ms as u64) * 27_000;
        let psi_interval_90khz = (config.psi_interval_ms as i64) * 90;

        // Build per-program state vectors in a single pass over programs.
        let mut video_streams: Vec<Vec<VideoStreamState>> = Vec::with_capacity(n_programs);
        let mut klv_streams: Vec<Vec<KlvStreamState>> = Vec::with_capacity(n_programs);
        let mut audio_streams: Vec<Vec<AudioStreamState>> = Vec::with_capacity(n_programs);
        let mut subtitle_streams: Vec<Vec<SubtitleStreamState>> = Vec::with_capacity(n_programs);
        let mut pcr_pids: Vec<u16> = Vec::with_capacity(n_programs);
        let mut pmt_descriptor_caches: Vec<Vec<Vec<u8>>> = Vec::with_capacity(n_programs);
        let mut per_stream: BTreeMap<u16, crate::mpegts::stats::StreamStats> = BTreeMap::new();

        for prog in &config.programs {
            // Per-program video + KLV stream state.
            let prog_video: Vec<VideoStreamState> = prog
                .streams
                .iter()
                .filter_map(|s| match s {
                    StreamSpec::Video { pid, codec } => Some(VideoStreamState {
                        pid: *pid,
                        codec: *codec,
                    }),
                    _ => None,
                })
                .collect();
            let prog_klv: Vec<KlvStreamState> = prog
                .streams
                .iter()
                .filter_map(|s| match s {
                    StreamSpec::Klv {
                        pid,
                        stream_type,
                        carries_pts,
                    } => Some(KlvStreamState {
                        pid: *pid,
                        stream_type: *stream_type,
                        carries_pts: *carries_pts,
                        au_cell_sequence_number: 0,
                    }),
                    _ => None,
                })
                .collect();
            let prog_audio: Vec<AudioStreamState> = prog
                .streams
                .iter()
                .filter_map(|s| match s {
                    StreamSpec::Audio { pid, codec, .. } => Some(AudioStreamState {
                        pid: *pid,
                        codec: *codec,
                    }),
                    _ => None,
                })
                .collect();
            let prog_subtitle: Vec<SubtitleStreamState> = prog
                .streams
                .iter()
                .filter_map(|s| match s {
                    StreamSpec::Subtitle { pid, codec } => Some(SubtitleStreamState {
                        pid: *pid,
                        codec: codec.clone(),
                    }),
                    _ => None,
                })
                .collect();

            // Resolve PCR PID for this program: caller-pin or auto-fallback.
            // Priority: caller-pinned > first video > first KLV > first audio.
            let pcr_pid = prog.pcr_pid.unwrap_or_else(|| {
                prog.first_video_pid()
                    .or_else(|| prog.first_klv_pid())
                    .or_else(|| prog.first_audio_pid())
                    .expect("validate() guarantees ≥1 stream per program")
            });

            // Pre-compose per-stream descriptor bytes for this program.
            // The auto-emitted KLVA Registration on PrivateData KLV PIDs is
            // suppressed when the caller supplies their own Registration
            // descriptor — TSDuck and ffprobe both flag duplicate Registrations,
            // and the corpus shows real senders never duplicate.
            let mut prog_cache: Vec<Vec<u8>> = Vec::with_capacity(prog.streams.len());
            for (i, spec) in prog.streams.iter().enumerate() {
                let caller_descs = &prog.stream_descriptors[i];
                let caller_has_registration = caller_descs
                    .iter()
                    .any(|tlv| !tlv.is_empty() && tlv[0] == 0x05);

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
                // KLVA Registration auto-emit on KLV streams (both
                // PrivateData=0x06 and SynchronousMetadata=0x15). ffmpeg
                // mpegtsenc.c:817-818 emits KLVA on the metadata
                // stream_type path too — receivers gate KLV
                // classification on the descriptor regardless of
                // stream_type. Sync KLV with metadata_descriptor
                // (tag 0x26) doesn't *replace* KLVA — TSDuck + ffmpeg
                // consume both side-by-side.
                if matches!(
                    spec,
                    StreamSpec::Klv {
                        stream_type: KlvStreamType::PrivateData
                            | KlvStreamType::SynchronousMetadata,
                        ..
                    }
                ) && !caller_has_registration
                {
                    bytes.extend_from_slice(KLVA_REGISTRATION_DESCRIPTOR);
                }
                // AV1 auto-emit: AV01 registration_descriptor (binding §2.1).
                // MUST be the FIRST descriptor in the per-stream PMT loop —
                // receivers gate AV1 classification on stream_type 0x06 +
                // first-position AV01 Registration. Suppress when the caller
                // has already supplied an AV01 Registration (mirrors KLVA
                // suppression). If the caller supplied a Registration with a
                // non-AV01 format_identifier, log warn but still auto-emit so
                // the stream stays classifiable as AV1 — we don't silently
                // override caller intent, but we don't let a stray non-AV01
                // Registration silently break receiver classification either.
                if let StreamSpec::Video {
                    codec: VideoCodec::Av1,
                    ..
                } = spec
                {
                    let caller_has_av01 = caller_descs
                        .iter()
                        .any(|tlv| tlv.len() >= 6 && tlv[0] == 0x05 && &tlv[2..6] == b"AV01");
                    let caller_has_other_registration = caller_descs
                        .iter()
                        .any(|tlv| tlv.len() >= 6 && tlv[0] == 0x05 && &tlv[2..6] != b"AV01");
                    if caller_has_other_registration && !caller_has_av01 {
                        tracing::warn!(
                            "caller-supplied Registration descriptor on AV1 PID has \
                             non-AV01 format_identifier; receivers may not recognize \
                             the stream as AV1"
                        );
                    }
                    if !caller_has_av01 {
                        bytes.extend_from_slice(
                            &crate::mpegts::descriptors::format_identifier_av01(),
                        );
                    }
                }
                // AC-3 auto-emit: Registration descriptor with format_identifier
                // "AC-3" per ATSC A/52 §A.2.3. Receivers use this to distinguish
                // AC-3 from other private-stream-1 (PES stream_id 0xBD) audio.
                // Suppression mirrors the KLVA / AV01 rules: suppress when the
                // caller has already supplied an AC-3 Registration (tag 0x05 with
                // format_identifier == b"AC-3"). If the caller supplied a
                // Registration with a different format_identifier, log warn but
                // do NOT auto-emit — caller intent takes precedence and we don't
                // silently override it.
                if let StreamSpec::Audio {
                    codec: AudioCodec::Ac3,
                    ..
                } = spec
                {
                    let caller_has_ac3 = caller_descs
                        .iter()
                        .any(|tlv| tlv.len() >= 6 && tlv[0] == 0x05 && &tlv[2..6] == b"AC-3");
                    let caller_has_other_registration = caller_descs
                        .iter()
                        .any(|tlv| tlv.len() >= 6 && tlv[0] == 0x05 && &tlv[2..6] != b"AC-3");
                    if caller_has_other_registration && !caller_has_ac3 {
                        tracing::warn!(
                            "caller-supplied Registration descriptor on AC-3 PID has \
                             non-AC-3 format_identifier; receivers may not recognize \
                             the stream as AC-3"
                        );
                    }
                    if !caller_has_ac3 {
                        bytes.extend_from_slice(
                            &crate::mpegts::descriptors::format_identifier_ac3(),
                        );
                    }
                }
                // ISO 639 language descriptor auto-emit on Audio when
                // StreamSpec::Audio.language is Some. Per ISO/IEC 13818-1
                // §2.6.18-19 (tag 0x0A, length 4: 3 lang bytes + 1
                // audio_type byte). audio_type=0x00 (undefined / clean
                // main) is the spec default; richer values come from
                // caller-supplied stream_descriptors_for_audio. Suppress
                // when caller already supplied any tag-0x0A descriptor —
                // caller intent wins (their language code may differ).
                if let StreamSpec::Audio {
                    language: Some(lang),
                    ..
                } = spec
                {
                    let caller_has_lang = caller_descs
                        .iter()
                        .any(|tlv| !tlv.is_empty() && tlv[0] == 0x0A);
                    if !caller_has_lang {
                        bytes.extend_from_slice(&crate::mpegts::descriptors::iso_639_language(
                            *lang, 0x00,
                        ));
                    }
                }
                // Subtitle auto-emit: codec-disambiguating per-stream descriptor.
                // All four SubtitleCodec variants ride PMT stream_type 0x06; the
                // descriptor here is what tells receivers which codec rides on
                // this PID. Mirrors the KLV/AV1 caller-supplied-Registration
                // suppression rule: when the caller has already supplied any
                // descriptor that the receiver-side classifier recognizes as a
                // subtitle codec marker (subtitling 0x59 / teletext 0x56 /
                // VBI teletext 0x46 / Registration with VTTC or GA94
                // format_identifier), the auto-emit is suppressed — caller's
                // takes precedence and we don't double-emit.
                if let StreamSpec::Subtitle { codec, .. } = spec {
                    if !caller_has_recognized_subtitle_descriptor(caller_descs) {
                        let auto = match codec {
                            SubtitleCodec::DvbSubtitling {
                                language,
                                subtitling_type,
                                composition_page_id,
                                ancillary_page_id,
                            } => crate::mpegts::descriptors::subtitling_descriptor(
                                *language,
                                *subtitling_type,
                                *composition_page_id,
                                *ancillary_page_id,
                            ),
                            SubtitleCodec::DvbTeletext {
                                language,
                                teletext_type,
                                magazine_number,
                                page_number,
                            } => crate::mpegts::descriptors::teletext_descriptor(
                                *language,
                                *teletext_type,
                                *magazine_number,
                                *page_number,
                            ),
                            SubtitleCodec::Cea708Standalone => {
                                crate::mpegts::descriptors::format_identifier_ga94()
                            }
                            SubtitleCodec::WebVttInTs => {
                                crate::mpegts::descriptors::format_identifier_vttc()
                            }
                        };
                        bytes.extend_from_slice(&auto);
                    }
                }
                for tlv in caller_descs {
                    bytes.extend_from_slice(tlv);
                }
                prog_cache.push(bytes);
            }

            // Eagerly create per-stream stats entries.
            for v in &prog_video {
                let stream_type_byte = match v.codec {
                    VideoCodec::H264 => StreamType::H264.as_u8(),
                    VideoCodec::H265 => StreamType::H265.as_u8(),
                    VideoCodec::H266 => StreamType::H266.as_u8(),
                    // AV1 rides PMT stream_type 0x06; the AV01
                    // registration_descriptor disambiguates on the receiver
                    // (auto-emitted in the per-stream descriptor cache).
                    VideoCodec::Av1 => StreamType::KlvPrivate.as_u8(),
                };
                per_stream.insert(
                    v.pid,
                    crate::mpegts::stats::StreamStats {
                        pid: v.pid,
                        stream_type: stream_type_byte,
                        program_number: prog.program_number,
                        ..Default::default()
                    },
                );
            }
            for k in &prog_klv {
                let stream_type_byte = match k.stream_type {
                    KlvStreamType::PrivateData => StreamType::KlvPrivate.as_u8(),
                    KlvStreamType::SynchronousMetadata => StreamType::KlvSyncMetadata.as_u8(),
                };
                per_stream.insert(
                    k.pid,
                    crate::mpegts::stats::StreamStats {
                        pid: k.pid,
                        stream_type: stream_type_byte,
                        program_number: prog.program_number,
                        ..Default::default()
                    },
                );
            }
            for a in &prog_audio {
                let stream_type_byte = match a.codec {
                    AudioCodec::Mp2 => StreamType::AudioMp2.as_u8(),
                    AudioCodec::Aac => StreamType::AudioAac.as_u8(),
                    AudioCodec::AacLatm => StreamType::AudioAacLatm.as_u8(),
                    AudioCodec::Ac3 => StreamType::AudioAc3.as_u8(),
                };
                per_stream.insert(
                    a.pid,
                    crate::mpegts::stats::StreamStats {
                        pid: a.pid,
                        stream_type: stream_type_byte,
                        program_number: prog.program_number,
                        ..Default::default()
                    },
                );
            }
            for s in &prog_subtitle {
                // All four subtitle codecs ride PMT stream_type 0x06
                // (PrivateData); the per-stream PMT descriptor
                // disambiguates between DVB-sub, teletext, CEA-708
                // standalone, and WebVTT-in-TS. The codec-derived label
                // is the one human-readable distinguisher in stats.
                per_stream.insert(
                    s.pid,
                    crate::mpegts::stats::StreamStats {
                        pid: s.pid,
                        stream_type: StreamType::KlvPrivate.as_u8(),
                        program_number: prog.program_number,
                        label: Some(
                            crate::mpegts::stats::subtitle_codec_label(&s.codec).to_string(),
                        ),
                        ..Default::default()
                    },
                );
            }

            video_streams.push(prog_video);
            klv_streams.push(prog_klv);
            audio_streams.push(prog_audio);
            subtitle_streams.push(prog_subtitle);
            pcr_pids.push(pcr_pid);
            pmt_descriptor_caches.push(prog_cache);
        }

        Ok(Self {
            config,
            pmt_descriptor_caches,
            video_streams,
            klv_streams,
            audio_streams,
            subtitle_streams,
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
        // is configured across all programs. N=0 and N>1 are both ambiguous.
        let total_video: usize = self.video_streams.iter().map(|v| v.len()).sum();
        if total_video != 1 {
            return Err(MuxError::AmbiguousTarget {
                kind: "video",
                count: total_video,
            });
        }
        let handle = VideoStreamHandle::pack(0, 0);
        self.push_video_to(handle, nal, pts_90khz, key_frame)
    }

    /// Push one KLV metadata blob.
    ///
    /// `pts_90khz` becomes the PES PTS when the KLV stream was configured with
    /// `carries_pts: true` in [`StreamSpec::Klv`]; ignored otherwise.
    /// Returns `Err(MuxError::BufferFull)` like `push_video`.
    ///
    /// `metadata_service_id` is written into the AU cell header per
    /// ITU-T H.222.0 V9 §2.12.4.2 / ST 1402.2 App. B Table 2 **only** when
    /// the stream is configured as [`KlvStreamType::SynchronousMetadata`]
    /// (stream_type 0x15). For [`KlvStreamType::PrivateData`] (0x06) streams
    /// the payload passes through verbatim and this parameter is ignored.
    ///
    /// The spec default is `0x00`. Pass `0x00` unless you have a specific
    /// reason to use a non-zero service_id (e.g. to mirror the `service_id`
    /// byte of a `metadata_klva` PMT descriptor you supplied at config time).
    pub fn push_klv(
        &mut self,
        klv: &[u8],
        pts_90khz: i64,
        metadata_service_id: u8,
    ) -> Result<(), MuxError> {
        let total_klv: usize = self.klv_streams.iter().map(|k| k.len()).sum();
        if total_klv == 0 {
            return Err(MuxError::NoKlvStreamsConfigured);
        }
        if total_klv > 1 {
            return Err(MuxError::AmbiguousTarget {
                kind: "klv",
                count: total_klv,
            });
        }
        let handle = KlvStreamHandle::pack(0, 0);
        self.push_klv_to(handle, klv, pts_90khz, metadata_service_id)
    }

    /// Push one audio frame buffer, single-stream shorthand.
    ///
    /// `pts_90khz` is required and becomes the PES PTS; audio has no DTS
    /// (no B-frame reorder). `frames` is one or more pre-framed audio frames
    /// concatenated by the caller.
    ///
    /// Resolves only when exactly one audio stream is configured across all
    /// programs. Otherwise rejects with [`MuxError::AmbiguousTarget`].
    ///
    /// Returns `Err(MuxError::BufferFull)` if the resulting TS packets would
    /// exceed `Config::buffer_packets`.
    pub fn push_audio(&mut self, frames: &[u8], pts_90khz: i64) -> Result<(), MuxError> {
        let total_audio: usize = self.audio_streams.iter().map(|a| a.len()).sum();
        if total_audio == 0 {
            return Err(MuxError::NoAudioStreamsConfigured);
        }
        if total_audio > 1 {
            return Err(MuxError::AmbiguousTarget {
                kind: "audio",
                count: total_audio,
            });
        }
        // Mirror push_video / push_klv: when exactly one stream exists, it is
        // at (prog_idx=0, within_idx=0) in audio_streams — the first program
        // that has audio is always index 0 in the nested vec. Note: if the lone
        // audio stream is in program 1 (prog_idx=1 in config), audio_streams[1]
        // is non-empty and audio_streams[0] is empty; pack(0,0) would resolve
        // to the wrong slot. Iterate to find the actual location.
        let (prog_idx, _within_idx) = self
            .audio_streams
            .iter()
            .enumerate()
            .find(|(_p, a)| !a.is_empty())
            .map(|(p, _)| (p, 0))
            .expect("total_audio == 1 guarantees one non-empty program");
        let handle = AudioStreamHandle::pack(prog_idx, 0);
        self.push_audio_to(handle, pts_90khz, frames)
    }

    /// Push one audio frame buffer on a specific audio stream.
    ///
    /// Routes to the audio stream identified by `handle`. Use the bare
    /// [`push_audio`][Self::push_audio] shorthand when exactly one audio
    /// stream is configured. Handles are obtained from
    /// [`audio_handles`][Self::audio_handles] /
    /// [`audio_handles_for_program`][Self::audio_handles_for_program].
    ///
    /// Returns [`MuxError::InvalidStreamHandle`] if the handle's index is out
    /// of range for this muxer's configured audio stream count.
    /// Returns `Err(MuxError::BufferFull)` if the resulting TS packets would
    /// exceed `Config::buffer_packets`.
    pub fn push_audio_to(
        &mut self,
        handle: AudioStreamHandle,
        pts_90khz: i64,
        frames: &[u8],
    ) -> Result<(), MuxError> {
        let (prog_idx, within_idx) = handle.unpack();
        if prog_idx >= self.audio_streams.len() || within_idx >= self.audio_streams[prog_idx].len()
        {
            return Err(MuxError::InvalidStreamHandle {
                kind: "audio",
                index: handle.0 as usize,
            });
        }
        let audio_pid = self.audio_streams[prog_idx][within_idx].pid;
        let audio_codec = self.audio_streams[prog_idx][within_idx].codec;

        // Audio always uses PTS, so PES overhead is 3 (start code) + 5 (PTS) = 8 bytes.
        // The remaining space in the u16 PES_packet_length field is for flags, header_data_length,
        // and the payload. Guard against frames that would overflow PES_packet_length.
        let pes_overhead = 3usize + 5;
        let max_audio = (u16::MAX as usize) - pes_overhead;
        if frames.len() > max_audio {
            return Err(MuxError::AudioTooLarge {
                size: frames.len(),
                max: max_audio,
            });
        }

        let pts = PesPtsField::PtsOnly(Pts90khz(pts_90khz));
        let mut pes_buf = Vec::with_capacity(MAX_PES_HEADER_SIZE + frames.len());
        write_audio_pes(&mut pes_buf, audio_codec, within_idx as u8, pts, frames);

        let total = pes_buf.len();
        let audio_packets = ts_packets_for(total);
        let psi_packets = if self.psi_due(prog_idx, pts_90khz) {
            2
        } else {
            0
        };

        if self.queue.len() + psi_packets + audio_packets > self.config.buffer_packets {
            return Err(MuxError::BufferFull {
                capacity_packets: self.config.buffer_packets,
            });
        }

        self.maybe_emit_psi(prog_idx, pts_90khz);

        let mut cursor = 0;
        let mut first = true;
        while cursor < pes_buf.len() {
            let mut adaptation = AdaptationField::default();
            if first && self.pcr_pids[prog_idx] == audio_pid && self.pcr_due(prog_idx, pts_90khz) {
                let pcr = Pcr27mhz::from_pts(Pts90khz(pts_90khz));
                adaptation.pcr = Some(pcr);
                self.pcr_last[prog_idx] = Some(pcr.0);
            }
            let mut pkt = [0u8; 188];
            let result = write_packet(
                &mut pkt,
                audio_pid,
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
        if let Some(s) = self.per_stream.get_mut(&audio_pid) {
            s.items += 1;
            s.bytes += frames.len() as u64;
        }

        Ok(())
    }

    /// All `AudioStreamHandle`s for this muxer, in `(program, within-program)`
    /// declaration order. One handle per `StreamSpec::Audio` across all programs.
    pub fn audio_handles(&self) -> Vec<AudioStreamHandle> {
        self.audio_streams
            .iter()
            .enumerate()
            .flat_map(|(p_idx, prog)| {
                (0..prog.len()).map(move |s_idx| AudioStreamHandle::pack(p_idx, s_idx))
            })
            .collect()
    }

    /// Audio stream handles for the named program, in declaration order.
    ///
    /// Returns `Err(MuxError::ProgramNotFound)` if no program with the given
    /// number exists.
    pub fn audio_handles_for_program(
        &self,
        program_number: u16,
    ) -> Result<Vec<AudioStreamHandle>, MuxError> {
        let prog_idx = self
            .config
            .programs
            .iter()
            .position(|p| p.program_number == program_number)
            .ok_or(MuxError::ProgramNotFound { program_number })?;
        Ok((0..self.audio_streams[prog_idx].len())
            .map(|s_idx| AudioStreamHandle::pack(prog_idx, s_idx))
            .collect())
    }

    /// Push one subtitle PES unit, single-stream shorthand.
    ///
    /// `pts_90khz` is required and becomes the PES PTS — subtitles are
    /// rendered at presentation time, never reordered. `payload` is one
    /// complete logical subtitle unit (DVB-sub composition page,
    /// teletext data field, CEA-708 service block, or WebVTT cue);
    /// fragmentation across PES is not used.
    ///
    /// Resolves only when exactly one subtitle stream is configured
    /// across all programs. Otherwise rejects with
    /// [`MuxError::AmbiguousTarget`].
    ///
    /// Returns `Err(MuxError::SubtitleTooLarge)` if `payload.len()`
    /// would overflow the PES packet length budget.
    /// Returns `Err(MuxError::BufferFull)` if the resulting TS packets
    /// would exceed `Config::buffer_packets`.
    pub fn push_subtitle(&mut self, pts_90khz: i64, payload: &[u8]) -> Result<(), MuxError> {
        let total_subtitle: usize = self.subtitle_streams.iter().map(|s| s.len()).sum();
        if total_subtitle == 0 {
            return Err(MuxError::NoSubtitleStreamsConfigured);
        }
        if total_subtitle > 1 {
            return Err(MuxError::AmbiguousTarget {
                kind: "subtitle",
                count: total_subtitle,
            });
        }
        // Locate the program with the lone subtitle stream — same iterate-to-find
        // pattern as `push_audio` / `push_klv` (the lone stream may not be in
        // program 0).
        let (prog_idx, _within_idx) = self
            .subtitle_streams
            .iter()
            .enumerate()
            .find(|(_p, s)| !s.is_empty())
            .map(|(p, _)| (p, 0))
            .expect("total_subtitle == 1 guarantees one non-empty program");
        let handle = SubtitleStreamHandle::pack(prog_idx, 0);
        self.push_subtitle_to(handle, pts_90khz, payload)
    }

    /// Push one subtitle PES unit on a specific subtitle stream.
    ///
    /// Routes to the subtitle stream identified by `handle`. Use the
    /// bare [`push_subtitle`][Self::push_subtitle] shorthand when
    /// exactly one subtitle stream is configured. Handles are obtained
    /// from [`subtitle_handles`][Self::subtitle_handles].
    ///
    /// Returns [`MuxError::InvalidStreamHandle`] if the handle's index
    /// is out of range for this muxer's configured subtitle stream count.
    /// Returns [`MuxError::SubtitleTooLarge`] if `payload.len()` would
    /// overflow the PES packet length budget (max 65527 bytes).
    /// Returns `Err(MuxError::BufferFull)` if the resulting TS packets
    /// would exceed `Config::buffer_packets`.
    pub fn push_subtitle_to(
        &mut self,
        handle: SubtitleStreamHandle,
        pts_90khz: i64,
        payload: &[u8],
    ) -> Result<(), MuxError> {
        let (prog_idx, within_idx) = handle.unpack();
        if prog_idx >= self.subtitle_streams.len()
            || within_idx >= self.subtitle_streams[prog_idx].len()
        {
            return Err(MuxError::InvalidStreamHandle {
                kind: "subtitle",
                index: handle.0 as usize,
            });
        }

        // Resolve codec-specific PES envelope shape. DVB-sub auto-wraps the
        // caller's segments in EN 300 743 §6.2's PES_data_field envelope
        // (data_identifier=0x20 + subtitle_stream_id=0x00 + segments + 0xFF
        // marker), adding 3 bytes of overhead. Other codecs pass through.
        let pes_shape = match &self.subtitle_streams[prog_idx][within_idx].codec {
            SubtitleCodec::DvbSubtitling { .. } => SubtitlePesShape::DvbSub,
            SubtitleCodec::DvbTeletext { .. } => SubtitlePesShape::DvbTeletext,
            SubtitleCodec::Cea708Standalone | SubtitleCodec::WebVttInTs => {
                SubtitlePesShape::Passthrough
            }
        };
        let envelope_overhead = match pes_shape {
            SubtitlePesShape::DvbSub => 3, // 0x20 + 0x00 + 0xFF
            // DVB-teletext writes its own 45-byte PES header per EN 300 472 §4.2
            // (rather than reusing the shared 14-byte header path), so it does
            // not contribute envelope bytes — its overhead is folded into
            // `pes_overhead` below.
            SubtitlePesShape::DvbTeletext => 0,
            SubtitlePesShape::Passthrough => 0,
        };

        // PES overhead in u16 PES_packet_length terms:
        // - DVB-teletext: writer emits a 45-byte header (everything before the
        //   caller payload) and pads the PES tail to N×184 bytes. The size cap
        //   is still header(45) + payload <= u16::MAX since PES_packet_length
        //   (which excludes the 6 fixed prefix bytes) holds 45 − 6 + payload =
        //   39 + payload + tail_stuffing.
        // - Other codecs: standard 14-byte header (3 byte prefix + flags(3) +
        //   PTS(5)), so PES_packet_length covers flags(3) + PTS(5) + envelope
        //   + payload.
        let pes_overhead = match pes_shape {
            SubtitlePesShape::DvbTeletext => 45,
            _ => 3usize + 5 + envelope_overhead,
        };
        let max_subtitle = (u16::MAX as usize) - pes_overhead;
        if payload.len() > max_subtitle {
            return Err(MuxError::SubtitleTooLarge {
                size: payload.len(),
                max: max_subtitle,
            });
        }

        let subtitle_pid = self.subtitle_streams[prog_idx][within_idx].pid;

        // Capacity hint: DVB-teletext rounds up to N×184 bytes total, so the
        // tail stuffing can add up to one TS packet's payload area (184 bytes)
        // beyond header + payload. Other codecs only need MAX_PES_HEADER_SIZE
        // + envelope + payload.
        let buf_capacity = match pes_shape {
            SubtitlePesShape::DvbTeletext => 45 + payload.len() + 184,
            _ => MAX_PES_HEADER_SIZE + envelope_overhead + payload.len(),
        };
        let mut pes_buf = Vec::with_capacity(buf_capacity);
        write_subtitle_pes(&mut pes_buf, pts_90khz, pes_shape, payload);

        let subtitle_packets = ts_packets_for(pes_buf.len());
        // Mirror push_audio_to: reserve 2 packets (PAT + 1 PMT) when a PSI
        // tick is due. Multi-program muxers actually emit 1 PAT + N PMTs,
        // but the muxer-wide buffer slop tolerates a small under-reservation
        // here (matches the audio precedent at plan #21 push_audio_to).
        let psi_packets = if self.psi_due(prog_idx, pts_90khz) {
            2
        } else {
            0
        };

        if self.queue.len() + psi_packets + subtitle_packets > self.config.buffer_packets {
            return Err(MuxError::BufferFull {
                capacity_packets: self.config.buffer_packets,
            });
        }

        self.maybe_emit_psi(prog_idx, pts_90khz);

        // Subtitles do NOT extend the PCR fallback chain — they are sparse
        // and event-driven, and the validate path rejects them as PCR PIDs
        // outright (SubtitlePidUsedAsPcrPid). The first packet here will
        // never carry PCR.
        let mut cursor = 0;
        let mut first = true;
        while cursor < pes_buf.len() {
            let adaptation = AdaptationField::default();
            let mut pkt = [0u8; 188];
            let result = write_packet(
                &mut pkt,
                subtitle_pid,
                first,
                adaptation,
                &pes_buf[cursor..],
                &mut self.counters,
            );
            cursor += result.payload_consumed;
            self.queue.push_back(pkt);
            first = false;
        }

        // Per-stream stats — Ok-path only.
        if let Some(s) = self.per_stream.get_mut(&subtitle_pid) {
            s.items += 1;
            s.bytes += payload.len() as u64;
        }

        Ok(())
    }

    /// All `SubtitleStreamHandle`s for this muxer, in
    /// `(program, within-program)` declaration order. One handle per
    /// `StreamSpec::Subtitle` across all programs.
    pub fn subtitle_handles(&self) -> Vec<SubtitleStreamHandle> {
        self.subtitle_streams
            .iter()
            .enumerate()
            .flat_map(|(p_idx, prog)| {
                (0..prog.len()).map(move |s_idx| SubtitleStreamHandle::pack(p_idx, s_idx))
            })
            .collect()
    }

    /// Subtitle stream handles for the named program, in declaration order.
    ///
    /// Returns `Err(MuxError::ProgramNotFound)` if no program with the given
    /// number exists.
    pub fn subtitle_handles_for_program(
        &self,
        program_number: u16,
    ) -> Result<Vec<SubtitleStreamHandle>, MuxError> {
        let prog_idx = self
            .config
            .programs
            .iter()
            .position(|p| p.program_number == program_number)
            .ok_or(MuxError::ProgramNotFound { program_number })?;
        Ok((0..self.subtitle_streams[prog_idx].len())
            .map(|s_idx| SubtitleStreamHandle::pack(prog_idx, s_idx))
            .collect())
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

    /// All `VideoStreamHandle`s for this muxer, in `(program, within-program)`
    /// declaration order. One handle per `StreamSpec::Video` across all programs.
    pub fn video_handles(&self) -> Vec<VideoStreamHandle> {
        self.video_streams
            .iter()
            .enumerate()
            .flat_map(|(p_idx, prog)| {
                (0..prog.len()).map(move |s_idx| VideoStreamHandle::pack(p_idx, s_idx))
            })
            .collect()
    }

    /// All `KlvStreamHandle`s for this muxer, in `(program, within-program)`
    /// declaration order. One handle per `StreamSpec::Klv` across all programs.
    pub fn klv_handles(&self) -> Vec<KlvStreamHandle> {
        self.klv_streams
            .iter()
            .enumerate()
            .flat_map(|(p_idx, prog)| {
                (0..prog.len()).map(move |s_idx| KlvStreamHandle::pack(p_idx, s_idx))
            })
            .collect()
    }

    /// Handle for the i-th video stream in programs[0], or `None` if out of
    /// range. Convenience for single-program callers.
    pub fn video_stream_handle(&self, index: usize) -> Option<VideoStreamHandle> {
        if !self.video_streams.is_empty() && index < self.video_streams[0].len() {
            Some(VideoStreamHandle::pack(0, index))
        } else {
            None
        }
    }

    /// Handle for the i-th KLV stream in programs[0], or `None` if out of
    /// range. Convenience for single-program callers.
    pub fn klv_stream_handle(&self, index: usize) -> Option<KlvStreamHandle> {
        if !self.klv_streams.is_empty() && index < self.klv_streams[0].len() {
            Some(KlvStreamHandle::pack(0, index))
        } else {
            None
        }
    }

    /// Video stream handles for the named program, in declaration order.
    ///
    /// Returns `Err(MuxError::ProgramNotFound)` if no program with the given
    /// number exists.
    pub fn video_handles_for_program(
        &self,
        program_number: u16,
    ) -> Result<Vec<VideoStreamHandle>, MuxError> {
        let prog_idx = self
            .config
            .programs
            .iter()
            .position(|p| p.program_number == program_number)
            .ok_or(MuxError::ProgramNotFound { program_number })?;
        Ok((0..self.video_streams[prog_idx].len())
            .map(|s_idx| VideoStreamHandle::pack(prog_idx, s_idx))
            .collect())
    }

    /// KLV stream handles for the named program, in declaration order.
    ///
    /// Returns `Err(MuxError::ProgramNotFound)` if no program with the given
    /// number exists.
    pub fn klv_handles_for_program(
        &self,
        program_number: u16,
    ) -> Result<Vec<KlvStreamHandle>, MuxError> {
        let prog_idx = self
            .config
            .programs
            .iter()
            .position(|p| p.program_number == program_number)
            .ok_or(MuxError::ProgramNotFound { program_number })?;
        Ok((0..self.klv_streams[prog_idx].len())
            .map(|s_idx| KlvStreamHandle::pack(prog_idx, s_idx))
            .collect())
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
        let (prog_idx, within_idx) = handle.unpack();
        if prog_idx >= self.video_streams.len() || within_idx >= self.video_streams[prog_idx].len()
        {
            // Report the raw packed value as an opaque index so the error message
            // carries the full handle encoding without confusing prog vs within.
            return Err(MuxError::InvalidStreamHandle {
                kind: "video",
                index: handle.0 as usize,
            });
        }
        let video_pid = self.video_streams[prog_idx][within_idx].pid;
        // AV1 carries OBUs (AV1 spec §5), not Annex-B NAL units — its push
        // payload is the OBU bitstream and must skip the Annex-B start-code
        // check that H.264 / H.265 / H.266 require.
        if !matches!(
            self.video_streams[prog_idx][within_idx].codec,
            VideoCodec::Av1
        ) {
            validate_annex_b(nal)?;
        }

        let mut header = [0u8; MAX_PES_HEADER_SIZE];
        // Per AV1-MPEG-2-TS binding §3.4, AV1 PES MUST set
        // data_alignment_indicator=1. H.222.0 §2.4.3.7 leaves the bit
        // codec-defined for H.264 / H.265 / H.266 — keep them unset.
        let pes_flags = PesFlags {
            data_alignment_indicator: matches!(
                self.video_streams[prog_idx][within_idx].codec,
                VideoCodec::Av1
            ),
        };
        let header_len = write_pes_header(
            &mut header,
            STREAM_ID_VIDEO,
            PesPtsField::PtsOnly(Pts90khz(pts_90khz)),
            None,
            pes_flags,
        );

        let total = header_len + nal.len();
        let video_packets = ts_packets_for(total);
        let psi_packets = if self.psi_due(prog_idx, pts_90khz) {
            2
        } else {
            0
        };

        if self.queue.len() + psi_packets + video_packets > self.config.buffer_packets {
            return Err(MuxError::BufferFull {
                capacity_packets: self.config.buffer_packets,
            });
        }

        self.maybe_emit_psi(prog_idx, pts_90khz);

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
                if self.pcr_pids[prog_idx] == video_pid {
                    // Per H.222.0 V9 §2.4.3.5: random_access_indicator may
                    // only be set on PCR_PID packets that also carry the PCR
                    // fields. Force PCR emission when key-frame coincides
                    // with this PID even if pcr_due() would otherwise return
                    // false — matches TSDuck / ffmpeg behavior. Random-access
                    // point + PCR coincide; downstream seekers benefit.
                    if self.pcr_due(prog_idx, pts_90khz) || key_frame {
                        let pcr = Pcr27mhz::from_pts(Pts90khz(pts_90khz));
                        adaptation.pcr = Some(pcr);
                        self.pcr_last[prog_idx] = Some(pcr.0);
                    }
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
    /// For [`KlvStreamType::SynchronousMetadata`] streams, the muxer
    /// auto-prepends a 5-byte `Metadata_AU_cell` header per ITU-T
    /// H.222.0 V9 §2.12.4.2 Tables 2-155+2-156 (see
    /// [`crate::mpegts::au_cell`]). Pass raw KLV LS bytes; do not
    /// pre-wrap. PTS lives in the PES header (per §2.12.4.1).
    /// [`KlvStreamType::PrivateData`] streams pass payload through
    /// unchanged, and `metadata_service_id` is silently ignored on
    /// that path.
    ///
    /// `metadata_service_id` lands in the AU cell header per
    /// ITU-T H.222.0 V9 §2.12.4.2 / ST 1402.2 App. B Table 2.
    /// The spec default is `0x00`. Pass `0x00` unless you have a
    /// specific reason to use a non-zero service_id (e.g. to mirror
    /// the `service_id` byte of a `metadata_klva` PMT descriptor you
    /// supplied at config time).
    ///
    /// Returns [`MuxError::InvalidStreamHandle`] if the handle's index
    /// is out of range.
    pub fn push_klv_to(
        &mut self,
        handle: KlvStreamHandle,
        klv: &[u8],
        pts_90khz: i64,
        metadata_service_id: u8,
    ) -> Result<(), MuxError> {
        let (prog_idx, within_idx) = handle.unpack();
        if prog_idx >= self.klv_streams.len() || within_idx >= self.klv_streams[prog_idx].len() {
            return Err(MuxError::InvalidStreamHandle {
                kind: "klv",
                index: handle.0 as usize,
            });
        }
        let k = &self.klv_streams[prog_idx][within_idx];
        let klv_pid = k.pid;
        let klv_carries_pts = k.carries_pts;
        let is_sync = k.stream_type == KlvStreamType::SynchronousMetadata;
        let seq_num = k.au_cell_sequence_number;

        // Auto-wrap sync KLV in an H.222.0 §2.12.4.2 Metadata_AU_cell header.
        // PrivateData streams pass payload through as-is (caller controls shape).
        let wrapped_storage: Option<Vec<u8>> = if is_sync {
            let header = crate::mpegts::au_cell::AuCellHeader {
                metadata_service_id, // caller-supplied; see push_klv_to doc comment.
                sequence_number: seq_num,
                cell_fragment_indication: crate::mpegts::au_cell::CellFragmentIndication::Complete,
                decoder_config_flag: false,
                random_access_indicator: true, // ST 0601 records are self-contained.
            };
            let mut buf = Vec::with_capacity(5 + klv.len());
            crate::mpegts::au_cell::write_metadata_au_cell(&mut buf, header, klv).map_err(|e| {
                match e {
                    crate::mpegts::au_cell::AuCellError::PayloadTooLarge { size, .. } => {
                        MuxError::KlvTooLarge {
                            size,
                            max: crate::mpegts::au_cell::MAX_AU_CELL_PAYLOAD,
                        }
                    }
                }
            })?;
            Some(buf)
        } else {
            None
        };
        let effective_klv: &[u8] = wrapped_storage.as_deref().unwrap_or(klv);

        let pts_field = if klv_carries_pts {
            PesPtsField::PtsOnly(Pts90khz(pts_90khz))
        } else {
            PesPtsField::None
        };

        let pes_overhead = 3usize + if klv_carries_pts { 5 } else { 0 };
        let max_klv = (u16::MAX as usize) - pes_overhead;
        if effective_klv.len() > max_klv {
            // Report the inner caller payload size in the error, since that's
            // what they control. Subtract 5-byte AU cell header overhead from
            // the cap when sync.
            let report_size = klv.len();
            let report_max = if is_sync { max_klv - 5 } else { max_klv };
            return Err(MuxError::KlvTooLarge {
                size: report_size,
                max: report_max,
            });
        }

        let mut header = [0u8; MAX_PES_HEADER_SIZE];
        // Sync KLV (stream_type 0x15 SynchronousMetadata): stream_id 0xFC per
        // H.222.0 V9 Table 2-22 (reserved for metadata streams).
        // Async KLV (stream_type 0x06 PrivateData): stream_id 0xBD per ffmpeg +
        // GStreamer convention — H.222.0 Table 2-22 reserves 0xFC for metadata
        // streams (stream_type 0x15) only.
        // data_alignment_indicator=1 on both paths: H.222.0 V9 §2.12.4.1
        // mandates it for sync KLV; also conventional for async KLV AU delivery.
        let pes_stream_id = if is_sync {
            STREAM_ID_KLV // 0xFC — H.222.0 metadata stream_id, sync KLV (stream_type 0x15).
        } else {
            STREAM_ID_PRIVATE_STREAM_1 // 0xBD — async KLV (stream_type 0x06).
        };
        let header_len = write_pes_header(
            &mut header,
            pes_stream_id,
            pts_field,
            Some(effective_klv.len() as u16),
            PesFlags {
                data_alignment_indicator: true,
            },
        );

        let total = header_len + effective_klv.len();
        let klv_packets = ts_packets_for(total);
        let psi_packets = if self.psi_due(prog_idx, pts_90khz) {
            2
        } else {
            0
        };

        if self.queue.len() + psi_packets + klv_packets > self.config.buffer_packets {
            return Err(MuxError::BufferFull {
                capacity_packets: self.config.buffer_packets,
            });
        }

        self.maybe_emit_psi(prog_idx, pts_90khz);

        let mut pes_buf = Vec::with_capacity(total);
        pes_buf.extend_from_slice(&header[..header_len]);
        pes_buf.extend_from_slice(effective_klv);

        let mut cursor = 0;
        let mut first = true;
        while cursor < pes_buf.len() {
            let mut adaptation = AdaptationField::default();
            if first && self.pcr_pids[prog_idx] == klv_pid && self.pcr_due(prog_idx, pts_90khz) {
                let pcr = Pcr27mhz::from_pts(Pts90khz(pts_90khz));
                adaptation.pcr = Some(pcr);
                self.pcr_last[prog_idx] = Some(pcr.0);
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

        // Count on the Ok path only — after all early-returns above. Stats
        // count caller's payload bytes, not auto-wrapped bytes.
        if let Some(s) = self.per_stream.get_mut(&klv_pid) {
            s.items += 1;
            s.bytes += klv.len() as u64;
        }
        if is_sync {
            self.klv_streams[prog_idx][within_idx].au_cell_sequence_number =
                seq_num.wrapping_add(1);
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
            programs_configured: self.config.programs.len() as u32,
            subtitle_streams_configured: self.subtitle_streams.iter().map(|s| s.len() as u32).sum(),
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

    /// Return the resolved PCR PID for program at `prog_idx` (0-based index).
    /// Returns `None` if `prog_idx` is out of range.
    #[cfg(test)]
    pub(crate) fn pcr_pid_for_program(&self, prog_idx: usize) -> Option<u16> {
        self.pcr_pids.get(prog_idx).copied()
    }

    fn psi_due(&self, prog_idx: usize, pts_90khz: i64) -> bool {
        match self.psi_last[prog_idx] {
            None => true,
            Some(last_masked) => {
                let now_masked = Pts90khz(pts_90khz).masked_33bit();
                let delta = crate::mpegts::common::pts_diff_33bit(now_masked, last_masked);
                delta >= self.psi_interval_90khz
            }
        }
    }

    fn pcr_due(&self, prog_idx: usize, pts_90khz: i64) -> bool {
        match self.pcr_last[prog_idx] {
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

    fn maybe_emit_psi(&mut self, prog_idx: usize, pts_90khz: i64) {
        if !self.psi_due(prog_idx, pts_90khz) {
            return;
        }
        // Emit one PAT that lists all programs, then one PMT per program.
        // The PAT is emitted on every PSI tick regardless of which program
        // triggered the tick, so all programs' state is always visible to
        // receivers after a single PSI interval.
        let mut pat = [0u8; 188];
        write_pat_packet(&mut pat, &self.config, &mut self.counters);
        self.queue.push_back(pat);

        // One PMT per program — iterate the full program set so every program
        // gets a fresh PMT on the tick (not just the triggering program).
        for pidx in 0..self.config.programs.len() {
            let prog = &self.config.programs[pidx];
            let mut entries: Vec<PmtStreamEntry> = Vec::with_capacity(prog.streams.len());
            for (i, spec) in prog.streams.iter().enumerate() {
                let stream_type = match spec {
                    StreamSpec::Video {
                        codec: VideoCodec::H264,
                        ..
                    } => StreamType::H264,
                    StreamSpec::Video {
                        codec: VideoCodec::H265,
                        ..
                    } => StreamType::H265,
                    StreamSpec::Video {
                        codec: VideoCodec::H266,
                        ..
                    } => StreamType::H266,
                    // AV1 rides PMT stream_type 0x06; the AV01
                    // registration_descriptor (auto-emitted at the top of the
                    // PMT descriptor loop, suppressed when caller supplies
                    // their own) disambiguates the codec at the wire level.
                    StreamSpec::Video {
                        codec: VideoCodec::Av1,
                        ..
                    } => StreamType::KlvPrivate,
                    StreamSpec::Klv {
                        stream_type: KlvStreamType::PrivateData,
                        ..
                    } => StreamType::KlvPrivate,
                    StreamSpec::Klv {
                        stream_type: KlvStreamType::SynchronousMetadata,
                        ..
                    } => StreamType::KlvSyncMetadata,
                    StreamSpec::Audio {
                        codec: AudioCodec::Mp2,
                        ..
                    } => StreamType::AudioMp2,
                    StreamSpec::Audio {
                        codec: AudioCodec::Aac,
                        ..
                    } => StreamType::AudioAac,
                    StreamSpec::Audio {
                        codec: AudioCodec::AacLatm,
                        ..
                    } => StreamType::AudioAacLatm,
                    StreamSpec::Audio {
                        codec: AudioCodec::Ac3,
                        ..
                    } => StreamType::AudioAc3,
                    // All four subtitle codecs share PMT stream_type 0x06
                    // (PrivateData); the per-stream descriptor cache carries
                    // the codec-specific disambiguator (subtitling_descriptor /
                    // teletext_descriptor / Registration "GA94" / "VTTC").
                    StreamSpec::Subtitle { .. } => StreamType::KlvPrivate,
                };
                entries.push(PmtStreamEntry {
                    stream_type,
                    elementary_pid: spec.pid(),
                    descriptors: &self.pmt_descriptor_caches[pidx][i],
                });
            }

            let mut pmt = [0u8; 188];
            write_pmt_packet(
                &mut pmt,
                prog,
                self.pcr_pids[pidx],
                &entries,
                &mut self.counters,
            )
            .expect("validated Config must produce single-section PMT");
            self.queue.push_back(pmt);
        }

        self.psi_last[prog_idx] = Some(Pts90khz(pts_90khz).masked_33bit());
    }
}

fn validate_annex_b(nal: &[u8]) -> Result<(), MuxError> {
    if nal.starts_with(&[0x00, 0x00, 0x00, 0x01]) || nal.starts_with(&[0x00, 0x00, 0x01]) {
        Ok(())
    } else {
        Err(MuxError::InvalidNal)
    }
}

/// ISO 639-2 language codes per ETSI EN 300 468 §6.2.41/§6.2.43 ride the
/// wire as 3 ISO/IEC 8859-1 bytes. Spec doesn't mandate lowercase; we
/// accept uppercase or lowercase ASCII letters but reject non-alphabetic
/// bytes (digits, symbols, control codes) to keep junk out.
fn validate_language_code(code: [u8; 3]) -> Result<(), MuxError> {
    if code.iter().all(|&b| b.is_ascii_alphabetic()) {
        Ok(())
    } else {
        Err(MuxError::InvalidLanguageCode { code })
    }
}

/// True iff `caller_descs` contains any descriptor that the receiver-side
/// subtitle classifier recognizes as a codec marker. Mirrors the demux-side
/// `mpegts::demux::demuxer::has_recognized_subtitle_descriptor` predicate
/// but operates on raw TLV bytes (the form held in `prog.stream_descriptors`)
/// rather than on parsed `RawDescriptor`.
///
/// Used to suppress the subtitle auto-emit when the caller has already
/// supplied one of:
///   * `subtitling_descriptor`   (tag 0x59)
///   * `teletext_descriptor`     (tag 0x56)
///   * `VBI_teletext_descriptor` (tag 0x46)
///   * `registration_descriptor` (tag 0x05) with format_identifier
///     `"VTTC"` or `"GA94"`
///
/// Mirrors the KLV/AV1 caller-supplied-Registration suppression rule so
/// receivers don't see two competing codec markers on the same PID.
fn caller_has_recognized_subtitle_descriptor(caller_descs: &[Vec<u8>]) -> bool {
    for tlv in caller_descs {
        if tlv.is_empty() {
            continue;
        }
        let tag = tlv[0];
        if tag == 0x59 || tag == 0x56 || tag == 0x46 {
            return true;
        }
        // registration_descriptor TLV layout: tag(1) + length(1) + body(length).
        // format_identifier is the first 4 body bytes.
        if tag == 0x05 && tlv.len() >= 6 {
            let fid = &tlv[2..6];
            if fid == b"VTTC" || fid == b"GA94" {
                return true;
            }
        }
    }
    false
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
    fn audio_codec_real_variants() {
        let codecs = [
            AudioCodec::Mp2,
            AudioCodec::Aac,
            AudioCodec::AacLatm,
            AudioCodec::Ac3,
        ];
        // Trivially constructible; equality holds.
        assert_ne!(codecs[0], codecs[1]);
    }

    #[test]
    fn stream_spec_audio_variant() {
        let spec = StreamSpec::Audio {
            pid: 0x300,
            codec: AudioCodec::Aac,
            language: None,
        };
        assert_eq!(spec.pid(), 0x300);
    }

    #[test]
    fn audio_stream_handle_pack_unpack_round_trip() {
        let h = AudioStreamHandle::pack(2, 5);
        assert_eq!(h.unpack(), (2, 5));
    }

    #[test]
    fn audio_stream_handle_from_raw() {
        let h = AudioStreamHandle::pack(3, 7);
        let raw: u32 = unsafe { std::mem::transmute_copy(&h) };
        let h2 = AudioStreamHandle::from_raw(raw);
        assert_eq!(h, h2);
    }

    #[test]
    fn subtitle_codec_real_variants() {
        let dvb_sub = SubtitleCodec::DvbSubtitling {
            language: *b"eng",
            subtitling_type: 0x10,
            composition_page_id: 1,
            ancillary_page_id: 1,
        };
        let dvb_tt = SubtitleCodec::DvbTeletext {
            language: *b"eng",
            teletext_type: 0x02,
            magazine_number: 1,
            page_number: 0x88,
        };
        let cea = SubtitleCodec::Cea708Standalone;
        let vtt = SubtitleCodec::WebVttInTs;
        assert_ne!(dvb_sub, dvb_tt);
        assert_ne!(cea, vtt);
    }

    #[test]
    fn stream_spec_subtitle_variant() {
        let spec = StreamSpec::Subtitle {
            pid: 0x400,
            codec: SubtitleCodec::WebVttInTs,
        };
        assert_eq!(spec.pid(), 0x400);
    }

    #[test]
    fn subtitle_stream_handle_pack_unpack_round_trip() {
        let h = SubtitleStreamHandle::pack(2, 5);
        assert_eq!(h.unpack(), (2, 5));
    }

    #[test]
    fn subtitle_stream_handle_from_raw() {
        let h = SubtitleStreamHandle::pack(3, 7);
        let raw: u32 = h.raw();
        let h2 = SubtitleStreamHandle::from_raw(raw);
        assert_eq!(h, h2);
    }

    #[test]
    fn rejects_video_pid_zero() {
        let mut cfg = Config::default();
        if let Some(StreamSpec::Video { pid, .. }) = cfg.programs[0]
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
        if let Some(StreamSpec::Klv { pid, .. }) = cfg.programs[0]
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
            .add_program(1, 0x1000)
            .add_video(0x1234, VideoCodec::H264)
            .add_klv(0x1234, KlvStreamType::PrivateData, false)
            .end_program()
            .build();
        assert!(matches!(
            cfg,
            Err(MuxError::InvalidConfig(
                "stream PIDs within a program must be distinct"
            ))
        ));
    }

    #[test]
    fn rejects_unrelated_pcr_pid() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .pcr_pid(0x0500)
            .end_program()
            .build();
        assert!(matches!(
            cfg,
            Err(MuxError::InvalidConfig(
                "pcr_pid must equal a configured stream PID in the same program"
            ))
        ));
    }

    #[test]
    fn rejects_pcr_pid_pinned_to_klv() {
        // KLV cadence (1-10 Hz) violates ETSI TR 101 290 §5.6.1's 100 ms
        // ceiling. Validate now rejects this combination.
        let err = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .pcr_pid(0x1031)
            .end_program()
            .build()
            .unwrap_err();
        assert!(
            matches!(err, MuxError::KlvPidUsedAsPcrPid { pid: 0x1031 }),
            "expected KlvPidUsedAsPcrPid {{ pid: 0x1031 }}, got {err:?}"
        );
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
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::SynchronousMetadata, false)
            .end_program()
            .build();
        assert!(cfg.is_err());
    }

    #[test]
    fn accepts_async_with_pts_combo() {
        // 0x06 + PTS — the common-practice "sync KLV everyone recognizes"
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, true)
            .end_program()
            .build();
        cfg.expect("0x06 + PTS is valid");
    }

    #[test]
    fn resolved_pcr_pid_default() {
        // Muxer auto-resolves PCR to the first video stream (0x1011) when
        // pcr_pid is None. Verify via the constructed Muxer's state.
        let mux = Muxer::new(Config::default()).unwrap();
        assert_eq!(mux.pcr_pids[0], 0x1011);
    }

    #[test]
    fn resolved_pcr_pid_explicit() {
        // Explicit pcr_pid on video PID — muxer's pcr_pids[] must reflect it.
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .pcr_pid(0x1011)
            .end_program()
            .build()
            .unwrap();
        let mux = Muxer::new(cfg).unwrap();
        assert_eq!(mux.pcr_pids[0], 0x1011);
    }

    #[test]
    fn muxer_constructs_with_valid_config() {
        let mux = Muxer::new(Config::default());
        assert!(mux.is_ok());
    }

    #[test]
    fn muxer_rejects_invalid_config() {
        let mut cfg = Config::default();
        if let Some(StreamSpec::Video { pid, .. }) = cfg.programs[0]
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
        let err = mux.push_klv(&too_big, 0, 0x00).unwrap_err();
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
        mux.push_klv(&max_klv, 0, 0x00)
            .expect("max-size KLV must succeed");
    }

    #[test]
    fn push_klv_with_pts_reduces_max() {
        // With klv_carries_pts=true, header_data_length=5, so max payload =
        // 65535 - 3 - 5 = 65527.
        let mut mux = Muxer::new(
            Config::builder()
                .add_program(1, 0x1000)
                .add_video(0x1011, VideoCodec::H264)
                .add_klv(0x1031, KlvStreamType::PrivateData, true)
                .end_program()
                .build()
                .unwrap(),
        )
        .unwrap();
        let too_big = vec![0u8; 65_528];
        let err = mux.push_klv(&too_big, 90_000, 0x00).unwrap_err();
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
        let mut cfg = Config::default();
        cfg.programs[0].streams.clear();
        let err = cfg.validate().unwrap_err();
        assert!(
            matches!(err, MuxError::EmptyProgram { program_number: 1 }),
            "expected EmptyProgram, got {err:?}",
        );
    }

    #[test]
    fn config_rejects_duplicate_pids() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1011, KlvStreamType::PrivateData, false)
            .end_program()
            .build();
        let err = cfg.unwrap_err();
        assert!(matches!(err, MuxError::InvalidConfig(msg) if msg.contains("distinct")));
    }

    #[test]
    fn config_pcr_pid_must_match_stream() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .pcr_pid(0x1099) // not configured
            .end_program()
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
        mux.push_klv_to(h, &klv, 0, 0x00).unwrap();
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
    fn klv_pes_sets_data_alignment_indicator_per_h2220_v9_2_12_4_1() {
        // SynchronousMetadata KLV stream (stream_type 0x15) — H.222.0 V9
        // §2.12.4.1 mandates data_alignment_indicator=1 on every metadata PES.
        // Video stream provides PCR; KLV-only programs are rejected at validate.
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::SynchronousMetadata, true)
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();

        // Minimal raw KLV LS — 16-byte ST 0601 UL + 1-byte BER length=0.
        // Muxer auto-prepends the 5-byte H.222.0 §2.12.4.2 AU cell header for
        // SynchronousMetadata streams (Plan #25); irrelevant to the PES flag bit.
        let raw_klv: Vec<u8> = vec![
            0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00,
            0x00, 0x00, 0x00,
        ];
        let h = mux.klv_stream_handle(0).unwrap();
        mux.push_klv_to(h, &raw_klv, 45_000, 0x00).unwrap();

        let mut buf = vec![0u8; 188 * 64];
        let n = mux.pull(&mut buf);
        assert!(n > 0);

        // Find the first TS packet on KLV PID 0x101 with PUSI=1.
        let pes_flags1 = buf[..n]
            .chunks_exact(188)
            .find_map(|p| {
                let pkt = crate::mpegts::demux::ts::parse_ts_packet(p).ok()?;
                if pkt.pid == 0x101 && pkt.payload_unit_start {
                    // Skip 6-byte fixed PES prefix (start_code(3) + stream_id(1)
                    // + length(2)); byte 6 is flags1.
                    Some(pkt.payload[6])
                } else {
                    None
                }
            })
            .expect("KLV PES start packet present");

        assert_eq!(
            (pes_flags1 >> 2) & 0b1,
            0b1,
            "KLV PES MUST set data_alignment_indicator=1 per H.222.0 V9 §2.12.4.1; got flags1={pes_flags1:#04x}",
        );
    }

    #[test]
    fn av1_pes_sets_data_alignment_indicator_per_av1_binding_3_4() {
        // AV1-MPEG-2-TS binding §3.4 mandates data_alignment_indicator=1
        // on every AV1 PES. ffmpeg has no AV1-in-MPEG-TS muxer, so this
        // can't be cross-validated against ffmpeg output — but the
        // binding normative is explicit and tsduck-tsp expects the bit.
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, VideoCodec::Av1)
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();

        // Minimal AV1 OBU payload — Temporal Delimiter (obu_type=2, empty
        // body) + Sequence Header (obu_type=1, placeholder body). Each
        // OBU has obu_has_size_field=1 per AV1-in-MPEG-2-TS §3.1. Header
        // byte = (obu_type << 3) | 0b010. The exact bytes don't matter
        // for this test — we're checking the PES flags1 byte, not the
        // payload.
        let obu: Vec<u8> = vec![
            // Temporal Delimiter: header=0x12, size=0
            0x12, 0x00, // Sequence Header: header=0x0A, size=2, body=0x00 0x00
            0x0A, 0x02, 0x00, 0x00,
        ];
        let h = mux.video_stream_handle(0).unwrap();
        mux.push_video_to(h, &obu, 45_000, true)
            .expect("push_video_to");

        let mut buf = vec![0u8; 188 * 64];
        let n = mux.pull(&mut buf);
        assert!(n > 0);

        let pes_flags1 = buf[..n]
            .chunks_exact(188)
            .find_map(|p| {
                let pkt = crate::mpegts::demux::ts::parse_ts_packet(p).ok()?;
                if pkt.pid == 0x101 && pkt.payload_unit_start {
                    Some(pkt.payload[6])
                } else {
                    None
                }
            })
            .expect("AV1 PES start packet present");

        assert_eq!(
            (pes_flags1 >> 2) & 0b1,
            0b1,
            "AV1 PES MUST set data_alignment_indicator=1 per AV1-MPEG-2-TS binding §3.4; got flags1={pes_flags1:#04x}",
        );
    }

    #[test]
    fn h264_pes_does_not_set_data_alignment_indicator() {
        // H.222.0 §2.4.3.7 leaves data_alignment_indicator codec-defined
        // for H.264 / H.265 / H.266 video — we conservatively keep it
        // unset.
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, VideoCodec::H264)
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();

        // Minimal H.264 access unit — Annex-B start code + IDR slice NAL
        // header. Body bytes are placeholder; only the PES flags1 byte
        // is under test.
        let nalu: Vec<u8> = vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x88];
        let h = mux.video_stream_handle(0).unwrap();
        mux.push_video_to(h, &nalu, 45_000, true)
            .expect("push_video_to");

        let mut buf = vec![0u8; 188 * 64];
        let n = mux.pull(&mut buf);
        assert!(n > 0);

        let pes_flags1 = buf[..n]
            .chunks_exact(188)
            .find_map(|p| {
                let pkt = crate::mpegts::demux::ts::parse_ts_packet(p).ok()?;
                if pkt.pid == 0x101 && pkt.payload_unit_start {
                    Some(pkt.payload[6])
                } else {
                    None
                }
            })
            .expect("H.264 PES start packet present");

        assert_eq!(
            (pes_flags1 >> 2) & 0b1,
            0,
            "H.264 PES should NOT set data_alignment_indicator; got flags1={pes_flags1:#04x}",
        );
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
        let err = mux.push_klv_to(bogus, &[0; 16], 0, 0x00).unwrap_err();
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
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264) // EO
            .add_video(0x1021, VideoCodec::H264) // IR
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .end_program()
            .build();
        assert!(cfg.is_ok(), "dual-video + KLV must validate");
    }

    #[test]
    fn config_validate_accepts_dual_klv_plus_video() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .add_klv(0x1041, KlvStreamType::PrivateData, true)
            .end_program()
            .build();
        assert!(cfg.is_ok(), "video + dual-KLV must validate");
    }

    #[test]
    fn config_validate_rejects_seventeen_video_streams() {
        let mut pb = Config::builder().add_program(1, 0x1000);
        for i in 0..17u16 {
            pb = pb.add_video(0x1010 + i, VideoCodec::H264);
        }
        pb = pb.add_klv(0x1100, KlvStreamType::PrivateData, false);
        let err = pb.end_program().build().unwrap_err();
        assert!(
            matches!(err, MuxError::TooManyVideoStreams { count: 17, cap: 16 }),
            "expected TooManyVideoStreams {{ 17, 16 }}, got {err:?}",
        );
    }

    #[test]
    fn config_validate_rejects_seventeen_klv_streams() {
        let mut pb = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264);
        for i in 0..17u16 {
            pb = pb.add_klv(0x1100 + i, KlvStreamType::PrivateData, false);
        }
        let err = pb.end_program().build().unwrap_err();
        assert!(
            matches!(err, MuxError::TooManyKlvStreams { count: 17, cap: 16 }),
            "expected TooManyKlvStreams {{ 17, 16 }}, got {err:?}",
        );
    }

    #[test]
    fn muxer_new_accepts_video_only() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .end_program()
            .build()
            .unwrap();
        let mux = Muxer::new(cfg);
        assert!(mux.is_ok(), "video-only muxer must construct");
    }

    #[test]
    fn muxer_new_accepts_video_plus_klv() {
        // KLV-only configs are rejected (KLV cadence too sparse for PCR).
        // Video + KLV with PCR auto-resolved to video is the correct shape.
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .end_program()
            .build()
            .unwrap();
        let mux = Muxer::new(cfg);
        assert!(mux.is_ok(), "video + klv muxer must construct");
    }

    #[test]
    fn push_video_rejects_when_multiple_video_streams_configured() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_video(0x1021, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .end_program()
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
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .add_klv(0x1041, KlvStreamType::PrivateData, true)
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();
        let err = mux.push_klv(&[0; 16], 0, 0x00).unwrap_err();
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
        // Audio-only muxer (valid config; PCR resolves to audio) — push_video
        // has no possible target and must return AmbiguousTarget { count: 0 }.
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_audio(0x1041, AudioCodec::Aac)
            .end_program()
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
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();
        let err = mux.push_klv(&[0; 16], 0, 0x00).unwrap_err();
        assert!(
            matches!(err, MuxError::NoKlvStreamsConfigured),
            "expected NoKlvStreamsConfigured, got {err:?}",
        );
    }

    #[test]
    fn push_subtitle_without_streams_returns_no_streams_configured() {
        // Single video, no subtitles configured; push_subtitle shorthand must
        // surface NoSubtitleStreamsConfigured (was misleading AmbiguousTarget{count:0}).
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();
        let err = mux.push_subtitle(0, &[]).unwrap_err();
        assert!(
            matches!(err, MuxError::NoSubtitleStreamsConfigured),
            "expected NoSubtitleStreamsConfigured, got {err:?}",
        );
    }

    #[test]
    fn push_audio_without_streams_returns_no_streams_configured() {
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();
        let err = mux.push_audio(&[], 0).unwrap_err();
        assert!(
            matches!(err, MuxError::NoAudioStreamsConfigured),
            "expected NoAudioStreamsConfigured, got {err:?}",
        );
    }

    #[test]
    fn push_klv_without_streams_returns_no_streams_configured() {
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();
        let err = mux.push_klv(&[], 0, 0x00).unwrap_err();
        assert!(
            matches!(err, MuxError::NoKlvStreamsConfigured),
            "expected NoKlvStreamsConfigured, got {err:?}",
        );
    }

    #[test]
    fn default_config_has_empty_per_stream_descriptors() {
        let cfg = Config::default();
        let prog = &cfg.programs[0];
        assert_eq!(prog.stream_descriptors.len(), prog.streams.len());
        for descs in &prog.stream_descriptors {
            assert!(descs.is_empty());
        }
    }

    #[test]
    fn validate_rejects_descriptor_count_mismatch() {
        let mut cfg = Config::default();
        // streams has 2, overwrite with 1-entry descriptor vec
        cfg.programs[0].stream_descriptors = vec![Vec::new()];
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, MuxError::InvalidConfig(_)));
    }

    #[test]
    fn validate_rejects_malformed_descriptor() {
        // Length byte claims 5 bytes of body but only 1 follows.
        let bad = vec![0xFF, 0x05, 0x00];
        let mut cfg = Config::default();
        cfg.programs[0].stream_descriptors = vec![vec![bad], Vec::new()];
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
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_video(0x101, VideoCodec::H264)
            .add_klv(0x102, KlvStreamType::PrivateData, false)
            .add_klv(0x103, KlvStreamType::PrivateData, false)
            .stream_descriptors_for_stream(0, vec![big.clone()])
            .stream_descriptors_for_stream(1, vec![big.clone()])
            .stream_descriptors_for_stream(2, vec![big.clone()])
            .stream_descriptors_for_stream(3, vec![big])
            .end_program()
            .build();
        assert!(matches!(cfg, Err(MuxError::PmtTooLarge { .. })));
    }

    #[test]
    fn builder_routes_video_descriptors_by_video_index() {
        // 2 video, 1 KLV. Setting video_index=1 should land on absolute index 2
        // (streams: [video0, klv, video1]).
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x102, KlvStreamType::PrivateData, false)
            .add_video(0x101, VideoCodec::H264)
            .stream_descriptors_for_video(1, vec![crate::mpegts::descriptors::user_private(b"V2")])
            .end_program()
            .build()
            .unwrap();
        let prog = &cfg.programs[0];
        assert_eq!(prog.stream_descriptors[0], Vec::<Vec<u8>>::new());
        assert_eq!(prog.stream_descriptors[1], Vec::<Vec<u8>>::new());
        assert_eq!(prog.stream_descriptors[2].len(), 1);
        assert_eq!(prog.stream_descriptors[2][0][0], 0xFF);
    }

    #[test]
    fn builder_rejects_out_of_range_video_index() {
        // With the new ProgramBuilder, out-of-range video_idx panics (not Err).
        // Use std::panic::catch_unwind to assert the panic fires.
        let result = std::panic::catch_unwind(|| {
            Config::builder()
                .add_program(1, 0x1000)
                .add_video(0x100, VideoCodec::H264)
                .stream_descriptors_for_video(
                    7,
                    vec![crate::mpegts::descriptors::user_private(b"X")],
                )
                .end_program()
                .build()
                .unwrap()
        });
        assert!(result.is_err(), "expected panic for out-of-range video_idx");
    }

    #[test]
    fn cache_composes_auto_emit_then_caller_bytes_on_klv_private() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .stream_descriptors_for_klv(
                0,
                vec![crate::mpegts::descriptors::user_private(b"KLV_LBL")],
            )
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();

        // Stream 0 (video) — no auto-emit, no caller — empty cache entry.
        assert!(muxer.pmt_descriptor_caches[0][0].is_empty());

        // Stream 1 (KLV PrivateData) — KLVA Registration (6 bytes) +
        // user_private("KLV_LBL") (9 bytes) = 15 bytes.
        let entry = &muxer.pmt_descriptor_caches[0][1];
        assert_eq!(entry.len(), 15);
        assert_eq!(&entry[..6], &[0x05, 0x04, b'K', b'L', b'V', b'A']);
        assert_eq!(entry[6], 0xFF); // user_private tag
        assert_eq!(entry[7], 7); // body length
        assert_eq!(&entry[8..], b"KLV_LBL");
    }

    #[test]
    fn cache_suppresses_klva_auto_emit_when_caller_supplies_registration() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .stream_descriptors_for_klv(
                0,
                vec![crate::mpegts::descriptors::registration(*b"KLVA", &[])],
            )
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();

        // Cache index 0 = video (empty), index 1 = KLV.
        // Caller's Registration only — auto-emit suppressed. Total = 6 bytes.
        assert_eq!(muxer.pmt_descriptor_caches[0][1].len(), 6);
    }

    #[test]
    fn cache_auto_emits_klva_on_sync_klv() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::SynchronousMetadata, true)
            .stream_descriptors_for_klv(
                0,
                vec![
                    crate::mpegts::descriptors::metadata_klva(0x00),
                    crate::mpegts::descriptors::metadata_std(0, 0, 0),
                ],
            )
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();
        // Cache index 0 = video (empty), index 1 = KLV.
        // KLVA auto-emit (6 bytes) prepended on SynchronousMetadata too.
        // 6 (KLVA) + 11 (0x26) + 11 (0x27) = 28 bytes.
        assert_eq!(muxer.pmt_descriptor_caches[0][1].len(), 28);
        assert_eq!(muxer.pmt_descriptor_caches[0][1][0], 0x05); // KLVA Registration
        assert_eq!(&muxer.pmt_descriptor_caches[0][1][2..6], b"KLVA");
        assert_eq!(muxer.pmt_descriptor_caches[0][1][6], 0x26);
        assert_eq!(muxer.pmt_descriptor_caches[0][1][17], 0x27);
    }

    // ── Task 9: subtitle PMT descriptor auto-emit ────────────────────────

    #[test]
    fn pmt_emits_subtitle_entry_with_subtitling_descriptor() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(
                0x200,
                SubtitleCodec::DvbSubtitling {
                    language: *b"eng",
                    subtitling_type: 0x10,
                    composition_page_id: 0x0001,
                    ancillary_page_id: 0x0001,
                },
            )
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();

        // Stream 0 (video) — no auto-emit, no caller — empty cache entry.
        assert!(muxer.pmt_descriptor_caches[0][0].is_empty());

        // Stream 1 (subtitle) — subtitling_descriptor: tag 0x59, len 0x08,
        // 3 bytes language + 1 type + 2 composition_page_id + 2 ancillary_page_id
        // = 10 bytes total.
        let entry = &muxer.pmt_descriptor_caches[0][1];
        assert_eq!(entry.len(), 10);
        assert_eq!(entry[0], 0x59); // subtitling_descriptor tag
        assert_eq!(entry[1], 0x08); // length
        assert_eq!(&entry[2..5], b"eng");
        assert_eq!(entry[5], 0x10);
        assert_eq!(&entry[6..8], &[0x00, 0x01]);
        assert_eq!(&entry[8..10], &[0x00, 0x01]);
    }

    #[test]
    fn pmt_emits_subtitle_entry_with_vttc_registration_for_webvtt() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();

        // WebVttInTs auto-emit: registration_descriptor tag 0x05, len 0x04,
        // format_identifier == "VTTC" — 6 bytes total.
        let entry = &muxer.pmt_descriptor_caches[0][1];
        assert_eq!(entry.len(), 6);
        assert_eq!(entry[0], 0x05); // registration_descriptor tag
        assert_eq!(entry[1], 0x04); // length
        assert_eq!(&entry[2..6], b"VTTC");
    }

    #[test]
    fn pmt_emits_subtitle_entry_with_ga94_registration_for_cea708_standalone() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(0x200, SubtitleCodec::Cea708Standalone)
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();

        // Cea708Standalone auto-emit: registration_descriptor "GA94".
        let entry = &muxer.pmt_descriptor_caches[0][1];
        assert_eq!(entry.len(), 6);
        assert_eq!(entry[0], 0x05);
        assert_eq!(entry[1], 0x04);
        assert_eq!(&entry[2..6], b"GA94");
    }

    #[test]
    fn pmt_emits_subtitle_entry_with_teletext_descriptor() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(
                0x200,
                SubtitleCodec::DvbTeletext {
                    language: *b"eng",
                    teletext_type: 0x02,
                    magazine_number: 1,
                    page_number: 0x88,
                },
            )
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();

        // teletext_descriptor: tag 0x56, len 0x05 — 7 bytes total.
        let entry = &muxer.pmt_descriptor_caches[0][1];
        assert_eq!(entry.len(), 7);
        assert_eq!(entry[0], 0x56);
        assert_eq!(entry[1], 0x05);
        assert_eq!(&entry[2..5], b"eng");
        // teletext_type (5 bits) << 3 | magazine_number (3 bits) = 0x02<<3 | 1 = 0x11
        assert_eq!(entry[5], (0x02 << 3) | 0x01);
        assert_eq!(entry[6], 0x88);
    }

    #[test]
    fn pmt_appends_caller_supplied_descriptors_after_auto_emit() {
        // Caller-supplied stream_identifier_descriptor (tag 0x52, len 0x01,
        // component_tag 0x42 — 3 bytes) appends AFTER the VTTC auto-emit.
        // The stream_identifier_descriptor is not a recognized subtitle codec
        // marker, so it does not suppress the auto-emit; the auto-emit fires
        // and the caller's bytes append afterwards. (Caller-supplied codec
        // markers — subtitling/teletext/VBI-teletext/VTTC/GA94 — do suppress
        // the auto-emit; see the `subtitle_auto_emit_suppressed_*` tests.)
        let extra: Vec<Vec<u8>> = vec![vec![0x52u8, 0x01, 0x42]];
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
            .stream_descriptors_for_subtitle(0, extra)
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();

        // VTTC auto-emit (6 bytes) + stream_identifier (3 bytes) = 9 bytes.
        let entry = &muxer.pmt_descriptor_caches[0][1];
        assert_eq!(entry.len(), 9);
        // Auto-emit first.
        assert_eq!(&entry[..6], &[0x05, 0x04, b'V', b'T', b'T', b'C']);
        // Caller's stream_identifier_descriptor after.
        assert_eq!(&entry[6..9], &[0x52, 0x01, 0x42]);
    }

    // ── Task 18: AV1 PMT descriptor auto-emit ────────────────────────────

    #[test]
    fn pmt_emits_av1_with_av01_registration_first() {
        // VideoCodec::Av1 must auto-emit the AV01 registration_descriptor as
        // the FIRST descriptor in the per-stream PMT loop (binding §2.1).
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, VideoCodec::Av1)
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();
        let entry = &muxer.pmt_descriptor_caches[0][0];
        // AV01 Registration: 0x05 0x04 'A' 'V' '0' '1' = 6 bytes.
        assert!(
            entry.len() >= 6,
            "expected AV01 auto-emit (≥6 bytes), got {}",
            entry.len()
        );
        assert_eq!(&entry[..6], &[0x05, 0x04, b'A', b'V', b'0', b'1']);
    }

    #[test]
    fn pmt_av1_with_caller_supplied_av01_suppresses_auto_emit() {
        // When caller has already supplied an AV01 Registration, suppress the
        // auto-emit — mirrors KLVA suppression. Result is exactly the caller's
        // bytes.
        let custom_av01 = vec![0x05, 0x04, b'A', b'V', b'0', b'1'];
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, VideoCodec::Av1)
            .stream_descriptors_for_video(0, vec![custom_av01.clone()])
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();
        let entry = &muxer.pmt_descriptor_caches[0][0];
        assert_eq!(
            entry.len(),
            6,
            "auto-emit should suppress when caller provides AV01"
        );
        assert_eq!(&entry[..], &custom_av01[..]);
    }

    // ── Task 7: add_audio + audio cap + audio-only program tests ─────────

    #[test]
    fn config_validate_rejects_too_many_audio_streams() {
        let mut builder = Config::builder().add_program(1, 0x1000);
        for i in 0..17 {
            builder = builder.add_audio(0x300 + i as u16, AudioCodec::Aac);
        }
        let err = builder.end_program().build().unwrap_err();
        assert!(
            matches!(err, MuxError::TooManyAudioStreams { count: 17, cap: 16 }),
            "expected TooManyAudioStreams {{ 17, 16 }}, got {err:?}",
        );
    }

    #[test]
    fn pcr_falls_back_to_first_audio_pid_for_audio_only_program() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_audio(0x300, AudioCodec::Aac)
            .add_audio(0x301, AudioCodec::Mp2)
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();
        // First audio PID = 0x300, no video, no KLV → 0x300 wins PCR.
        assert_eq!(muxer.pcr_pid_for_program(0).unwrap(), 0x300);
    }

    #[test]
    fn push_audio_to_writes_pes_with_pts_only_header() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_audio(0x300, AudioCodec::Aac)
            .end_program()
            .build()
            .unwrap();
        let mut muxer = Muxer::new(cfg).unwrap();
        let handles = muxer.audio_handles();
        assert_eq!(handles.len(), 1);

        // Push 100 bytes of synthetic audio data with PTS = 90000 (1 second).
        let frames: Vec<u8> = (0..100).map(|i| i as u8).collect();
        muxer.push_audio_to(handles[0], 90_000, &frames).unwrap();

        // Pull the resulting TS bytes; locate the PES start packet for PID 0x300.
        let mut buf = vec![0u8; 188 * 64];
        let n = muxer.pull(&mut buf);
        assert!(n > 0);

        // Find the audio PES packet — first TS packet for PID 0x300 with PUSI=1.
        let packet = buf[..n]
            .chunks_exact(188)
            .find(|p| {
                p[0] == 0x47
                    && (((p[1] as u16 & 0x1F) << 8) | (p[2] as u16)) == 0x300
                    && (p[1] & 0x40) != 0 // payload_unit_start_indicator
            })
            .expect("audio PES start packet present");

        // Locate the PES payload start. The adaptation_field_control bits
        // (bits 5-4 of byte 3) determine whether an adaptation field is
        // present. When set to 0b11 the adaptation field comes first, and
        // byte 4 holds its length — skip past it to reach the payload.
        let afc = (packet[3] >> 4) & 0b11;
        let payload_start = if afc == 0b11 {
            5 + packet[4] as usize // 4 (TS hdr) + 1 (af_length byte) + af_length
        } else {
            4 // payload-only (afc == 0b01): payload starts right after TS header
        };

        let pes = &packet[payload_start..];
        assert_eq!(&pes[0..3], &[0x00, 0x00, 0x01], "PES start code");
        assert_eq!(pes[3], 0xC0, "stream_id = first audio (0xC0)");
        // flags2 byte at PES offset 7 — high two bits are PTS_DTS_flags
        assert_eq!(pes[7] >> 6, 0b10, "PTS only (no DTS)");
    }

    #[test]
    fn bare_push_audio_rejects_when_two_audio_streams_configured() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_audio(0x300, AudioCodec::Aac)
            .add_audio(0x301, AudioCodec::Mp2)
            .end_program()
            .build()
            .unwrap();
        let mut muxer = Muxer::new(cfg).unwrap();
        let err = muxer.push_audio(b"frame", 90_000).unwrap_err();
        assert!(
            matches!(
                err,
                MuxError::AmbiguousTarget {
                    kind: "audio",
                    count: 2
                }
            ),
            "expected AmbiguousTarget {{ audio, 2 }}, got {err:?}",
        );
    }

    #[test]
    fn audio_handles_lists_in_declaration_order() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_audio(0x300, AudioCodec::Aac)
            .add_audio(0x301, AudioCodec::Mp2)
            .end_program()
            .add_program(2, 0x1100)
            .add_audio(0x400, AudioCodec::Ac3)
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();
        let handles = muxer.audio_handles();
        assert_eq!(handles.len(), 3);
        assert_eq!(handles[0].unpack(), (0, 0));
        assert_eq!(handles[1].unpack(), (0, 1));
        assert_eq!(handles[2].unpack(), (1, 0));
    }

    #[test]
    fn audio_handles_for_program_filters_correctly() {
        let cfg = Config::builder()
            .add_program(7, 0x1000)
            .add_audio(0x300, AudioCodec::Aac)
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();
        let handles = muxer.audio_handles_for_program(7).unwrap();
        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].unpack(), (0, 0));

        // Unknown program number rejects.
        assert!(muxer.audio_handles_for_program(99).is_err());
    }

    #[test]
    fn stream_descriptors_for_audio_attaches_at_build_time() {
        use crate::mpegts::descriptors::iso_639_language;
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_audio(0x300, AudioCodec::Aac)
            .stream_descriptors_for_audio(0, vec![iso_639_language(*b"eng", 0)])
            .end_program()
            .build()
            .unwrap();
        // The descriptor list reaches the per-program stream_descriptors slot.
        let prog = &cfg.programs[0];
        let audio_idx = prog
            .streams
            .iter()
            .position(|s| matches!(s, StreamSpec::Audio { .. }))
            .unwrap();
        assert_eq!(prog.stream_descriptors[audio_idx].len(), 1);
    }

    #[test]
    fn add_subtitle_records_the_stream_in_program_order() {
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
            .end_program()
            .build()
            .unwrap();
        // The subtitle stream is the 2nd entry in this program's streams Vec
        // (after the video at index 0).
        assert!(matches!(
            &cfg.programs[0].streams[1],
            StreamSpec::Subtitle {
                pid: 0x200,
                codec: SubtitleCodec::WebVttInTs,
            }
        ));
    }

    #[test]
    fn stream_descriptors_for_subtitle_attaches_at_build_time() {
        // stream_identifier_descriptor: tag 0x52, len 0x01, component_tag 0x42.
        let extra: Vec<Vec<u8>> = vec![vec![0x52u8, 0x01, 0x42]];
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
            .stream_descriptors_for_subtitle(0, extra.clone())
            .end_program()
            .build()
            .unwrap();
        // abs_idx 1 (after video at 0).
        assert_eq!(cfg.programs[0].stream_descriptors[1], extra);
    }

    #[test]
    fn push_subtitle_to_emits_pes_for_configured_handle() {
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();
        let handles = mux.subtitle_handles();
        assert_eq!(handles.len(), 1);

        mux.push_subtitle_to(
            handles[0],
            90_000,
            b"WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nhello\n",
        )
        .unwrap();

        let mut buf = vec![0u8; 188 * 64];
        let n = mux.pull(&mut buf);
        assert!(n > 0, "expected at least one TS packet");

        // At least one TS packet was emitted on PID 0x200.
        let saw_subtitle_pid = buf[..n]
            .chunks_exact(188)
            .any(|p| p[0] == 0x47 && (((p[1] as u16 & 0x1F) << 8) | (p[2] as u16)) == 0x200);
        assert!(
            saw_subtitle_pid,
            "expected a TS packet on subtitle PID 0x200"
        );
    }

    #[test]
    fn push_subtitle_bare_rejects_when_multiple_subtitle_streams() {
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
            .add_subtitle(
                0x201,
                SubtitleCodec::DvbTeletext {
                    language: *b"eng",
                    teletext_type: 0x02,
                    magazine_number: 1,
                    page_number: 0x88,
                },
            )
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();
        let err = mux.push_subtitle(90_000, b"x").unwrap_err();
        assert!(
            matches!(
                err,
                MuxError::AmbiguousTarget {
                    kind: "subtitle",
                    count: 2,
                }
            ),
            "expected AmbiguousTarget {{ subtitle, 2 }}, got {err:?}",
        );
    }

    #[test]
    fn push_subtitle_payload_too_large_rejected() {
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();
        let too_big = vec![0u8; 70_000];
        let err = mux.push_subtitle(90_000, &too_big).unwrap_err();
        assert!(
            matches!(err, MuxError::SubtitleTooLarge { .. }),
            "expected SubtitleTooLarge, got {err:?}",
        );
    }

    #[test]
    fn subtitle_handles_returns_one_per_configured_stream_across_programs() {
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
            .end_program()
            .add_program(2, 0x300)
            .add_video(0x301, VideoCodec::H265)
            .add_subtitle(
                0x400,
                SubtitleCodec::DvbSubtitling {
                    language: *b"eng",
                    subtitling_type: 0x10,
                    composition_page_id: 1,
                    ancillary_page_id: 1,
                },
            )
            .add_subtitle(
                0x401,
                SubtitleCodec::DvbTeletext {
                    language: *b"spa",
                    teletext_type: 0x02,
                    magazine_number: 1,
                    page_number: 0x88,
                },
            )
            .end_program()
            .build()
            .unwrap();
        let mux = Muxer::new(cfg).unwrap();
        assert_eq!(mux.subtitle_handles().len(), 3);

        let p1 = mux.subtitle_handles_for_program(1).unwrap();
        assert_eq!(p1.len(), 1);
        let p2 = mux.subtitle_handles_for_program(2).unwrap();
        assert_eq!(p2.len(), 2);
    }

    #[test]
    fn subtitle_handles_for_unknown_program_returns_error() {
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .end_program()
            .build()
            .unwrap();
        let mux = Muxer::new(cfg).unwrap();
        assert!(mux.subtitle_handles_for_program(99).is_err());
    }

    // ── Task 8: subtitle Config::validate tests ──────────────────────────

    #[test]
    fn config_validate_too_many_subtitle_streams() {
        let mut prog_builder = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264);
        for i in 0..17 {
            prog_builder = prog_builder.add_subtitle(0x200 + i, SubtitleCodec::WebVttInTs);
        }
        let err = prog_builder.end_program().build().unwrap_err();
        assert!(matches!(
            err,
            MuxError::TooManySubtitleStreams { count: 17, cap: 16 }
        ));
    }

    #[test]
    fn config_validate_subtitle_pid_conflicts_with_video_pid() {
        let err = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(0x101, SubtitleCodec::WebVttInTs)
            .end_program()
            .build()
            .unwrap_err();
        // Existing within-program PID uniqueness check.
        assert!(matches!(err, MuxError::InvalidConfig(_)));
    }

    #[test]
    fn config_validate_rejects_subtitle_pid_as_pcr() {
        let err = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
            .pcr_pid(0x200) // pin PCR to the subtitle PID
            .end_program()
            .build()
            .unwrap_err();
        assert!(matches!(
            err,
            MuxError::SubtitlePidUsedAsPcrPid { pid: 0x200 }
        ));
    }

    #[test]
    fn validate_rejects_caller_pinned_pcr_on_klv_pid() {
        // Caller pins pcr_pid=0x101 explicitly to a KLV stream.
        let err = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x200, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .pcr_pid(0x101)
            .end_program()
            .build()
            .unwrap_err();
        assert!(
            matches!(err, MuxError::KlvPidUsedAsPcrPid { pid: 0x101 }),
            "expected KlvPidUsedAsPcrPid {{ pid: 0x101 }}, got {err:?}"
        );
    }

    #[test]
    fn validate_rejects_klv_only_program_via_pcr_fallback() {
        // No video, no audio — only KLV. The fallback chain
        // `video > KLV > audio` would resolve PCR to the first KLV PID.
        let err = Config::builder()
            .add_program(1, 0x100)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .end_program()
            .build()
            .unwrap_err();
        assert!(
            matches!(err, MuxError::KlvPidUsedAsPcrPid { pid: 0x101 }),
            "expected KlvPidUsedAsPcrPid for fallback-resolved KLV PID, got {err:?}"
        );
    }

    #[test]
    fn validate_accepts_pcr_pinned_to_video_with_klv_present() {
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x200, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .pcr_pid(0x200)
            .end_program()
            .build();
        cfg.expect("video-as-PCR is fine");
    }

    #[test]
    fn validate_accepts_audio_as_pcr() {
        // AAC frames push at ~21 ms intervals — within the 100 ms ETSI TR
        // 101 290 ceiling. Audio-as-PCR remains permitted.
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_audio(0x201, AudioCodec::Aac)
            .end_program()
            .build();
        cfg.expect("audio-as-PCR fallback is fine");
    }

    #[test]
    fn config_validate_rejects_non_ascii_language_code() {
        let err = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(
                0x200,
                SubtitleCodec::DvbSubtitling {
                    language: [0xFF, 0xFE, 0xFD],
                    subtitling_type: 0x10,
                    composition_page_id: 1,
                    ancillary_page_id: 1,
                },
            )
            .end_program()
            .build()
            .unwrap_err();
        assert!(matches!(err, MuxError::InvalidLanguageCode { .. }));
    }

    #[test]
    fn config_validate_rejects_magazine_out_of_range() {
        let err = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(
                0x200,
                SubtitleCodec::DvbTeletext {
                    language: *b"eng",
                    teletext_type: 0x02,
                    magazine_number: 8, // out of range (3-bit; max 7)
                    page_number: 0x88,
                },
            )
            .end_program()
            .build()
            .unwrap_err();
        assert!(matches!(err, MuxError::InvalidTeletextField { .. }));
    }

    /// Reassemble the PES payload bytes for a single PID across the TS packets
    /// emitted in `buf[..n]`. Strips PES header. Used by AU cell auto-wrap tests.
    fn reassemble_pes_payload_for_pid(buf: &[u8], n: usize, target_pid: u16) -> Vec<u8> {
        let mut payload = Vec::new();
        for pkt in buf[..n].chunks_exact(188) {
            let pid = ((pkt[1] as u16 & 0x1F) << 8) | pkt[2] as u16;
            if pid != target_pid {
                continue;
            }
            let payload_unit_start = (pkt[1] & 0x40) != 0;
            let adaptation_present = (pkt[3] & 0x20) != 0;
            let mut idx = 4usize;
            if adaptation_present {
                let af_len = pkt[idx] as usize;
                idx += 1 + af_len;
            }
            if payload_unit_start && idx + 9 <= 188 {
                // Standard PES: start_code(3) + stream_id(1) + length(2) +
                // flags(2) + PES_header_data_length(1) + N PTS bytes.
                let pes_header_data_length = pkt[idx + 8] as usize;
                idx += 9 + pes_header_data_length;
            }
            if idx < 188 {
                payload.extend_from_slice(&pkt[idx..188]);
            }
        }
        payload
    }

    #[test]
    fn sync_klv_push_auto_wraps_with_5_byte_au_cell_header() {
        use crate::mpegts::au_cell::{CellFragmentIndication, read_metadata_au_cell};

        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::SynchronousMetadata, true)
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();

        // Push first sync-KLV blob — synthetic ST 0601-shaped LS.
        let mut inner_klv = Vec::new();
        inner_klv.extend_from_slice(&[
            0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00,
            0x00, 0x00,
        ]);
        inner_klv.push(0x04);
        inner_klv.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        mux.push_klv(&inner_klv, 90_000, 0x00).unwrap();

        let mut buf = vec![0u8; 188 * 32];
        let n = mux.pull(&mut buf);
        let pes_payload = reassemble_pes_payload_for_pid(&buf, n, 0x1031);
        assert!(
            !pes_payload.is_empty(),
            "expected at least one TS packet on KLV PID 0x1031"
        );

        // PES payload must start with the 5-byte AU cell header followed by
        // the inner KLV bytes verbatim.
        let (hdr, body) = read_metadata_au_cell(&pes_payload).expect("valid AU cell header");
        assert_eq!(hdr.metadata_service_id, 0x00, "ST 1402.2 App. B default");
        assert_eq!(hdr.sequence_number, 0, "first push starts at seq 0");
        assert_eq!(
            hdr.cell_fragment_indication,
            CellFragmentIndication::Complete
        );
        assert!(!hdr.decoder_config_flag);
        assert!(hdr.random_access_indicator);
        assert_eq!(body, &inner_klv[..]);

        // Push second blob; sequence_number must increment.
        mux.push_klv(&inner_klv, 90_000 * 2, 0x00).unwrap();
        let n2 = mux.pull(&mut buf);
        let pes2 = reassemble_pes_payload_for_pid(&buf, n2, 0x1031);
        let (hdr2, _) = read_metadata_au_cell(&pes2).expect("valid AU cell header");
        assert_eq!(
            hdr2.sequence_number, 1,
            "sequence_number must increment per push"
        );
    }

    #[test]
    fn private_data_klv_does_not_auto_wrap() {
        // PrivateData streams must pass payload through as-is; the muxer
        // must NOT prepend an AU cell header.
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();

        let mut inner_klv = Vec::new();
        inner_klv.extend_from_slice(&[
            0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00,
            0x00, 0x00,
        ]);
        inner_klv.push(0x00);
        mux.push_klv(&inner_klv, 0, 0x00).unwrap();

        let mut buf = vec![0u8; 188 * 32];
        let n = mux.pull(&mut buf);
        let pes_payload = reassemble_pes_payload_for_pid(&buf, n, 0x1031);
        assert_eq!(
            &pes_payload[..inner_klv.len()],
            &inner_klv[..],
            "PrivateData payload must pass through unchanged"
        );
    }

    // ── Subtitle auto-emit suppression on caller-supplied descriptors ────

    #[test]
    fn subtitle_auto_emit_suppressed_on_caller_supplied_subtitling() {
        // Caller supplies a 2-entry subtitling_descriptor; the muxer must
        // NOT also auto-emit the single-entry one for this PID — caller's
        // takes precedence. Mirrors the KLV/AV1 caller-supplied-Registration
        // suppression rule.
        let caller_desc = crate::mpegts::descriptors::subtitling_descriptor_multi(&[
            (*b"eng", 0x10, 1, 1),
            (*b"spa", 0x10, 2, 2),
        ])
        .expect("non-empty entries");
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(
                0x200,
                SubtitleCodec::DvbSubtitling {
                    language: *b"eng",
                    subtitling_type: 0x10,
                    composition_page_id: 1,
                    ancillary_page_id: 1,
                },
            )
            .stream_descriptors_for_subtitle(0, vec![caller_desc.clone()])
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();

        // Inspect the per-stream descriptor cache for the subtitle stream's
        // PMT entry. Stream index 1 = subtitle (after video at index 0).
        // There must be exactly one 0x59 descriptor — the caller's
        // multi-entry one — and no auto-emitted single-entry one.
        let cache = &muxer.pmt_descriptor_caches[0][1];
        let mut count_0x59 = 0;
        let mut idx = 0;
        while idx + 1 < cache.len() {
            let tag = cache[idx];
            let len = cache[idx + 1] as usize;
            if tag == 0x59 {
                count_0x59 += 1;
                assert_eq!(&cache[idx..idx + 2 + len], &caller_desc[..]);
            }
            idx += 2 + len;
        }
        assert_eq!(
            count_0x59, 1,
            "auto-emit must suppress when caller supplies subtitling_descriptor"
        );
    }

    #[test]
    fn subtitle_auto_emit_suppressed_on_caller_supplied_teletext() {
        // Caller supplies a teletext_descriptor (tag 0x56); auto-emit must
        // suppress.
        let caller_desc = crate::mpegts::descriptors::teletext_descriptor(*b"eng", 0x02, 1, 0x88);
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(
                0x200,
                SubtitleCodec::DvbTeletext {
                    language: *b"fra",
                    teletext_type: 0x02,
                    magazine_number: 2,
                    page_number: 0x77,
                },
            )
            .stream_descriptors_for_subtitle(0, vec![caller_desc.clone()])
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();
        let cache = &muxer.pmt_descriptor_caches[0][1];
        // Exactly the caller's bytes — no auto-emit prepended.
        assert_eq!(cache, &caller_desc);
    }

    #[test]
    fn subtitle_auto_emit_suppressed_on_caller_supplied_vttc_registration() {
        // Caller supplies a VTTC registration_descriptor; suppress auto-emit.
        let caller_desc = vec![0x05u8, 0x04, b'V', b'T', b'T', b'C'];
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
            .stream_descriptors_for_subtitle(0, vec![caller_desc.clone()])
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();
        let cache = &muxer.pmt_descriptor_caches[0][1];
        // Exactly the caller's bytes — no double VTTC.
        assert_eq!(cache, &caller_desc);
    }

    #[test]
    fn subtitle_auto_emit_fires_when_caller_supplies_unrelated_descriptors() {
        // Caller supplies stream_identifier_descriptor (tag 0x52) — not a
        // subtitle codec marker — so auto-emit must still fire.
        let unrelated = vec![0x52u8, 0x01, 0x42];
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(
                0x200,
                SubtitleCodec::DvbSubtitling {
                    language: *b"eng",
                    subtitling_type: 0x10,
                    composition_page_id: 1,
                    ancillary_page_id: 1,
                },
            )
            .stream_descriptors_for_subtitle(0, vec![unrelated])
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();
        let cache = &muxer.pmt_descriptor_caches[0][1];
        // Walk descriptors; expect exactly one 0x59 (the auto-emit).
        let mut count_0x59 = 0;
        let mut idx = 0;
        while idx + 1 < cache.len() {
            let tag = cache[idx];
            let len = cache[idx + 1] as usize;
            if tag == 0x59 {
                count_0x59 += 1;
            }
            idx += 2 + len;
        }
        assert_eq!(
            count_0x59, 1,
            "auto-emit must fire when caller-supplied descriptors don't include a subtitle codec marker"
        );
    }

    #[test]
    fn validate_language_code_accepts_uppercase_per_en_300_468() {
        // ISO/IEC 8859-1 character coding does not mandate lowercase. Real-world
        // DVB encoders sometimes emit uppercase ISO 639-2 codes.
        assert!(
            validate_language_code(*b"ENG").is_ok(),
            "uppercase ASCII alphabetic must validate"
        );
        assert!(
            validate_language_code(*b"eng").is_ok(),
            "lowercase still accepted"
        );
        assert!(
            validate_language_code(*b"EnG").is_ok(),
            "mixed case accepted"
        );
        // Non-letters still rejected — admitting digits/symbols would let
        // junk through.
        assert!(validate_language_code(*b"123").is_err());
        assert!(validate_language_code(*b"e n").is_err());
        assert!(validate_language_code([0x00, 0x01, 0x02]).is_err());
    }

    #[test]
    fn validate_rejects_subtitle_only_program() {
        // Subtitles must not carry PCR per ETSI EN 300 472 §4.0 +
        // EN 300 743 §6.1. The PCR fallback chain (caller-pinned > video >
        // KLV > audio) excludes subtitles, so a subtitle-only program has
        // no resolvable PCR PID.
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
            .end_program()
            .build();
        match cfg {
            Err(MuxError::SubtitleOnlyProgram { program_number }) => {
                assert_eq!(program_number, 1);
            }
            other => panic!("expected SubtitleOnlyProgram, got {other:?}"),
        }
    }

    #[test]
    fn validate_accepts_video_plus_subtitle_program() {
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
            .end_program()
            .build();
        assert!(cfg.is_ok(), "video + subtitle program must validate");
    }

    #[test]
    fn stream_kind_display() {
        assert_eq!(StreamKind::Video.to_string(), "video");
        assert_eq!(StreamKind::Audio.to_string(), "audio");
        assert_eq!(StreamKind::Klv.to_string(), "klv");
        assert_eq!(StreamKind::Subtitle.to_string(), "subtitle");
    }

    #[test]
    fn teletext_field_display() {
        assert_eq!(TeletextField::MagazineNumber.to_string(), "magazine_number");
        assert_eq!(TeletextField::TeletextType.to_string(), "teletext_type");
    }
}

#[cfg(test)]
mod stats_tests {
    use super::*;

    #[test]
    fn stats_starts_with_per_stream_entries_for_configured_streams() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .end_program()
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
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .end_program()
            .build()
            .unwrap();
        let mut m = Muxer::new(cfg).unwrap();
        let nal: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67, 0xBB, 0xCC];
        m.push_video(nal, 0, true).unwrap();
        let klv: &[u8] = &[
            0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00,
            0x00, 0x00, 0x00,
        ];
        m.push_klv(klv, 0, 0x00).unwrap();
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
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .end_program()
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

    #[test]
    fn h266_video_per_stream_stats_records_stream_type_0x33() {
        // Exercises the VideoCodec::H266 -> StreamType::H266 mapping arm in
        // Muxer::new's per_stream stats setup (the second of two H266 sites
        // in mux/mod.rs that previously panicked with unimplemented!()).
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, VideoCodec::H266)
            .end_program()
            .build()
            .unwrap();
        let m = Muxer::new(cfg).unwrap();
        let st = m.stats();
        assert_eq!(st.per_stream[&0x101].stream_type, 0x33);
    }

    /// Per H.222.0 V9 §2.4.3.5: "In the PCR_PID the random_access_indicator
    /// may only be set to '1' in a transport stream packet containing the PCR
    /// fields." Prior code unconditionally set RA=1 on a key-frame's first TS
    /// packet whether or not that packet also carried a PCR — emitted when
    /// key-frame timing landed between PCR ticks.
    ///
    /// This test pushes two key-frames close enough that the second is not
    /// pcr_due, and asserts the second key-frame's first packet either carries
    /// a PCR (forced emission) or has RA=0.
    #[test]
    fn random_access_indicator_only_on_packets_with_pcr() {
        let cfg = Config::builder()
            // Default pcr_interval_ms = 40, plenty of room for a "not due" gap.
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();

        // Synthetic Annex-B H.264 IDR access unit.
        let nal: &[u8] = &[
            0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x21,
            0xff, // start_code + IDR header + filler
        ];

        // First key-frame at PTS=0 — pcr_last is None, so PCR is due. This
        // first packet should carry both PCR and RA.
        mux.push_video(nal, 0, true).unwrap();
        // Second key-frame at PTS=10ms (= 900 90kHz ticks) — well below the
        // 40ms PCR threshold. PCR is NOT due. After the fix we force PCR
        // emission on PCR_PID + key_frame; the buggy code would set RA=1
        // without a PCR.
        mux.push_video(nal, 900, true).unwrap();

        let mut all = Vec::new();
        let mut buf = vec![0u8; 1316];
        loop {
            let n = mux.pull(&mut buf);
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n]);
        }

        // Walk all PUSI packets on PID 0x1011. Skip the first (it's the
        // first key-frame and has PCR by virtue of pcr_last=None).
        let pusi_packets: Vec<&[u8]> = all
            .chunks_exact(188)
            .filter(|p| {
                p[0] == 0x47
                    && (((p[1] as u16 & 0x1F) << 8) | (p[2] as u16)) == 0x1011
                    && (p[1] & 0x40) != 0
            })
            .collect();
        assert!(
            pusi_packets.len() >= 2,
            "expected at least two PUSI packets on video PID, got {}",
            pusi_packets.len(),
        );

        let second = pusi_packets[1];
        // adaptation_field_control: bits 5-4 of byte 3.
        let afc = (second[3] >> 4) & 0b11;
        assert!(
            afc == 0b11 || afc == 0b10,
            "second key-frame packet must carry adaptation field; afc = {afc:#b}",
        );
        let af_length = second[4] as usize;
        // af_length = 0 means just the length byte itself, no flags. With RA we
        // expect a flags byte at byte 5.
        assert!(
            af_length >= 1,
            "second key-frame AF must include flags; len = {af_length}",
        );
        let af_flags = second[5];
        let random_access = (af_flags & 0b0100_0000) != 0;
        let pcr_present = (af_flags & 0b0001_0000) != 0;
        assert!(
            random_access,
            "second key-frame should still indicate random_access (it's an IDR)",
        );
        assert!(
            pcr_present,
            "spec rule: RA on PCR_PID must coincide with PCR — \
             second key-frame has RA but no PCR (af_flags = {af_flags:#b})",
        );
    }

    #[test]
    fn muxer_stats_reports_subtitle_streams_configured() {
        let cfg = Config::builder()
            .add_program(1, 0x100)
            .add_video(0x101, VideoCodec::H264)
            .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
            .add_subtitle(
                0x201,
                SubtitleCodec::DvbTeletext {
                    language: *b"eng",
                    teletext_type: 0x02,
                    magazine_number: 1,
                    page_number: 0x88,
                },
            )
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();
        mux.push_subtitle_to(SubtitleStreamHandle::pack(0, 0), 90_000, b"x")
            .unwrap();
        let s = mux.stats();
        assert_eq!(s.subtitle_streams_configured, 2);
        let stream_stat = s.per_stream.get(&0x200).unwrap();
        assert_eq!(stream_stat.label.as_deref(), Some("WebVTT-in-TS"));
        assert!(stream_stat.items >= 1);
        let teletext_stat = s.per_stream.get(&0x201).unwrap();
        assert_eq!(teletext_stat.label.as_deref(), Some("DVB-Teletext"));
    }
}
