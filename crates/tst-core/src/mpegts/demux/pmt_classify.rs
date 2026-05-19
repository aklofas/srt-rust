//! PMT stream classification: descriptor-driven derivation of `StreamKind`
//! from PMT entries, plus utility functions for stream-type-byte mapping
//! and descriptor recognition.
//!
//! Hosts 2 helper methods on `Demuxer` (`derive_stream_kind`,
//! `get_stream_kind`) and 7 module-level free functions consumed across
//! the demux module tree. All items are `pub(super)` — invisible outside
//! `mpegts::demux`.
//!
//! Per Wave 6.B Decision DB6, free functions stay as free functions (not
//! wrapped in a struct). The audit's recommended `stream_classifier.rs`
//! shape is collapsed into this module — narrower scope (PMT-specific).

use crate::mpegts::demux::event::{AudioCodec, NalUnit, StreamKind, SubtitleCodec, VideoCodec};
use crate::mpegts::demux::psi::{
    classify_audio_stream_type, extract_metadata_link, has_klva_registration,
};
use crate::mpegts::descriptors::RawDescriptor;

impl super::demuxer::Demuxer {
    pub(super) fn derive_stream_kind(
        &self,
        s: &crate::mpegts::demux::psi::PmtStream,
    ) -> (StreamKind, Option<u16>) {
        let declared_link = extract_metadata_link(&s.descriptors);
        let kind = match s.stream_type {
            0x1B => StreamKind::Video(VideoCodec::H264),
            0x24 => StreamKind::Video(VideoCodec::H265),
            0x33 => StreamKind::Video(VideoCodec::H266),
            0x06 => classify_0x06(&s.descriptors),
            0x15 => StreamKind::KlvSync { declared_link },
            other => {
                if let Some(codec) = classify_audio_stream_type(other) {
                    StreamKind::Audio(codec)
                } else {
                    StreamKind::Unknown(other)
                }
            }
        };
        (kind, declared_link)
    }

    pub(super) fn get_stream_kind(
        &self,
        pid: u16,
        s: &crate::mpegts::demux::psi::PmtStream,
    ) -> (StreamKind, Option<u16>) {
        // Caller override wins over PMT classification.
        if let Some(&kind) = self.options.stream_kind_overrides.get(&pid) {
            let declared_link = extract_metadata_link(&s.descriptors);
            (kind, declared_link)
        } else {
            self.derive_stream_kind(s)
        }
    }
}

// ─── Free functions ────────────────────────────────────────────────────────

/// Extract the `metadata_descriptor` declared link for a specific PID from
/// a parsed PMT. Used by `build_program_map` to rebuild klv_links after
/// collision filtering has already reduced the stream list.
pub(super) fn extract_metadata_link_for_pid(
    pmt: &crate::mpegts::demux::psi::Pmt,
    pid: u16,
) -> Option<u16> {
    pmt.streams
        .iter()
        .find(|s| s.elementary_pid == pid)
        .and_then(|s| extract_metadata_link(&s.descriptors))
}

/// Compute the total payload byte count for a slice of NAL units.
pub(super) fn nal_payload_bytes(nals: &[NalUnit]) -> usize {
    nals.iter()
        .map(|n| match n {
            NalUnit::H264 { payload, .. }
            | NalUnit::H265 { payload, .. }
            | NalUnit::H266 { payload, .. } => payload.len(),
        })
        .sum()
}

/// Map a `StreamKind` to its MPEG-TS `stream_type` byte (PMT value).
///
/// Used for `StreamStats.stream_type` labelling on the receiver side; not
/// emitted on the wire (the demuxer reads stream_type from the PMT). See
/// `mpegts::common::StreamType` for the canonical mux-side encoding.
pub(super) fn stream_type_from_kind(k: &StreamKind) -> u8 {
    match k {
        StreamKind::Video(VideoCodec::H264) => 0x1B,
        StreamKind::Video(VideoCodec::H265) => 0x24,
        StreamKind::Video(VideoCodec::H266) => 0x33,
        // AV1 rides stream_type 0x06 (PES private data) plus an AV01
        // registration_descriptor in the PMT.
        StreamKind::Video(VideoCodec::Av1) => 0x06,
        StreamKind::Audio(AudioCodec::Mp2) => 0x03,
        StreamKind::Audio(AudioCodec::Aac) => 0x0F,
        StreamKind::Audio(AudioCodec::AacLatm) => 0x11,
        StreamKind::Audio(AudioCodec::Ac3) => 0x81,
        StreamKind::Subtitle(_) => 0x06,
        StreamKind::KlvSync { .. } => 0x15,
        StreamKind::KlvAsync => 0x06,
        StreamKind::Unknown(t) => *t,
    }
}

/// Classify a stream_type 0x06 ("PES private data") by inspecting its
/// PMT-stream descriptors. Subtitle-disambiguating tags take priority
/// over the existing KLV registration check; if no subtitle descriptor
/// is present the result is identical to the prior behavior.
///
/// Priority (most-specific first):
///   1. `subtitling_descriptor` (tag 0x59, ETSI EN 300 468) → DVB subtitling.
///   2. `teletext_descriptor` (tag 0x56) or `VBI_teletext_descriptor`
///      (tag 0x46) → DVB teletext.
///   3. `registration_descriptor` (tag 0x05) format_identifier `"VTTC"` →
///      WebVTT-in-MPEG-TS.
///   4. `registration_descriptor` format_identifier `"GA94"` → CEA-708
///      standalone.
///   5. `registration_descriptor` format_identifier `"KLVA"` → asynchronous
///      MISB KLV (existing behavior).
///   6. Otherwise → `StreamKind::Unknown(0x06)`.
pub(super) fn classify_0x06(descriptors: &[RawDescriptor]) -> StreamKind {
    use crate::mpegts::descriptors::{find_descriptor_tag, find_format_identifier};
    // AV1 in MPEG-2 TS binding §2.1: format_identifier = "AV01".
    // AV01 registration is exclusive — wins over any other descriptor.
    if find_format_identifier(descriptors, b"AV01") {
        return StreamKind::Video(VideoCodec::Av1);
    }
    if find_descriptor_tag(descriptors, 0x59) {
        StreamKind::Subtitle(SubtitleCodec::DvbSubtitling)
    } else if find_descriptor_tag(descriptors, 0x56) || find_descriptor_tag(descriptors, 0x46) {
        StreamKind::Subtitle(SubtitleCodec::DvbTeletext)
    } else if find_format_identifier(descriptors, b"VTTC") {
        StreamKind::Subtitle(SubtitleCodec::WebVttInTs)
    } else if find_format_identifier(descriptors, b"GA94") {
        StreamKind::Subtitle(SubtitleCodec::Cea708Standalone)
    } else if has_klva_registration(descriptors) {
        StreamKind::KlvAsync
    } else {
        StreamKind::Unknown(0x06)
    }
}

/// Same as [`classify_0x06`] but also returns the list of recognized
/// subtitle/KLV codec markers found on the PID — empty if there's no
/// ambiguity (zero or one marker), populated if more than one was found.
///
/// Tag list encoding mirrors [`crate::mpegts::demux::event::NonConformantIssue::SubtitleDescriptorAmbiguous`]:
/// descriptor tag bytes for tag-presence matches (0x59 / 0x56 / 0x46),
/// synthetic codepoints for `format_identifier` matches (0xF0=VTTC,
/// 0xF1=GA94, 0xF2=KLVA). The classification result follows the existing
/// first-match priority — only the diagnostic tag list changes.
pub(super) fn classify_0x06_with_ambiguity(descriptors: &[RawDescriptor]) -> (StreamKind, Vec<u8>) {
    use crate::mpegts::descriptors::{find_descriptor_tag, find_format_identifier};
    let mut markers: Vec<u8> = Vec::new();
    if find_descriptor_tag(descriptors, 0x59) {
        markers.push(0x59);
    }
    // 0x56 and 0x46 are sibling teletext tags — count as one marker so
    // a stream carrying both doesn't trip ambiguity on the teletext side.
    if find_descriptor_tag(descriptors, 0x56) {
        markers.push(0x56);
    } else if find_descriptor_tag(descriptors, 0x46) {
        markers.push(0x46);
    }
    if find_format_identifier(descriptors, b"VTTC") {
        markers.push(0xF0);
    }
    if find_format_identifier(descriptors, b"GA94") {
        markers.push(0xF1);
    }
    if find_format_identifier(descriptors, b"KLVA") {
        markers.push(0xF2);
    }
    let kind = classify_0x06(descriptors);
    let ambiguous = if markers.len() <= 1 {
        Vec::new()
    } else {
        markers
    };
    (kind, ambiguous)
}

/// True iff `descriptors` contains any descriptor that lets the demuxer
/// recognize this stream as a subtitle/caption track:
///   * `subtitling_descriptor`  (tag 0x59)
///   * `teletext_descriptor`    (tag 0x56)
///   * `VBI_teletext_descriptor`(tag 0x46)
///   * `registration_descriptor` with format_identifier `"VTTC"` or `"GA94"`.
///
/// Used by the PMT classifier to surface `SubtitleMissingDescriptor`
/// when a `treat_as` override (or any other path) routes a PID to
/// `StreamKind::Subtitle(_)` but the PMT entry has none of the above.
pub(super) fn has_recognized_subtitle_descriptor(descriptors: &[RawDescriptor]) -> bool {
    use crate::mpegts::descriptors::{find_descriptor_tag, find_format_identifier};
    find_descriptor_tag(descriptors, 0x59)
        || find_descriptor_tag(descriptors, 0x56)
        || find_descriptor_tag(descriptors, 0x46)
        || find_format_identifier(descriptors, b"VTTC")
        || find_format_identifier(descriptors, b"GA94")
}

/// True iff `descriptors` contains a Registration descriptor that
/// LOOKS like an attempted AV1 (`AV01`) registration but is truncated.
/// Specifically: a descriptor with `tag == 0x05`, body length < 4 bytes,
/// and body starts with `b"AV"`. Outer length-vs-buffer overflow would
/// already error via `PsiParseError::DescriptorLoopOverflow` at walk
/// time; this catches the subtler case where the descriptor is
/// well-formed but its body can't be a valid 4-byte format_identifier.
///
/// Used by the demuxer to surface `NonConformantIssue::Av1RegistrationMalformed`
/// from the PMT processing path. Lenient mode silently still falls
/// through to `StreamKind::Unknown(0x06)` from the standard cascade;
/// strict mode (`StrictMode::Full`) converts the issue to a fatal
/// `DemuxError::StrictRejection`.
pub(super) fn is_malformed_av1_registration(descriptors: &[RawDescriptor]) -> bool {
    descriptors
        .iter()
        .any(|d| d.tag == 0x05 && d.data.len() < 4 && d.data.starts_with(b"AV"))
}
