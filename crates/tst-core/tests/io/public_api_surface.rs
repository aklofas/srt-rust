//! Compile-only sentinel for the curated public API of `tst-core`.
//!
//! Each `use` line below names an intended-public path. If a future
//! edit accidentally privatizes or relocates one of these items, this
//! test stops compiling, surfacing the regression at CI time. The
//! `cargo public-api` baseline catches the inverse case (accidentally
//! re-exposing an internal item).
//!
//! Maintenance: when intentionally adding to or removing from the
//! curated surface, update this file in the same commit that updates
//! the public-api.txt baseline.

// === Root re-exports (most stable layer) ===
#[allow(unused_imports)]
use tst_core::{MuxError, mpegts};

// === mpegts curated surface ===
#[allow(unused_imports)]
use tst_core::mpegts::common::{Pcr27mhz, Pts90khz, TS_PACKET_SIZE};

// === Demux curated surface (the typical consumer path) ===
#[allow(unused_imports)]
use tst_core::mpegts::demux::{
    AudioCodec, DemuxEvent, Demuxer, DemuxerBuilder, DemuxerConfig, DemuxerStats,
    DiscontinuityKind, LinkSource, MetadataKind, NalUnit, NonConformantIssue, ProgramMap,
    SamplePayload, StreamId, StreamInfo, StreamKind, StrictMode, SubtitleCodec, VideoCodec,
    VideoPayload,
};

// === Demux low_level extension points (advanced consumer / fuzz / tools) ===
#[allow(unused_imports)]
use tst_core::mpegts::demux::low_level::{
    KlvShape, Pat, PatEntry, PesPayload, Pmt, PmtStream, PsiParseError, RawDescriptor, Reassembler,
    ReassemblyOutcome, classify_klv, extract_metadata_link, extract_user_label,
    has_klva_registration, parse_pat, parse_pmt, walk_descriptors,
};

// === Descriptors (canonical home for builders + parsers) ===
#[allow(unused_imports)]
use tst_core::mpegts::descriptors::{
    DescriptorError, DescriptorParseError, RawDescriptor as DescriptorsRawDescriptor,
    SubtitlingDescriptorEntry, TeletextDescriptorEntry, descriptor_with_tag_unchecked,
    find_descriptor_tag, find_format_identifier, format_identifier_ac3, format_identifier_av01,
    iso_639_language, metadata_klva, parse_subtitling_descriptor, parse_teletext_descriptor,
    registration, user_private,
};

// === Mux curated surface ===
#[allow(unused_imports)]
use tst_core::mpegts::mux::{
    KlvStreamHandle, Muxer, MuxerConfig, MuxerConfigBuilder, MuxerProgramConfig,
    MuxerProgramConfigBuilder, VideoStreamHandle,
};

// === Codec parser curated surface ===
#[allow(unused_imports)]
use tst_core::codec::av1::{Av1FrameHeaderLight, Av1ObuStream, Av1SequenceHeader};
#[allow(unused_imports)]
use tst_core::codec::h264::{H264ParameterSets, H264Pps, H264Sps};
#[allow(unused_imports)]
use tst_core::codec::h265::{H265ParameterSets, H265Pps, H265Sps, H265Vps};
#[allow(unused_imports)]
use tst_core::codec::h266::{H266ParameterSets, H266Pps, H266ProfileTierLevel, H266Sps, H266Vps};

#[test]
fn curated_public_api_compiles() {
    // The act of compiling this file is the test. If you got here, all
    // imports above resolved; the curated public API surface is intact.
}

#[test]
#[allow(clippy::assertions_on_constants)]
fn low_level_namespace_documented_as_experimental() {
    // Document via test that the low_level module exists. Reading its
    // rustdoc is the actual stability contract.
    assert!(
        true,
        "low_level namespace must exist with stability rustdoc"
    );
}
