// crates/srt-core/src/mpegts/demux/event.rs
//! Public event types emitted by `Demuxer`.
//!
//! Independent of the demuxer's internal state — these are the types
//! consumers match on. Adding a future variant (audio codec, subtitle
//! codec, AV1 codec) is additive: add the variant to the appropriate
//! enum, no other public type changes.

use std::time::Duration;

/// Top-level event emitted by `Demuxer::next_event`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemuxEvent {
    /// PSI topology — emitted on PAT/PMT discovery and on each PSI
    /// version-bump. Carries declared linkage info with provenance.
    ProgramMap(ProgramMap),

    /// Generic elementary-stream sample. Covers video today; reserves
    /// shape for audio, subtitles, and future PES-carried ES types via
    /// additive variants on `SamplePayload`.
    Sample {
        stream: StreamId,
        pts: i64,
        dts: Option<i64>,
        payload: SamplePayload,
    },

    /// Standalone metadata — KLV (sync or async), or any future
    /// metadata-stream pattern.
    Metadata {
        stream: StreamId,
        pts: i64,
        kind: MetadataKind,
        payload: Vec<u8>,
    },

    /// Continuity discontinuity on a specific PID.
    Discontinuity {
        stream: StreamId,
        kind: DiscontinuityKind,
    },

    /// Non-conformance detected; demuxer continued anyway with a
    /// best-effort interpretation. In `StrictMode::Off` (default) this
    /// surfaces as an event; in stricter modes it converts to
    /// `DemuxError::StrictRejection`.
    NonConformant {
        stream: StreamId,
        issue: NonConformantIssue,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamId {
    pub pid: u16,
    pub kind: StreamKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamKind {
    Video(VideoCodec),
    Audio(AudioCodec),
    Subtitle(SubtitleCodec),
    KlvSync { declared_link: Option<u16> },
    KlvAsync,
    Unknown(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoCodec {
    H264,
    H265,
    /// H.266 / VVC (ITU-T H.266 V4). PMT stream_type = 0x33.
    H266,
    /// AV1 (AOM Bitstream Spec). PMT stream_type = 0x06 with
    /// `registration_descriptor` `format_identifier = "AV01"`.
    Av1,
}

/// Audio codec carried in `SamplePayload::Audio`. Identifies the codec
/// for typed dispatch but does not parse the bitstream — `frames` holds
/// the raw PES payload bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    Mp2,
    Aac,
    AacLatm,
    Ac3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubtitleCodec {
    /// DVB subtitling (bitmap-shaped). Per ETSI EN 300 468 §6.2.41 +
    /// ETSI EN 300 743. Per-stream params (language, page IDs)
    /// surface on `StreamInfo::raw_descriptors`; decode lazily via
    /// `parse_subtitling_descriptor`.
    DvbSubtitling,
    /// DVB teletext. Per ETSI EN 300 468 §6.2.43 + ETSI EN 300 706.
    /// Per-stream params (language, magazine, page) surface on
    /// `StreamInfo::raw_descriptors`; decode lazily via
    /// `parse_teletext_descriptor`.
    DvbTeletext,
    /// CEA-708 caption data carried as a separate elementary stream
    /// (rather than embedded in H.264/H.265 SEI). Marked via
    /// registration_descriptor format_identifier "GA94".
    Cea708Standalone,
    /// WebVTT cues carried inside MPEG-TS PES per Apple's HLS
    /// authoring spec. Marked via registration_descriptor
    /// format_identifier "VTTC".
    WebVttInTs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SamplePayload {
    Video {
        codec: VideoCodec,
        payload: VideoPayload,
    },
    Audio {
        codec: AudioCodec,
        frames: Vec<u8>,
    },
    Subtitle {
        codec: SubtitleCodec,
        payload: Vec<u8>,
    },
    Unknown {
        stream_type: u8,
        raw: Vec<u8>,
    },
}

/// Codec-specific bitstream payload shape.
///
/// `Nals` covers Annex-B NAL-shaped codecs (H.264, H.265, H.266).
/// `Obus` covers AV1's Open Bitstream Unit format. The variant is
/// determined by [`VideoCodec`] on the parent [`SamplePayload::Video`]
/// event:
///
/// * `codec ∈ {H264, H265, H266}` ⇒ `payload = Nals(_)`
/// * `codec = Av1`                 ⇒ `payload = Obus(_)`
///
/// The demuxer enforces this invariant by construction; consumers
/// can match on `payload` after dispatching on `codec`, or vice versa,
/// and assume the mapping holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoPayload {
    /// H.264, H.265, H.266 — Annex-B NAL-shaped.
    Nals(Vec<NalUnit>),
    /// AV1 — OBU-shaped (Open Bitstream Unit).
    Obus(Vec<Obu>),
}

/// One AV1 Open Bitstream Unit. Per AV1 Bitstream Spec §5.3.2.
///
/// The header byte (`obu_forbidden_bit | obu_type | obu_extension_flag |
/// obu_has_size_field | obu_reserved_1bit`) and any extension byte are
/// parsed during split; the LEB128 `obu_size` field is consumed and
/// stripped. `payload` carries only the OBU body bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Obu {
    /// 4-bit `obu_type` (AV1 §5.3.2). 1=SequenceHeader, 2=TemporalDelimiter,
    /// 3=FrameHeader, 4=TileGroup, 5=Metadata, 6=Frame,
    /// 7=RedundantFrameHeader, 8=TileList, 15=Padding.
    pub obu_type: u8,
    /// Optional extension header bytes when `obu_extension_flag = 1`.
    pub extension: Option<ObuExtension>,
    /// OBU payload bytes — header + extension byte + LEB128 size field
    /// stripped. Pass-through; parsed by `codec::av1::parse_*` if the
    /// consumer wants typed fields.
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ObuExtension {
    /// 3-bit `temporal_id` (AV1 §5.3.3).
    pub temporal_id: u8,
    /// 2-bit `spatial_id` (AV1 §5.3.3).
    pub spatial_id: u8,
}

/// One H.264 or H.265 NAL unit. Codec-tagged so wrapped languages
/// (Kotlin, Swift, Java) get idiomatic `when`/`switch` exhaustiveness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NalUnit {
    H264 {
        /// 5-bit `nal_unit_type` (H.264 §7.3.1).
        nal_type: u8,
        /// 2-bit `nal_ref_idc` (H.264 §7.3.1).
        ref_idc: u8,
        /// RBSP bytes; Annex-B start codes stripped, emulation prevention
        /// bytes preserved (consumer's decoder removes them).
        payload: Vec<u8>,
    },
    H265 {
        /// 6-bit `nal_unit_type` (H.265 §7.3.1.2).
        nal_type: u8,
        /// 6-bit `nuh_layer_id` (H.265 §7.3.1.2).
        layer_id: u8,
        /// 3-bit `nuh_temporal_id_plus1` (H.265 §7.3.1.2).
        temporal_id_plus1: u8,
        /// RBSP bytes; same stripping/preservation rules as `H264`.
        payload: Vec<u8>,
    },
    /// One H.266 / VVC NAL unit. Header parsed per H.266 V4 §7.3.1.2.
    ///
    /// Common `nal_type` values: VPS_NUT=14, SPS_NUT=15, PPS_NUT=16,
    /// PREFIX_APS_NUT=17, SUFFIX_APS_NUT=18, PH_NUT=19, AUD_NUT=20,
    /// IDR_W_RADL=7, IDR_N_LP=8, CRA_NUT=9, GDR_NUT=10. Full table at
    /// H.266 V4 Table 5.
    H266 {
        /// 5-bit `nal_unit_type` (H.266 V4 §7.3.1.2).
        nal_type: u8,
        /// 6-bit `nuh_layer_id` (H.266 V4 §7.3.1.2).
        layer_id: u8,
        /// 3-bit `nuh_temporal_id_plus1` (H.266 V4 §7.3.1.2).
        temporal_id_plus1: u8,
        /// RBSP bytes; Annex-B start codes stripped, emulation
        /// prevention preserved (consumer's decoder removes 0x03 escapes).
        payload: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataKind {
    /// ST 1910 AU cell wrapped KLV. The demuxer has unwrapped the AU cell;
    /// `payload` is the inner KLV LS ready to feed to `klv::st0601::decode`.
    /// `pts` on the parent event is the AU cell's metadata access unit
    /// timestamp (from `klv::st0605::PrecisionTimeStampPack`).
    KlvSyncAuCell,

    /// Bare KLV LS (no AU cell wrap). Async metadata, typically 1–10 Hz.
    /// `pts` on the parent event is the PES PTS.
    KlvAsync,

    /// Unrecognized metadata `stream_type`. `payload` is raw PES payload.
    Unknown(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgramMap {
    pub program_number: u16,
    pub pcr_pid: u16,
    pub streams: Vec<StreamInfo>,
    pub klv_links: Vec<KlvLink>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    pub pid: u16,
    pub stream_type: u8,
    pub kind: StreamKind,
    /// Program number from the PAT entry whose PMT owns this stream.
    /// Apps filtering `Sample`/`Metadata` events by program can build a
    /// `pid → program_number` map from `ProgramMap` events.
    pub program_number: u16,
    /// Raw PMT per-stream descriptors for this PID, in PMT loop order.
    /// Empty when the PMT carried no descriptors for this stream. Use
    /// [`crate::mpegts::demux::psi::extract_user_label`] for a quick
    /// label decode; reach into this list for vendor-specific or
    /// stack-shape (Family B) decoding.
    pub raw_descriptors: Vec<crate::mpegts::demux::psi::RawDescriptor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KlvLink {
    pub klv_pid: u16,
    pub video_pid: u16,
    pub source: LinkSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSource {
    /// `metadata_descriptor` in the PMT explicitly linked these PIDs.
    Declared,
    /// Demuxer inferred the link from topology (e.g., one video + one
    /// metadata PID with no descriptor). Treat as a hint, not authority.
    Inferred,
    /// Caller provided the link via `DemuxerBuilder::link_klv`.
    Override,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NonConformantIssue {
    /// `stream_type=0x06` carries ST 1910 AU cell payload; treated as sync KLV.
    StreamTypeMismatchSyncOnAsyncPid,
    /// `stream_type=0x15` carries bare KLV without AU cell wrap; treated as async KLV.
    StreamTypeMismatchAsyncOnSyncPid,
    /// `metadata_descriptor` missing on a sync-KLV-shaped PID.
    MissingMetadataDescriptor,
    /// PCR jump or stream-monotonic timing inconsistency.
    PcrAnomaly { delta: i64 },
    /// PSI section checksum mismatch. Lenient mode falls back to the
    /// previous PSI version; strict mode converts to error.
    PsiChecksumMismatch { pid: u16 },
    /// PUSI mid-PES — a new PUSI packet arrived before the previous PES
    /// completed. Lenient mode discards the partial PES and starts fresh.
    PusiMidPes,
    /// A PMT introduced a stream PID that's already bound to a different
    /// program. PID uniqueness across programs is required by ISO 13818-1;
    /// the demuxer keeps the first-program-wins binding and drops the second.
    PidReusedAcrossPrograms { pid: u16, programs: [u16; 2] },

    /// `treat_as` (or fallback) routed a PID to a subtitle codec but no
    /// recognized subtitle descriptor (subtitling/teletext/VTTC/GA94) is
    /// present on the PMT entry. Lenient mode classifies anyway; strict
    /// mode converts to `DemuxError::StrictRejection`.
    SubtitleMissingDescriptor { pid: u16 },

    /// Subtitle descriptor tag was recognized but the inner length /
    /// payload bytes did not satisfy spec invariants. Per-stream params
    /// fall back to defaults (language `*b"und"`, page ids 0).
    ///
    /// Reserved variant: not currently emitted by the demuxer. The
    /// classification cascade in [`Demuxer`](crate::mpegts::demux::Demuxer)
    /// is tag-presence-based via `find_descriptor_tag`, so malformed
    /// descriptor bodies pass through. Typed-parser integration in the
    /// cascade — and therefore this variant's first emission — is
    /// deferred to the typed WebVTT cue / DVB-sub data-segment /
    /// teletext data-unit substrate session.
    SubtitleDescriptorMalformed { pid: u16, tag: u8 },

    /// AV1 stream's PMT entry has a malformed `registration_descriptor`
    /// for `format_identifier "AV01"` — length byte mismatches payload.
    Av1RegistrationMalformed { pid: u16 },

    /// AV1 OBU encountered with `obu_has_size_field = 0`. AV1 in MPEG-2 TS
    /// binding §3.1 (linked through AV1 spec §5.2 "low overhead bitstream
    /// format") requires `=1`. Streams violating this can't be split
    /// reliably; the splitter places remaining bytes into one trailing
    /// `Obu` and stops walking.
    Av1ObuMissingSizeField { pid: u16, obu_type: u8 },

    /// Tile List OBU (`obu_type = 8`) encountered. Forbidden by AV1 in
    /// MPEG-2 TS binding §3.3. Lenient mode passes through; strict mode
    /// rejects.
    Av1TileListNotAllowed { pid: u16 },

    /// Other.
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscontinuityKind {
    /// Continuity counter jumped on this PID.
    ContinuityJump { expected: u8, observed: u8 },
    /// Per-PID PES reassembly buffer cap exceeded; partial PES dropped.
    PesOversize { pid: u16 },
    /// Aggregate PES-reassembly cap exceeded across all PIDs.
    PesTotalOversize,
    /// `discontinuity_indicator` set in the adaptation field.
    AdaptationFieldFlag,
}

/// Approximate "elapsed time since first event" for a stream-monotonic
/// PTS. Exposed for diagnostic / test use; production consumers usually
/// just compare `i64` PTS values directly.
pub fn pts_to_duration(pts_90khz: i64) -> Duration {
    Duration::from_micros((pts_90khz as i128 * 1_000_000 / 90_000) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nal_unit_h264_h265_distinct() {
        let h264 = NalUnit::H264 {
            nal_type: 5,
            ref_idc: 3,
            payload: vec![],
        };
        let h265 = NalUnit::H265 {
            nal_type: 19,
            layer_id: 0,
            temporal_id_plus1: 1,
            payload: vec![],
        };
        assert_ne!(h264, h265);
    }

    #[test]
    fn pts_to_duration_simple() {
        // 90,000 ticks @ 90 kHz = 1 second.
        assert_eq!(pts_to_duration(90_000), Duration::from_secs(1));
    }

    #[test]
    fn audio_codec_real_variants_in_demux_event_surface() {
        let codecs = [
            AudioCodec::Mp2,
            AudioCodec::Aac,
            AudioCodec::AacLatm,
            AudioCodec::Ac3,
        ];
        assert_ne!(codecs[0], codecs[1]);
    }

    #[test]
    fn demux_subtitle_codec_real_variants_in_event_surface() {
        let codecs = [
            SubtitleCodec::DvbSubtitling,
            SubtitleCodec::DvbTeletext,
            SubtitleCodec::Cea708Standalone,
            SubtitleCodec::WebVttInTs,
        ];
        assert_ne!(codecs[0], codecs[1]);
        assert_ne!(codecs[2], codecs[3]);
    }
}
