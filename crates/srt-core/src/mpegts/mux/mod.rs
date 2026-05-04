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
    },
}

impl StreamSpec {
    pub(crate) fn pid(&self) -> u16 {
        match self {
            StreamSpec::Video { pid, .. } => *pid,
            StreamSpec::Klv { pid, .. } => *pid,
            StreamSpec::Audio { pid, .. } => *pid,
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
            let mut video_count = 0;
            let mut klv_count = 0;
            for s in &prog.streams {
                match s {
                    StreamSpec::Video { .. } => video_count += 1,
                    StreamSpec::Klv { .. } => klv_count += 1,
                    StreamSpec::Audio { .. } => {
                        // Audio stream count validation will be added in a later task.
                    }
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
/// use srt_core::mpegts::mux::{Config, KlvStreamType, VideoCodec};
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
    /// Per-stream counters, keyed by PID. One entry per configured
    /// video or KLV stream. `StreamStats::items` = push_video_to /
    /// push_klv_to call count; `StreamStats::bytes` = raw ES bytes pushed
    /// (before PES/TS framing overhead).
    pub per_stream: BTreeMap<u16, crate::mpegts::stats::StreamStats>,
}

use self::pes::{
    MAX_PES_HEADER_SIZE, PesPtsField, STREAM_ID_KLV, STREAM_ID_VIDEO, write_audio_pes,
    write_pes_header,
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
}

/// Per-audio-stream cached state.
struct AudioStreamState {
    pid: u16,
    codec: AudioCodec,
}

/// Sender-side MPEG-TS muxer.
///
/// Construct with `Muxer::new(config)`, push encoded frames via `push_video`
/// and `push_klv`, then drain TS packets with `pull`. The muxer is
/// deterministic — output is a function of inputs only, not wall-clock time.
///
/// See the design doc for full semantics:
/// `docs/specs/2026-05-01-srt-core-mpegts-mux-design.md`.
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
                    }),
                    _ => None,
                })
                .collect();
            let prog_audio: Vec<AudioStreamState> = prog
                .streams
                .iter()
                .filter_map(|s| match s {
                    StreamSpec::Audio { pid, codec } => Some(AudioStreamState {
                        pid: *pid,
                        codec: *codec,
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
                prog_cache.push(bytes);
            }

            // Eagerly create per-stream stats entries.
            for v in &prog_video {
                let stream_type_byte = match v.codec {
                    VideoCodec::H264 => StreamType::H264.as_u8(),
                    VideoCodec::H265 => StreamType::H265.as_u8(),
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

            video_streams.push(prog_video);
            klv_streams.push(prog_klv);
            audio_streams.push(prog_audio);
            pcr_pids.push(pcr_pid);
            pmt_descriptor_caches.push(prog_cache);
        }

        Ok(Self {
            config,
            pmt_descriptor_caches,
            video_streams,
            klv_streams,
            audio_streams,
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
    pub fn push_klv(&mut self, klv: &[u8], pts_90khz: i64) -> Result<(), MuxError> {
        let total_klv: usize = self.klv_streams.iter().map(|k| k.len()).sum();
        if total_klv != 1 {
            return Err(MuxError::AmbiguousTarget {
                kind: "klv",
                count: total_klv,
            });
        }
        let handle = KlvStreamHandle::pack(0, 0);
        self.push_klv_to(handle, klv, pts_90khz)
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
        if total_audio != 1 {
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

        let pts = PesPtsField::PtsOnly(Pts90khz(pts_90khz));
        let mut pes_buf = Vec::with_capacity(MAX_PES_HEADER_SIZE + frames.len());
        write_audio_pes(&mut pes_buf, within_idx as u8, pts, frames);

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
        validate_annex_b(nal)?;

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

        let mut header = [0u8; MAX_PES_HEADER_SIZE];
        let header_len = write_pes_header(
            &mut header,
            STREAM_ID_VIDEO,
            PesPtsField::PtsOnly(Pts90khz(pts_90khz)),
            None,
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
                if self.pcr_pids[prog_idx] == video_pid && self.pcr_due(prog_idx, pts_90khz) {
                    let pcr = Pcr27mhz::from_pts(Pts90khz(pts_90khz));
                    adaptation.pcr = Some(pcr);
                    self.pcr_last[prog_idx] = Some(pcr.0);
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
        pes_buf.extend_from_slice(klv);

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
            programs_configured: self.config.programs.len() as u32,
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
    fn accepts_pcr_pid_on_klv() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .pcr_pid(0x1031)
            .end_program()
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
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .pcr_pid(0x1031)
            .end_program()
            .build()
            .unwrap();
        let mux = Muxer::new(cfg).unwrap();
        assert_eq!(mux.pcr_pids[0], 0x1031);
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
                .add_program(1, 0x1000)
                .add_video(0x1011, VideoCodec::H264)
                .add_klv(0x1031, KlvStreamType::PrivateData, true)
                .end_program()
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
    fn muxer_new_accepts_klv_only() {
        // KLV-only requires PCR pinned to KLV PID (it carries PCR).
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_klv(0x1031, KlvStreamType::PrivateData, true)
            .pcr_pid(0x1031)
            .end_program()
            .build()
            .unwrap();
        let mux = Muxer::new(cfg);
        assert!(mux.is_ok(), "klv-only muxer must construct");
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
            .add_program(1, 0x1000)
            .add_klv(0x1031, KlvStreamType::PrivateData, true)
            .pcr_pid(0x1031)
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
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .stream_descriptors_for_klv(
                0,
                vec![crate::mpegts::descriptors::registration(*b"KLVA", &[])],
            )
            .end_program()
            .build()
            .unwrap();
        let muxer = Muxer::new(cfg).unwrap();

        // Caller's Registration only — auto-emit suppressed. Total = 6 bytes.
        assert_eq!(muxer.pmt_descriptor_caches[0][0].len(), 6);
    }

    #[test]
    fn cache_no_auto_emit_on_sync_klv() {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
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
        // No KLVA auto-emit on SynchronousMetadata. 11 + 11 = 22 bytes.
        assert_eq!(muxer.pmt_descriptor_caches[0][0].len(), 22);
        assert_eq!(muxer.pmt_descriptor_caches[0][0][0], 0x26);
        assert_eq!(muxer.pmt_descriptor_caches[0][0][11], 0x27);
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
}
