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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    /// Reserved variant — no typed audio codec yet. The presence of this
    /// enum is the surface guarantee that adding e.g. `Aac` later is
    /// additive, not a breaking change.
    #[doc(hidden)]
    __Reserved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubtitleCodec {
    #[doc(hidden)]
    __Reserved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SamplePayload {
    Video {
        codec: VideoCodec,
        nals: Vec<NalUnit>,
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
}
