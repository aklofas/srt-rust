//! Muxer configuration types — programs, stream specs, and chainable builders.
//!
//! `MuxerConfig::validate` enforces program/stream caps and uniqueness rules
//! at builder-time. The chainable builders (`MuxerConfigBuilder`,
//! `MuxerProgramConfigBuilder`) follow the Phase 3 `&mut self -> &mut Self`
//! shape. Production muxer code lives in `mod.rs`.

use crate::error::MuxError;
use crate::mpegts::common::pid;
use crate::mpegts::mux::types::{
    AudioCodec, Av1CarriageMode, KlvStreamType, MAX_PROGRAMS, MAX_SUBTITLE_STREAMS_PER_PROGRAM,
    StreamKind, StreamSpec, SubtitleCodec, TeletextField, VideoCodec,
};
use alloc::string::String;
use alloc::vec::Vec;

/// One program in a multi-program TS multiplex. Each program has its own
/// PMT (carried on `pmt_pid`), its own PCR (driven by `pcr_pid` or
/// auto-falling-back to the first video stream's PID), and its own
/// elementary stream set.
#[must_use]
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct MuxerProgramConfig {
    /// Program number (PAT entry). Must be > 0 (program 0 is reserved for
    /// network information). Must be unique across all programs in the
    /// MuxerConfig.
    pub program_number: u16,

    /// PID carrying this program's PMT. PAT lists `(program_number, pmt_pid)`
    /// tuples. Must not collide with any stream PID in any program, and must
    /// be unique across all programs in the MuxerConfig.
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
    /// inner is the descriptor list for that stream. Hand-built
    /// `MuxerProgramConfig` callers must keep
    /// `stream_descriptors.len() == streams.len()`;
    /// `MuxerConfigBuilder::build()` enforces this.
    pub stream_descriptors: Vec<Vec<Vec<u8>>>,
}

impl MuxerProgramConfig {
    /// Construct a new program config with empty stream/descriptor sets.
    ///
    /// External callers (different-crate code that cannot use struct
    /// literal syntax against this `#[non_exhaustive]` type) build a
    /// `MuxerProgramConfig` by:
    ///
    /// 1. Calling `MuxerProgramConfig::new(program_number, pmt_pid)` to
    ///    seed the required PIDs.
    /// 2. Populating `streams`, `pcr_pid`, `program_descriptors`, and
    ///    `stream_descriptors` via direct field assignment.
    ///
    /// Alternatively, the [`MuxerProgramConfigBuilder`] offers a
    /// chainable shape that handles `streams` + `stream_descriptors`
    /// in parallel — preferred when assembling more than 2-3 streams.
    ///
    /// `program_number` must be > 0 (program 0 is reserved for network
    /// information) and unique within the outer [`MuxerConfig`].
    /// `pmt_pid` carries this program's PMT and must not collide with
    /// any other PID. These constraints are enforced by
    /// `MuxerConfig::validate()`, not by this constructor.
    pub fn new(program_number: u16, pmt_pid: u16) -> Self {
        Self {
            program_number,
            pmt_pid,
            streams: Vec::new(),
            pcr_pid: None,
            program_descriptors: Vec::new(),
            stream_descriptors: Vec::new(),
        }
    }

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
/// Contains one or more [`MuxerProgramConfig`]s. Multi-program transport
/// streams carry a PAT that lists all programs; each program has its own PMT.
///
/// Construct with [`MuxerConfig::builder()`] for ergonomic chaining, or
/// directly with field updates over [`MuxerConfig::default()`] for the
/// canonical single-program single-video-plus-single-KLV case.
#[must_use]
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct MuxerConfig {
    /// Programs in this multiplex. ≤ `MAX_PROGRAMS`, ≥ 1.
    pub programs: Vec<MuxerProgramConfig>,

    /// PCR re-emission interval, in milliseconds. Default 40. Validation 1..=100.
    /// Applied per-program (each program's PCR PID re-emits independently).
    pub pcr_interval_ms: u32,

    /// PAT/PMT re-emission interval, in milliseconds. Default 100. Validation >= 10.
    /// One PAT + N PMTs emitted per tick.
    pub psi_interval_ms: u32,

    /// Maximum buffered TS packets before push returns `BufferFull`.
    /// Default 10000 (~1.88 MB, ~600 ms at 25 Mbps). Validation: >= 10.
    pub buffer_packets: usize,

    /// AV1 PES carriage mode — see [`Av1CarriageMode`].
    ///
    /// Default is [`Av1CarriageMode::Mpeg2TsBinding`] for spec
    /// conformance (PES `stream_id=0xBD`, `ts_open_bitstream_unit()`
    /// framing). Set to [`Av1CarriageMode::InteropRawObu`] for the
    /// ffmpeg / libaom / hls.js / mediamtx interop carriage.
    ///
    /// Has no effect on non-AV1 streams. The matching
    /// `DemuxerConfig::av1_carriage` controls the receiver-side
    /// expectation — the two MUST match for a round-trip.
    pub av1_carriage: Av1CarriageMode,
}

impl Default for MuxerConfig {
    fn default() -> Self {
        // Single program: H.264 video at 0x1011, KLV PrivateData at 0x1031,
        // async KLV (no PTS), PCR auto-resolved to first video stream.
        Self {
            programs: vec![MuxerProgramConfig {
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
            av1_carriage: Av1CarriageMode::Mpeg2TsBinding,
        }
    }
}

impl MuxerConfig {
    /// Start a new builder. Equivalent to `MuxerConfigBuilder::default()`.
    pub fn builder() -> MuxerConfigBuilder {
        MuxerConfigBuilder::default()
    }

    /// Build a single-program `MuxerConfig` from a demuxed
    /// [`ProgramMap`](crate::mpegts::demux::ProgramMap).
    ///
    /// This is the transmux bridge: capture the
    /// [`DemuxEvent::ProgramMap`](crate::mpegts::demux::DemuxEvent::ProgramMap)
    /// emitted on PMT discovery and hand it here to obtain a muxer config
    /// that reproduces the program's topology (program number, PMT PID,
    /// stream PIDs, codecs). One `ProgramMap` describes one program, so
    /// the result always contains exactly one [`MuxerProgramConfig`];
    /// convert each program of a multi-program TS separately and combine
    /// the results via [`MuxerConfigBuilder`] by hand.
    ///
    /// # Strictness and the `drop` filter
    ///
    /// Strict by default: any stream the muxer cannot represent —
    /// [`StreamKind::Unknown`](crate::mpegts::demux::StreamKind::Unknown)
    /// stream types, and DVB subtitling/teletext streams (whose
    /// per-stream parameters such as language and page IDs are not
    /// recoverable from the PMT entry alone) — fails the conversion with
    /// [`MuxError::ConfigInvalid`](crate::error::MuxError::ConfigInvalid)
    /// naming every offender. Pass the offenders' kinds in `drop` (e.g.
    /// `&[StreamKindTag::Unknown]`) to exclude those streams instead; see
    /// [`StreamKindTag`](crate::mpegts::demux::StreamKindTag). The filter
    /// is kind-coarse: `StreamKindTag::Subtitle` drops every subtitle
    /// stream, including representable CEA-708 / WebVTT ones, not just the
    /// DVB offenders.
    ///
    /// # Mapping rules
    ///
    /// - Video and audio map codec-for-codec.
    ///   [`StreamKind::KlvSync`](crate::mpegts::demux::StreamKind::KlvSync)
    ///   maps to [`KlvStreamType::SynchronousMetadata`] and
    ///   [`StreamKind::KlvAsync`](crate::mpegts::demux::StreamKind::KlvAsync)
    ///   to [`KlvStreamType::PrivateData`]; CEA-708 / WebVTT subtitle
    ///   streams map to their parameter-free mux variants.
    /// - **`carries_pts` is always `true`**, including for async KLV:
    ///   whether the KLV PES carries a PTS is a PES-level property the PMT
    ///   cannot declare, and PTS-carrying KLV is the STANAG 4609 norm.
    ///   Callers that need a PTS-less async stream build the config by
    ///   hand.
    /// - **PCR copy rule**: the demuxed `pcr_pid` is copied iff it equals
    ///   the PID of a kept non-KLV stream. Otherwise (PCR on a dropped
    ///   stream, on a PID outside the program, or on a KLV PID — which
    ///   [`validate`](Self::validate) would reject) `pcr_pid` is left
    ///   `None`, so the builder default applies: `validate()` resolves
    ///   first video → first KLV → first audio, which can itself error
    ///   for a video-less program whose fallback lands on a KLV PID
    ///   ([`MuxError::KlvPidUsedAsPcrPid`](crate::error::MuxError::KlvPidUsedAsPcrPid)).
    /// - **Audio language**: recovered from the first ISO 639 language
    ///   descriptor (tag `0x0A`) on the stream's raw PMT descriptors when
    ///   it carries a plausible lowercase ISO 639-2 code; otherwise the
    ///   stream is added language-less (never an error).
    /// - `klv_links` are ignored — the muxer re-derives metadata linkage
    ///   from its own configuration.
    ///
    /// # Errors
    ///
    /// [`MuxError::ConfigInvalid`](crate::error::MuxError::ConfigInvalid)
    /// listing the unrepresentable streams not excluded via `drop`, or any
    /// [`MuxerConfigBuilder::build`] validation error on the converted
    /// program (e.g. an empty program when every stream was dropped).
    pub fn from_program_map(
        pm: &crate::mpegts::demux::ProgramMap,
        drop: &[crate::mpegts::demux::StreamKindTag],
    ) -> Result<MuxerConfig, MuxError> {
        use crate::mpegts::demux::{
            AudioCodec as DemuxAudio, StreamKind as DemuxKind, SubtitleCodec as DemuxSub,
            VideoCodec as DemuxVideo,
        };
        let mut prog = MuxerProgramConfigBuilder::new(pm.program_number, pm.pmt_pid);
        let mut offenders: Vec<String> = Vec::new();
        // (pid, is_klv) of every stream added to the builder — drives the PCR copy rule.
        let mut kept: Vec<(u16, bool)> = Vec::new();
        for s in &pm.streams {
            if drop.contains(&s.kind.tag()) {
                continue;
            }
            match &s.kind {
                DemuxKind::Video(c) => {
                    let codec = match c {
                        DemuxVideo::H264 => VideoCodec::H264,
                        DemuxVideo::H265 => VideoCodec::H265,
                        DemuxVideo::H266 => VideoCodec::H266,
                        DemuxVideo::Av1 => VideoCodec::Av1,
                    };
                    prog.add_video(s.pid, codec);
                    kept.push((s.pid, false));
                }
                DemuxKind::Audio(c) => {
                    let codec = match c {
                        DemuxAudio::Mp2 => AudioCodec::Mp2,
                        DemuxAudio::Aac => AudioCodec::Aac,
                        DemuxAudio::AacLatm => AudioCodec::AacLatm,
                        DemuxAudio::Ac3 => AudioCodec::Ac3,
                    };
                    match iso639_language(&s.raw_descriptors) {
                        Some(lang) => prog.add_audio_with_language(s.pid, codec, lang),
                        None => prog.add_audio(s.pid, codec),
                    };
                    kept.push((s.pid, false));
                }
                DemuxKind::KlvSync { .. } => {
                    prog.add_klv(s.pid, KlvStreamType::SynchronousMetadata, true);
                    kept.push((s.pid, true));
                }
                DemuxKind::KlvAsync => {
                    // carries_pts is a PES-level property the PMT cannot declare;
                    // true is the STANAG 4609 norm. Callers needing false build
                    // the config by hand.
                    prog.add_klv(s.pid, KlvStreamType::PrivateData, true);
                    kept.push((s.pid, true));
                }
                DemuxKind::Subtitle(DemuxSub::Cea708Standalone) => {
                    prog.add_subtitle(s.pid, SubtitleCodec::Cea708Standalone);
                    kept.push((s.pid, false));
                }
                DemuxKind::Subtitle(DemuxSub::WebVttInTs) => {
                    prog.add_subtitle(s.pid, SubtitleCodec::WebVttInTs);
                    kept.push((s.pid, false));
                }
                DemuxKind::Subtitle(other) => offenders.push(format!(
                    "pid 0x{:04X} ({other:?} subtitle: per-stream parameters are not \
                     recoverable from the PMT)",
                    s.pid
                )),
                DemuxKind::Unknown(st) => offenders.push(format!(
                    "pid 0x{:04X} (unknown stream_type 0x{st:02X})",
                    s.pid
                )),
            }
        }
        if !offenders.is_empty() {
            return Err(MuxError::ConfigInvalid {
                reason: format!(
                    "from_program_map: streams the muxer cannot represent: {}; pass \
                     their kinds in `drop` to exclude them",
                    offenders.join(", ")
                ),
            });
        }
        if let Some(&(pid, is_klv)) = kept.iter().find(|(pid, _)| *pid == pm.pcr_pid) {
            // Explicit PCR-on-KLV is rejected by validate(); fall back to the
            // builder default (first video) instead.
            if !is_klv {
                prog.pcr_pid(pid);
            }
        }
        MuxerConfig::builder().add_program(prog.build()).build()
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
            const DATA_CAP: usize = 16;
            let mut video_count = 0;
            let mut klv_count = 0;
            let mut audio_count = 0;
            let mut subtitle_count = 0;
            let mut data_count = 0;
            for s in &prog.streams {
                match s {
                    StreamSpec::Video { .. } => video_count += 1,
                    StreamSpec::Klv { .. } => klv_count += 1,
                    StreamSpec::Audio { .. } => audio_count += 1,
                    StreamSpec::Subtitle { .. } => subtitle_count += 1,
                    StreamSpec::Data { .. } => data_count += 1,
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
            if data_count > DATA_CAP {
                return Err(MuxError::TooManyDataStreams {
                    count: data_count,
                    cap: DATA_CAP,
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
                    StreamSpec::Data { pid, .. } => {
                        if !pid::is_user_pid(*pid) {
                            return Err(MuxError::InvalidConfig(
                                "data pid must be in 0x0010..=0x1FFE",
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
                                    field: TeletextField::MagazineNumber,
                                    value: *magazine_number,
                                    max: 7,
                                });
                            }
                            if *teletext_type > 0x1F {
                                return Err(MuxError::InvalidTeletextField {
                                    field: TeletextField::TeletextType,
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
                return Err(MuxError::ConfigInvalid {
                    reason: format!(
                        "program {} has {} streams but {} stream_descriptors \
                         (lengths must match — call the corresponding \
                         stream_descriptors_for_{{video,klv,audio,subtitle,data}} \
                         builder methods or hand-build with parallel Vecs)",
                        prog.program_number,
                        prog.streams.len(),
                        prog.stream_descriptors.len(),
                    ),
                });
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
            let pmt_size = super::psi::estimate_pmt_section_size(prog);
            if pmt_size > super::psi::MAX_PMT_SECTION_BYTES {
                return Err(MuxError::PmtTooLarge {
                    used_bytes: pmt_size,
                    max_bytes: super::psi::MAX_PMT_SECTION_BYTES,
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

/// Ergonomic construction of [`MuxerConfig`]. Build each program with
/// [`MuxerProgramConfigBuilder`] and pass the resulting [`MuxerProgramConfig`]
/// to [`MuxerConfigBuilder::add_program`]; finalize with
/// [`MuxerConfigBuilder::build`].
///
/// All mutators take `&mut self` and return `&mut Self`, so the builder
/// translates cleanly to FFI consumers (Kotlin `apply { }`, Swift `var b`,
/// Java step-wise, Python attribute assignment, C opaque-handle): callers
/// step through mutators on a bound variable rather than chaining a moved
/// value. Inline chaining still works, but bind-then-step is the canonical
/// shape.
///
/// # Example — single program with H.265 video + sync KLV
/// ```
/// use tst_core::mpegts::mux::{
///     KlvStreamType, MuxerConfig, MuxerProgramConfigBuilder, VideoCodec,
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Build the program block first (standalone — no parent borrow).
/// let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
/// prog.add_video(0x1011, VideoCodec::H265);
/// // SynchronousMetadata (PMT stream_type 0x15) requires
/// // `carries_pts: true` so the PTS in the PES header can align each
/// // KLV record with the corresponding video frame. The muxer
/// // auto-prepends the 5-byte H.222.0 §2.12.4.2 Metadata_AU_cell
/// // header on every push.
/// prog.add_klv(0x1031, KlvStreamType::SynchronousMetadata, true);
///
/// // Hand the built program to the outer builder.
/// let mut b = MuxerConfig::builder();
/// b.add_program(prog.build());
/// let config = b.build()?;
/// assert_eq!(config.programs.len(), 1);
/// # Ok(())
/// # }
/// ```
#[must_use]
#[derive(Default, Debug)]
pub struct MuxerConfigBuilder {
    programs: Vec<MuxerProgramConfig>,
    pcr_interval_ms: Option<u32>,
    psi_interval_ms: Option<u32>,
    buffer_packets: Option<usize>,
    av1_carriage: Option<Av1CarriageMode>,
}

impl MuxerConfigBuilder {
    /// Append a fully-constructed [`MuxerProgramConfig`] to this builder.
    ///
    /// Build the program with [`MuxerProgramConfigBuilder`] and pass its
    /// `build()` value here. Validation (duplicate program_number, PMT PID
    /// collision, etc.) is deferred to [`Self::build`] / `MuxerConfig::validate`.
    pub fn add_program(&mut self, program: MuxerProgramConfig) -> &mut Self {
        self.programs.push(program);
        self
    }

    pub fn pcr_interval_ms(&mut self, ms: u32) -> &mut Self {
        self.pcr_interval_ms = Some(ms);
        self
    }

    pub fn psi_interval_ms(&mut self, ms: u32) -> &mut Self {
        self.psi_interval_ms = Some(ms);
        self
    }

    pub fn buffer_packets(&mut self, n: usize) -> &mut Self {
        self.buffer_packets = Some(n);
        self
    }

    /// Set the AV1 PES carriage mode. Default is
    /// [`Av1CarriageMode::Mpeg2TsBinding`] (spec-conformant per the
    /// AV1-in-MPEG-2-TS binding). Set to [`Av1CarriageMode::InteropRawObu`]
    /// for ffmpeg/libaom/hls.js interop carriage. See [`Av1CarriageMode`]
    /// for the carriage-shape differences.
    pub fn av1_carriage(&mut self, mode: Av1CarriageMode) -> &mut Self {
        self.av1_carriage = Some(mode);
        self
    }

    /// Finalize. Returns a validated [`MuxerConfig`] or an error describing
    /// the failed rule.
    ///
    /// Takes `&self` (not `self`) so the builder can be reused; clones inner
    /// state into the returned `MuxerConfig`. Cloning is cheap for typical
    /// configs (≤16 programs × ≤16 streams each).
    ///
    /// # Errors
    /// Returns the first error from [`MuxerConfig::validate`].
    pub fn build(&self) -> Result<MuxerConfig, MuxError> {
        let cfg = MuxerConfig {
            programs: self.programs.clone(),
            pcr_interval_ms: self.pcr_interval_ms.unwrap_or(40),
            psi_interval_ms: self.psi_interval_ms.unwrap_or(100),
            buffer_packets: self.buffer_packets.unwrap_or(10_000),
            av1_carriage: self.av1_carriage.unwrap_or_default(),
        };
        cfg.validate()?;
        Ok(cfg)
    }
}

/// Standalone builder for one [`MuxerProgramConfig`].
///
/// Build a program independently of the outer [`MuxerConfigBuilder`], then
/// hand the result to [`MuxerConfigBuilder::add_program`]. All mutators take
/// `&mut self` and return `&mut Self` for consistency with the outer builder
/// and clean FFI-binding semantics. `build()` takes `&self` and clones, so
/// the same builder can produce multiple programs.
///
/// # Example — gimbaled-platform program with EO + IR + sync KLV
/// ```
/// use tst_core::mpegts::mux::{
///     KlvStreamType, MuxerConfig, MuxerProgramConfigBuilder, VideoCodec,
/// };
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Two video streams (EO + IR sensors on the same gimbal) plus one sync
/// // KLV metadata stream sharing the platform's pose / FOV / pointing data.
/// let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
/// prog.add_video(0x1011, VideoCodec::H265); // EO (visible)
/// prog.add_video(0x1012, VideoCodec::H265); // IR (thermal)
/// prog.add_klv(0x1031, KlvStreamType::SynchronousMetadata, true);
///
/// let mut b = MuxerConfig::builder();
/// b.add_program(prog.build());
/// let config = b.build()?;
/// assert_eq!(config.programs[0].streams.len(), 3);
/// # Ok(())
/// # }
/// ```
#[must_use]
#[derive(Debug)]
pub struct MuxerProgramConfigBuilder {
    program: MuxerProgramConfig,
}

impl MuxerProgramConfigBuilder {
    /// Begin a new program block. `program_number` must be > 0 (program 0
    /// is reserved for network information) and unique within the outer
    /// [`MuxerConfig`]. `pmt_pid` carries this program's PMT and must not
    /// collide with any other program's PMT or any stream PID.
    pub fn new(program_number: u16, pmt_pid: u16) -> Self {
        Self {
            program: MuxerProgramConfig {
                program_number,
                pmt_pid,
                streams: Vec::new(),
                pcr_pid: None,
                program_descriptors: Vec::new(),
                stream_descriptors: Vec::new(),
            },
        }
    }

    /// Add a video elementary stream to this program.
    pub fn add_video(&mut self, pid: u16, codec: VideoCodec) -> &mut Self {
        self.program.streams.push(StreamSpec::Video { pid, codec });
        self.program.stream_descriptors.push(Vec::new());
        self
    }

    /// Add a KLV metadata elementary stream to this program.
    pub fn add_klv(
        &mut self,
        pid: u16,
        stream_type: KlvStreamType,
        carries_pts: bool,
    ) -> &mut Self {
        self.program.streams.push(StreamSpec::Klv {
            pid,
            stream_type,
            carries_pts,
        });
        self.program.stream_descriptors.push(Vec::new());
        self
    }

    /// Add an audio elementary stream to this program.
    ///
    /// `pid` must be in `0x0010..=0x1FFE` and distinct from all other PIDs in
    /// this program. `codec` drives the PMT `stream_type` byte.
    pub fn add_audio(&mut self, pid: u16, codec: AudioCodec) -> &mut Self {
        self.program.streams.push(StreamSpec::Audio {
            pid,
            codec,
            language: None,
        });
        self.program.stream_descriptors.push(Vec::new());
        self
    }

    /// Like [`add_audio`][Self::add_audio] but emits an
    /// `iso_639_language_descriptor` (ISO/IEC 13818-1 §2.6.18) on the PMT
    /// entry. Three-byte ISO 639-2 language code, lowercase ASCII
    /// (e.g. `*b"eng"`, `*b"deu"`, `*b"jpn"`). `audio_type` is set to
    /// `0x00` (undefined / clean main) per §2.6.19 Table 2-83.
    ///
    /// For richer audio_type semantics or multi-language tracks, supply
    /// the descriptor manually via
    /// [`stream_descriptors_for_audio`][Self::stream_descriptors_for_audio].
    pub fn add_audio_with_language(
        &mut self,
        pid: u16,
        codec: AudioCodec,
        language: [u8; 3],
    ) -> &mut Self {
        self.program.streams.push(StreamSpec::Audio {
            pid,
            codec,
            language: Some(language),
        });
        self.program.stream_descriptors.push(Vec::new());
        self
    }

    /// Add a subtitle elementary stream to this program.
    ///
    /// `pid` must be in `0x0010..=0x1FFE` and distinct from all other PIDs
    /// in this program. All four `SubtitleCodec` variants emit PMT
    /// `stream_type = 0x06` (PrivateData); the per-stream PMT descriptor
    /// disambiguates the codec at the wire level.
    pub fn add_subtitle(&mut self, pid: u16, codec: SubtitleCodec) -> &mut Self {
        self.program
            .streams
            .push(StreamSpec::Subtitle { pid, codec });
        self.program.stream_descriptors.push(Vec::new());
        self
    }

    /// Add an arbitrary private/application data elementary stream to this
    /// program (PES pass-through, the write-side dual of demux
    /// `StreamKind::Unknown`).
    ///
    /// `pid` must be in `0x0010..=0x1FFE` and distinct from all other PIDs
    /// in this program. `stream_type` is the raw PMT stream_type byte
    /// (e.g. 0xF0/0xF1 user-private, bare 0x06); `carries_pts` controls
    /// whether the PES header carries a PTS. The muxer never auto-emits a
    /// descriptor on a data stream — supply caller descriptors via
    /// [`stream_descriptors_for_data`][Self::stream_descriptors_for_data].
    ///
    /// The `(stream_type, descriptors)` pair must classify as Unknown on
    /// the demux side (no typed stream_type bytes, no classifying 0x06
    /// markers); the rule is enforced at validate/build time.
    pub fn add_data(&mut self, pid: u16, stream_type: u8, carries_pts: bool) -> &mut Self {
        self.program.streams.push(StreamSpec::Data {
            pid,
            stream_type,
            carries_pts,
        });
        self.program.stream_descriptors.push(Vec::new());
        self
    }

    /// Pin this program's PCR to a specific PID. Default: first video stream's
    /// PID (or first KLV PID for KLV-only programs).
    pub fn pcr_pid(&mut self, pid: u16) -> &mut Self {
        self.program.pcr_pid = Some(pid);
        self
    }

    /// Set program-level descriptors (PMT program info loop, before per-stream
    /// entries). Each `Vec<u8>` is one complete descriptor TLV.
    pub fn program_descriptors(&mut self, descs: Vec<Vec<u8>>) -> &mut Self {
        self.program.program_descriptors = descs;
        self
    }

    /// Set the descriptor list for the `video_idx`-th video stream in this
    /// program (zero-indexed among `StreamSpec::Video` entries in add-order).
    ///
    /// # Errors
    /// [`MuxError::DescriptorIndexOutOfRange`](crate::error::MuxError::DescriptorIndexOutOfRange) when `video_idx` is past the
    /// number of `add_video` calls so far. Call after the corresponding
    /// [`add_video`][Self::add_video].
    pub fn stream_descriptors_for_video(
        &mut self,
        video_idx: usize,
        descs: Vec<Vec<u8>>,
    ) -> Result<&mut Self, MuxError> {
        let abs_idx = self
            .program
            .streams
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, StreamSpec::Video { .. }))
            .nth(video_idx)
            .map(|(i, _)| i);
        match abs_idx {
            Some(i) => {
                self.program.stream_descriptors[i] = descs;
                Ok(self)
            }
            None => Err(MuxError::DescriptorIndexOutOfRange {
                kind: StreamKind::Video,
                index: video_idx as u32,
                program_number: self.program.program_number,
            }),
        }
    }

    /// Set the descriptor list for the `klv_idx`-th KLV stream in this
    /// program (zero-indexed among `StreamSpec::Klv` entries in add-order).
    ///
    /// # Errors
    /// [`MuxError::DescriptorIndexOutOfRange`](crate::error::MuxError::DescriptorIndexOutOfRange) when `klv_idx` is past the
    /// number of `add_klv` calls so far. Call after the corresponding
    /// [`add_klv`][Self::add_klv].
    pub fn stream_descriptors_for_klv(
        &mut self,
        klv_idx: usize,
        descs: Vec<Vec<u8>>,
    ) -> Result<&mut Self, MuxError> {
        let abs_idx = self
            .program
            .streams
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, StreamSpec::Klv { .. }))
            .nth(klv_idx)
            .map(|(i, _)| i);
        match abs_idx {
            Some(i) => {
                self.program.stream_descriptors[i] = descs;
                Ok(self)
            }
            None => Err(MuxError::DescriptorIndexOutOfRange {
                kind: StreamKind::Klv,
                index: klv_idx as u32,
                program_number: self.program.program_number,
            }),
        }
    }

    /// Set the descriptor list for the `audio_idx`-th audio stream in this
    /// program (zero-indexed among `StreamSpec::Audio` entries in add-order).
    ///
    /// # Errors
    /// [`MuxError::DescriptorIndexOutOfRange`](crate::error::MuxError::DescriptorIndexOutOfRange) when `audio_idx` is past the
    /// number of `add_audio` calls so far. Call after the corresponding
    /// [`add_audio`][Self::add_audio].
    pub fn stream_descriptors_for_audio(
        &mut self,
        audio_idx: usize,
        descs: Vec<Vec<u8>>,
    ) -> Result<&mut Self, MuxError> {
        let abs_idx = self
            .program
            .streams
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, StreamSpec::Audio { .. }))
            .nth(audio_idx)
            .map(|(i, _)| i);
        match abs_idx {
            Some(i) => {
                self.program.stream_descriptors[i] = descs;
                Ok(self)
            }
            None => Err(MuxError::DescriptorIndexOutOfRange {
                kind: StreamKind::Audio,
                index: audio_idx as u32,
                program_number: self.program.program_number,
            }),
        }
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
    /// # Errors
    /// [`MuxError::DescriptorIndexOutOfRange`](crate::error::MuxError::DescriptorIndexOutOfRange) when `subtitle_idx` is past
    /// the number of `add_subtitle` calls so far. Call after the
    /// corresponding [`add_subtitle`][Self::add_subtitle].
    pub fn stream_descriptors_for_subtitle(
        &mut self,
        subtitle_idx: usize,
        descs: Vec<Vec<u8>>,
    ) -> Result<&mut Self, MuxError> {
        let abs_idx = self
            .program
            .streams
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, StreamSpec::Subtitle { .. }))
            .nth(subtitle_idx)
            .map(|(i, _)| i);
        match abs_idx {
            Some(i) => {
                self.program.stream_descriptors[i] = descs;
                Ok(self)
            }
            None => Err(MuxError::DescriptorIndexOutOfRange {
                kind: StreamKind::Subtitle,
                index: subtitle_idx as u32,
                program_number: self.program.program_number,
            }),
        }
    }

    /// Set the descriptor list for the `data_idx`-th data stream in this
    /// program (zero-indexed among `StreamSpec::Data` entries in add-order).
    ///
    /// Data streams have no auto-emit — these caller descriptors are the
    /// entire PMT descriptor loop for the stream.
    ///
    /// # Errors
    /// [`MuxError::DescriptorIndexOutOfRange`](crate::error::MuxError::DescriptorIndexOutOfRange) when `data_idx` is past the
    /// number of `add_data` calls so far. Call after the corresponding
    /// [`add_data`][Self::add_data].
    pub fn stream_descriptors_for_data(
        &mut self,
        data_idx: usize,
        descs: Vec<Vec<u8>>,
    ) -> Result<&mut Self, MuxError> {
        let abs_idx = self
            .program
            .streams
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, StreamSpec::Data { .. }))
            .nth(data_idx)
            .map(|(i, _)| i);
        match abs_idx {
            Some(i) => {
                self.program.stream_descriptors[i] = descs;
                Ok(self)
            }
            None => Err(MuxError::DescriptorIndexOutOfRange {
                kind: StreamKind::Data,
                index: data_idx as u32,
                program_number: self.program.program_number,
            }),
        }
    }

    /// Set the descriptor list for a stream by absolute index within this
    /// program (across all streams in add-order).
    ///
    /// # Errors
    /// [`MuxError::AbsIndexOutOfRange`](crate::error::MuxError::AbsIndexOutOfRange) when `abs_idx` is past the number
    /// of streams added so far.
    pub fn stream_descriptors_for_stream(
        &mut self,
        abs_idx: usize,
        descs: Vec<Vec<u8>>,
    ) -> Result<&mut Self, MuxError> {
        if abs_idx < self.program.streams.len() {
            self.program.stream_descriptors[abs_idx] = descs;
            Ok(self)
        } else {
            Err(MuxError::AbsIndexOutOfRange {
                abs_idx: abs_idx as u32,
                len: self.program.streams.len() as u32,
                program_number: self.program.program_number,
            })
        }
    }

    /// Finalize the program. Returns a [`MuxerProgramConfig`] ready to hand
    /// to [`MuxerConfigBuilder::add_program`].
    ///
    /// Takes `&self` and clones; the same builder can produce multiple
    /// programs (e.g. by mutating a baseline shape between `build()` calls).
    /// Cross-program validation (PID collisions, etc.) runs at
    /// [`MuxerConfigBuilder::build`] time.
    pub fn build(&self) -> MuxerProgramConfig {
        self.program.clone()
    }
}

/// First ISO 639 language descriptor (tag 0x0A) → 3-byte code, only when
/// it looks like a valid lowercase ISO 639-2 code; None otherwise (the
/// caller falls back to language-less audio rather than erroring).
fn iso639_language(descs: &[crate::mpegts::descriptors::RawDescriptor]) -> Option<[u8; 3]> {
    let d = descs.iter().find(|d| d.tag == 0x0A)?;
    let code: [u8; 3] = d.data.get(..3)?.try_into().ok()?;
    code.iter().all(|b| b.is_ascii_lowercase()).then_some(code)
}

/// ISO 639-2 language codes per ETSI EN 300 468 §6.2.41/§6.2.43 ride the
/// wire as 3 ISO/IEC 8859-1 bytes. Spec doesn't mandate lowercase; we
/// accept uppercase or lowercase ASCII letters but reject non-alphabetic
/// bytes (digits, symbols, control codes) to keep junk out.
pub(super) fn validate_language_code(code: [u8; 3]) -> Result<(), MuxError> {
    if code.iter().all(|&b| b.is_ascii_alphabetic()) {
        Ok(())
    } else {
        Err(MuxError::InvalidLanguageCode { code })
    }
}
