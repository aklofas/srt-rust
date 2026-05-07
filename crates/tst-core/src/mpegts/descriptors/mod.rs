//! Builders for MPEG-TS PMT per-stream descriptor TLVs.
//!
//! Each builder returns a `Vec<u8>` containing one complete descriptor
//! (`descriptor_tag` + `descriptor_length` + body). Hand the result to
//! [`crate::mpegts::mux::ConfigBuilder::stream_descriptors_for_video`]
//! / `_for_klv` / `_for_stream` to splice it into the per-stream
//! descriptor loop emitted in PMT.
//!
//! Reference: ISO/IEC 13818-1 §2.6 and ETSI EN 300 468 §6.2.

pub mod parse;
pub use parse::{
    ParseError, SubtitlingDescriptorEntry, TeletextDescriptorEntry, find_descriptor_tag,
    find_format_identifier, parse_subtitling_descriptor, parse_teletext_descriptor,
};

/// Errors returned by descriptor builder helpers in this module.
///
/// Empty `entries` arguments produce a degenerate `tag 0x00` descriptor
/// that the demux parser rejects with [`ParseError::EmptyInput`]. The
/// encoder rejects the same shape symmetrically rather than emitting
/// invalid PSI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DescriptorError {
    /// `entries` slice was empty for a multi-entry descriptor builder
    /// (subtitling 0x59 or teletext 0x56). Caller must supply at least
    /// one entry.
    EmptyEntries { tag: u8 },
}

impl core::fmt::Display for DescriptorError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DescriptorError::EmptyEntries { tag } => write!(
                f,
                "descriptor tag 0x{tag:02X}: entries slice is empty (must be non-empty)"
            ),
        }
    }
}

impl core::error::Error for DescriptorError {}

/// Registration descriptor (tag 0x05) — H.222.0 §2.6.8.
///
/// `format_identifier` is a 4-byte ASCII tag. `additional` is the
/// optional `additional_identification_info` payload (e.g. the 4-byte
/// `FF 1B 44 3F` trailer Haivision-shaped senders put after `"HDMV"`
/// on H.264 video PIDs).
///
/// Total body must be ≤ 251 bytes (`additional.len() ≤ 247`).
/// `debug_assert!` catches overflow in debug builds; release builds
/// silently clamp the trailing bytes (caller is responsible for
/// staying within the bound — invariant is not checked in release).
pub fn registration(format_identifier: [u8; 4], additional: &[u8]) -> Vec<u8> {
    let body_len = 4 + additional.len();
    debug_assert!(body_len <= 251, "registration descriptor body too large");
    let body_len = body_len.min(251);
    let mut out = Vec::with_capacity(2 + body_len);
    out.push(0x05);
    out.push(body_len as u8);
    out.extend_from_slice(&format_identifier);
    out.extend_from_slice(&additional[..body_len - 4]);
    out
}

/// Metadata descriptor (tag 0x26) — H.222.0 §2.6.58 — for KLV PIDs
/// carried as `stream_type=0x15` (Metadata in PES). Hard-coded to the
/// canonical KLVA shape:
///   metadata_application_format = 0x0100 (User defined)
///   metadata_format             = 0xFF (defined by format_identifier)
///   metadata_format_identifier  = 0x4B4C5641 ("KLVA")
///   metadata_service_id         = service_id (caller; 0 is universal)
///   decoder_config_flags + DSM_CC_flag = 0  (4 reserved bits = 1)
///
/// Matches the Family B (ARS) corpus byte-for-byte.
pub fn metadata_klva(service_id: u8) -> Vec<u8> {
    vec![
        0x26, 0x09, 0x01, 0x00, 0xFF, b'K', b'L', b'V', b'A', service_id, 0x0F,
    ]
}

/// Metadata STD descriptor (tag 0x27) — H.222.0 §2.6.62 — STD-buffer
/// dimensions for a Metadata-in-PES stream. All-zero values match the
/// "no rate-shaping declared" shape Family B uses.
///
/// Each rate field is 22 bits (top 2 bits reserved = `11`).
pub fn metadata_std(input_leak_rate: u32, buffer_size: u32, output_leak_rate: u32) -> Vec<u8> {
    fn pack_22(value: u32) -> [u8; 3] {
        let v = value & 0x003F_FFFF;
        [0xC0 | ((v >> 16) as u8 & 0x3F), (v >> 8) as u8, v as u8]
    }
    let i = pack_22(input_leak_rate);
    let b = pack_22(buffer_size);
    let o = pack_22(output_leak_rate);
    vec![
        0x27, 0x09, i[0], i[1], i[2], b[0], b[1], b[2], o[0], o[1], o[2],
    ]
}

/// User-private descriptor (tag 0xFF). ISO/IEC 13818-1 reserves tag
/// 0xFF, but real-world ARS-shape senders use it as the de-facto
/// per-stream label slot (e.g. `"VIDEO-ARS"`, `"KLV_SYNC"`,
/// `"JSONCMD"`).
///
/// `tst-core`'s demuxer surfaces it both via
/// [`crate::mpegts::demux::event::StreamInfo::raw_descriptors`] and (when
/// payload is valid UTF-8) via the demuxer-side stats label.
///
/// `payload` ≤ 255 bytes; not interpreted by this helper.
pub fn user_private(payload: &[u8]) -> Vec<u8> {
    user_private_with_tag(0xFF, payload)
}

/// Same as [`user_private`] but with a caller-chosen tag in the
/// user-private / reserved range (0x40..=0xFF). Use when emitting
/// vendor-defined slots that aren't tag 0xFF.
///
/// The `tag >= 0x40` invariant is enforced via `debug_assert!` and
/// is NOT checked in release builds — callers are responsible for
/// staying within the user-private range. Passing a reserved-by-spec
/// tag (e.g. `0x05` for Registration) will produce a malformed
/// descriptor with no error.
pub fn user_private_with_tag(tag: u8, payload: &[u8]) -> Vec<u8> {
    debug_assert!(tag >= 0x40, "user-private tags must be in 0x40..=0xFF");
    debug_assert!(
        payload.len() <= 255,
        "descriptor body must fit in u8 length"
    );
    let len = payload.len().min(255);
    let mut out = Vec::with_capacity(2 + len);
    out.push(tag);
    out.push(len as u8);
    out.extend_from_slice(&payload[..len]);
    out
}

/// Component descriptor (tag 0x50) — ETSI EN 300 468 §6.2.8.
/// Carries language-tagged free text. Not seen in the testfiles
/// corpus; included because it's the textbook "human label" slot
/// that [`crate::mpegts::demux::psi::extract_user_label`] reads first.
///
/// `text` is UTF-8; receivers conventionally treat the body as
/// language-coded per the descriptor's `iso_639_language_code` field.
pub fn component(
    stream_content: u8,
    component_type: u8,
    component_tag: u8,
    iso_639_language: [u8; 3],
    text: &str,
) -> Vec<u8> {
    let text_bytes = text.as_bytes();
    debug_assert!(
        text_bytes.len() <= 249,
        "component text too long for single descriptor"
    );
    let text_len = text_bytes.len().min(249);
    let body_len = 6 + text_len;
    let mut out = Vec::with_capacity(2 + body_len);
    out.push(0x50);
    out.push(body_len as u8);
    out.push(stream_content & 0x0F | 0xF0); // 4 reserved bits = 1, per ETSI EN 300 468 §6.2.8
    out.push(component_type);
    out.push(component_tag);
    out.extend_from_slice(&iso_639_language);
    out.extend_from_slice(&text_bytes[..text_len]);
    out
}

/// Stream Identifier descriptor (tag 0x52) — ETSI EN 300 468 §6.2.39.
/// Single `component_tag` byte; pairs with a Component descriptor for
/// the actual text.
pub fn stream_identifier(component_tag: u8) -> Vec<u8> {
    vec![0x52, 0x01, component_tag]
}

/// DVB subtitling_descriptor (tag 0x59), single-entry form.
/// ETSI EN 300 468 §6.2.41.
///
/// `language` is ISO 639-2 lowercase ASCII. `subtitling_type` per
/// Table 26 (e.g. 0x10 = DVB sub, no AR signalling). `composition_page_id`
/// and `ancillary_page_id` are 16-bit values per spec.
pub fn subtitling_descriptor(
    language: [u8; 3],
    subtitling_type: u8,
    composition_page_id: u16,
    ancillary_page_id: u16,
) -> Vec<u8> {
    // Single-entry input is statically non-empty, so the multi helper's
    // EmptyEntries branch is unreachable here.
    subtitling_descriptor_multi(&[(
        language,
        subtitling_type,
        composition_page_id,
        ancillary_page_id,
    )])
    .expect("single-entry slice is statically non-empty")
}

/// DVB subtitling_descriptor (tag 0x59), multi-entry form per
/// ETSI EN 300 468 §6.2.41. Each entry is `(language, subtitling_type,
/// composition_page_id, ancillary_page_id)`.
///
/// Use this for multi-language single-PID DVB subtitling services. The
/// single-entry helper [`subtitling_descriptor`] is a `len=1` shorthand.
///
/// Returns [`DescriptorError::EmptyEntries`] if `entries` is empty —
/// the demux parser rejects an empty subtitling_descriptor with
/// [`ParseError::EmptyInput`], so the encoder rejects symmetrically.
pub fn subtitling_descriptor_multi(
    entries: &[([u8; 3], u8, u16, u16)],
) -> Result<Vec<u8>, DescriptorError> {
    if entries.is_empty() {
        return Err(DescriptorError::EmptyEntries { tag: 0x59 });
    }
    let body_len = entries.len() * 8;
    debug_assert!(body_len <= u8::MAX as usize, "descriptor length is u8");
    let mut out = Vec::with_capacity(2 + body_len);
    out.push(0x59); // tag
    out.push(body_len as u8); // length
    for (language, subtitling_type, comp_page_id, anc_page_id) in entries {
        out.extend_from_slice(language);
        out.push(*subtitling_type);
        out.extend_from_slice(&comp_page_id.to_be_bytes());
        out.extend_from_slice(&anc_page_id.to_be_bytes());
    }
    Ok(out)
}

/// DVB teletext_descriptor (tag 0x56), single-entry form.
/// ETSI EN 300 468 §6.2.43.
///
/// `language` is ISO 639-2 lowercase ASCII. `teletext_type` is a 5-bit
/// value (e.g. 0x02 = subtitle page). `magazine_number` is 0..=7
/// (3 bits). `page_number` is BCD-encoded.
pub fn teletext_descriptor(
    language: [u8; 3],
    teletext_type: u8,
    magazine_number: u8,
    page_number: u8,
) -> Vec<u8> {
    // Single-entry input is statically non-empty, so the multi helper's
    // EmptyEntries branch is unreachable here.
    teletext_descriptor_multi(&[(language, teletext_type, magazine_number, page_number)])
        .expect("single-entry slice is statically non-empty")
}

/// DVB teletext_descriptor (tag 0x56), multi-entry form per
/// ETSI EN 300 468 §6.2.43. Each entry is `(language, teletext_type,
/// magazine_number, page_number)`.
///
/// Use this for multi-language single-PID DVB teletext services. The
/// single-entry helper [`teletext_descriptor`] is a `len=1` shorthand.
///
/// Returns [`DescriptorError::EmptyEntries`] if `entries` is empty —
/// the demux parser rejects an empty teletext_descriptor with
/// [`ParseError::EmptyInput`], so the encoder rejects symmetrically.
pub fn teletext_descriptor_multi(
    entries: &[([u8; 3], u8, u8, u8)],
) -> Result<Vec<u8>, DescriptorError> {
    if entries.is_empty() {
        return Err(DescriptorError::EmptyEntries { tag: 0x56 });
    }
    let body_len = entries.len() * 5;
    debug_assert!(body_len <= u8::MAX as usize, "descriptor length is u8");
    let mut out = Vec::with_capacity(2 + body_len);
    out.push(0x56); // tag
    out.push(body_len as u8); // length
    for (language, teletext_type, magazine_number, page_number) in entries {
        out.extend_from_slice(language);
        out.push(((teletext_type & 0x1F) << 3) | (magazine_number & 0x07));
        out.push(*page_number);
    }
    Ok(out)
}

/// `registration_descriptor` (tag 0x05) carrying ASCII format_identifier
/// `"VTTC"` — **informal industry convention** for WebVTT-in-MPEG-TS.
/// Not defined by RFC 8216 / draft-pantos-hls-rfc8216bis nor any
/// published normative spec. Originates in ffmpeg's `mpegtsenc.c`
/// emitter and is recognized by hls.js v1.7+ and mediamtx.
pub fn format_identifier_vttc() -> Vec<u8> {
    vec![0x05, 0x04, b'V', b'T', b'T', b'C']
}

/// `registration_descriptor` (tag 0x05) carrying ASCII format_identifier
/// `"GA94"` — **informal industry convention** for CEA-708 caption data
/// carried as a standalone elementary stream. ATSC A/53 Part 4 §6.2.3
/// defines `"GA94"` as the `user_data_identifier` for caption data
/// **embedded in MPEG-2 video user_data**, not as a stream-level
/// marker. The auto-emitted descriptor here is best-effort interop with
/// ATSC ecosystem tooling, not normatively defined.
pub fn format_identifier_ga94() -> Vec<u8> {
    vec![0x05, 0x04, b'G', b'A', b'9', b'4']
}

/// `registration_descriptor` (tag 0x05) carrying ASCII format_identifier
/// `"AV01"` — the AV1-in-MPEG-2-TS binding §2.1 marker. Per the binding
/// spec, this descriptor MUST be the first in the per-stream PMT
/// descriptor loop.
pub fn format_identifier_av01() -> Vec<u8> {
    vec![0x05, 0x04, b'A', b'V', b'0', b'1']
}

/// `registration_descriptor` (tag 0x05) carrying ASCII format_identifier
/// `"AC-3"` — the ATSC AC-3 marker per ATSC A/53 Part 3 §5.1. Strict
/// ATSC consumers (ffmpeg, GStreamer, TSDuck) gate AC-3 classification
/// on stream_type 0x81 + this registration; without it they may
/// fall back to probing or misclassify as MP3-on-user-private.
///
/// 6 bytes total: tag(1) + length(1) + format_identifier(4).
///
/// Note: DVB-shaped AC-3 (stream_type 0x06 + DVB AC-3 descriptor 0x6A)
/// is a distinct path and remains deferred — see `deferred-features.md`.
pub fn format_identifier_ac3() -> Vec<u8> {
    vec![0x05, 0x04, b'A', b'C', b'-', b'3']
}

/// ISO 639 Language descriptor (tag 0x0A) — H.222.0 §2.6.18.
/// 3-byte language code + 1-byte audio_type. Conventional on audio PIDs;
/// valid on any ES.
pub fn iso_639_language(language: [u8; 3], audio_type: u8) -> Vec<u8> {
    vec![
        0x0A,
        0x04,
        language[0],
        language[1],
        language[2],
        audio_type,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_klva_no_additional() {
        let bytes = registration(*b"KLVA", &[]);
        assert_eq!(bytes, vec![0x05, 0x04, b'K', b'L', b'V', b'A']);
    }

    #[test]
    fn format_identifier_ac3_canonical_bytes() {
        let bytes = format_identifier_ac3();
        assert_eq!(bytes, vec![0x05, 0x04, b'A', b'C', b'-', b'3']);
        assert_eq!(bytes.len(), 6);
        assert_eq!(bytes[0], 0x05); // registration_descriptor tag
        assert_eq!(bytes[1], 0x04); // length
        assert_eq!(&bytes[2..6], b"AC-3");
    }

    #[test]
    fn format_identifier_av01_canonical_bytes() {
        let bytes = format_identifier_av01();
        assert_eq!(bytes, vec![0x05, 0x04, b'A', b'V', b'0', b'1']);
    }

    #[test]
    fn registration_hdmv_with_trailing_bytes() {
        // Family A's video PID shape from the testfiles corpus.
        let bytes = registration(*b"HDMV", &[0xFF, 0x1B, 0x44, 0x3F]);
        assert_eq!(
            bytes,
            vec![0x05, 0x08, b'H', b'D', b'M', b'V', 0xFF, 0x1B, 0x44, 0x3F]
        );
    }

    // Skipped in debug builds because debug_assert! catches the overflow
    // before the clamp branch runs. Release builds rely on the clamp;
    // this test verifies the clamp is correct so the cache path in
    // Muxer::new can't overflow downstream descriptor buffers.
    #[cfg(not(debug_assertions))]
    #[test]
    fn registration_clamps_additional_to_251_body() {
        // Caller-supplied 252 bytes of additional info — body would be 256
        // (4 + 252) which exceeds the 251 single-byte length cap. Release
        // build silently clamps trailing bytes; verify the resulting TLV
        // is well-formed.
        let long = vec![0xAAu8; 252];
        let bytes = registration(*b"TEST", &long);
        assert_eq!(bytes[0], 0x05); // descriptor_tag
        assert_eq!(bytes[1], 251); // descriptor_length (clamped)
        assert_eq!(bytes.len(), 2 + 251); // tag + length + 251 body
        assert_eq!(&bytes[2..6], b"TEST");
        // 251 - 4 (format_identifier) = 247 bytes of additional data retained.
        assert!(bytes[6..].iter().all(|&b| b == 0xAA));
        assert_eq!(bytes[6..].len(), 247);
    }

    #[test]
    fn metadata_klva_canonical_shape() {
        // Family B's KLV PID 0x26 descriptor — exact bytes from the corpus.
        // app_format=0x0100 | metadata_format=0xFF | format_id=0x4B4C5641
        // (KLVA) | service_id=0x00 | flags=0x00 (decoder_config_flags=0,
        // DSM_CC_flag=0, 4 reserved bits=1).
        let bytes = metadata_klva(0x00);
        assert_eq!(
            bytes,
            vec![
                0x26, 0x09, 0x01, 0x00, 0xFF, 0x4B, 0x4C, 0x56, 0x41, 0x00, 0x0F
            ]
        );
    }

    #[test]
    fn metadata_std_all_zero() {
        // Matches Family B's 0x27 descriptor (input/buf/output rates all 0,
        // upper bits reserved = 1).
        let bytes = metadata_std(0, 0, 0);
        assert_eq!(
            bytes,
            vec![
                0x27, 0x09, 0xC0, 0x00, 0x00, 0xC0, 0x00, 0x00, 0xC0, 0x00, 0x00
            ]
        );
    }

    #[test]
    fn metadata_std_packs_22bit_rates_with_reserved_bits() {
        // input_leak_rate = 1 → low-byte = 0x01, high byte = 0xC0 | 0 = 0xC0.
        // buffer_size = 0x3F_FFFF (max 22-bit) → 0xFF 0xFF 0xFF (high byte
        // = 0xC0 | 0x3F = 0xFF).
        // output_leak_rate = 0x100 → bytes 0xC0, 0x01, 0x00.
        let bytes = metadata_std(1, 0x003F_FFFF, 0x100);
        assert_eq!(
            bytes,
            vec![
                0x27, 0x09, 0xC0, 0x00, 0x01, 0xFF, 0xFF, 0xFF, 0xC0, 0x01, 0x00
            ]
        );
    }

    #[test]
    fn user_private_default_tag_0xff() {
        let bytes = user_private(b"VIDEO-ARS");
        assert_eq!(
            bytes,
            vec![
                0xFF, 9, b'V', b'I', b'D', b'E', b'O', b'-', b'A', b'R', b'S'
            ]
        );
    }

    #[test]
    fn user_private_with_tag_lets_caller_pick_slot() {
        let bytes = user_private_with_tag(0x7E, b"VENDOR");
        assert_eq!(bytes, vec![0x7E, 6, b'V', b'E', b'N', b'D', b'O', b'R']);
    }

    #[test]
    fn component_descriptor_textbook_shape() {
        let bytes = component(0x09, 0x00, 0x42, *b"eng", "EO 1080p");
        // tag(1) + len(1) + (4-bit reserved + 4-bit content)(1) + type(1)
        // + tag(1) + lang(3) + text("EO 1080p" = 8 bytes) = 16 bytes total.
        // First body byte = 0xF0 | (0x09 & 0x0F) = 0xF9.
        assert_eq!(
            bytes,
            vec![
                0x50, 14, 0xF9, 0x00, 0x42, b'e', b'n', b'g', b'E', b'O', b' ', b'1', b'0', b'8',
                b'0', b'p',
            ]
        );
    }

    #[test]
    fn stream_identifier_one_byte_body() {
        let bytes = stream_identifier(0x42);
        assert_eq!(bytes, vec![0x52, 0x01, 0x42]);
    }

    #[test]
    fn iso_639_language_4_byte_body() {
        let bytes = iso_639_language(*b"eng", 0x00);
        assert_eq!(bytes, vec![0x0A, 0x04, b'e', b'n', b'g', 0x00]);
    }

    #[test]
    fn subtitling_descriptor_single_entry_round_trip_bytes() {
        // ETSI EN 300 468 §6.2.41 single-entry: tag(1) + length(1) +
        //   ISO_639_lang(3) + subtitling_type(1) + composition_page_id(2) +
        //   ancillary_page_id(2) = 8 payload bytes.
        let bytes = subtitling_descriptor(*b"eng", 0x10, 0x0001, 0x0001);
        assert_eq!(
            bytes,
            vec![
                0x59, // tag
                0x08, // length
                b'e', b'n', b'g', 0x10, 0x00, 0x01, 0x00, 0x01,
            ]
        );
    }

    #[test]
    fn teletext_descriptor_single_entry_round_trip_bytes() {
        // ETSI EN 300 468 §6.2.43 single-entry: tag(1) + length(1) +
        //   ISO_639_lang(3) + (teletext_type<<3 | magazine_number)(1) +
        //   page_number(1) = 5 payload bytes.
        let bytes = teletext_descriptor(*b"eng", 0x02, 1, 0x88);
        assert_eq!(
            bytes,
            vec![
                0x56, // tag
                0x05, // length
                b'e',
                b'n',
                b'g',
                (0x02 << 3) | 1,
                0x88,
            ]
        );
    }

    #[test]
    fn format_identifier_vttc_descriptor_round_trip_bytes() {
        let bytes = format_identifier_vttc();
        assert_eq!(bytes, vec![0x05, 0x04, b'V', b'T', b'T', b'C']);
    }

    #[test]
    fn format_identifier_ga94_descriptor_round_trip_bytes() {
        let bytes = format_identifier_ga94();
        assert_eq!(bytes, vec![0x05, 0x04, b'G', b'A', b'9', b'4']);
    }

    #[test]
    fn format_identifier_av01_descriptor_round_trip_bytes() {
        let bytes = format_identifier_av01();
        assert_eq!(bytes, vec![0x05, 0x04, b'A', b'V', b'0', b'1']);
    }

    #[test]
    fn subtitling_descriptor_multi_emits_two_entries() {
        let descriptor =
            subtitling_descriptor_multi(&[(*b"eng", 0x10, 1, 1), (*b"spa", 0x10, 2, 2)])
                .expect("non-empty entries");
        // tag(1) + length(1) + 2 × 8 bytes per entry = 18 bytes total.
        assert_eq!(descriptor[0], 0x59, "tag");
        assert_eq!(descriptor[1], 0x10, "length = 16 (2 × 8 byte entries)");
        // First entry: lang(3) + subtitling_type(1) + comp_page_id(2) + anc_page_id(2).
        assert_eq!(&descriptor[2..5], b"eng");
        assert_eq!(descriptor[5], 0x10);
        assert_eq!(&descriptor[6..8], &[0x00, 0x01]);
        assert_eq!(&descriptor[8..10], &[0x00, 0x01]);
        // Second entry.
        assert_eq!(&descriptor[10..13], b"spa");
        assert_eq!(descriptor[13], 0x10);
        assert_eq!(&descriptor[14..16], &[0x00, 0x02]);
        assert_eq!(&descriptor[16..18], &[0x00, 0x02]);
        assert_eq!(descriptor.len(), 18);
    }

    #[test]
    #[allow(clippy::identity_op)] // spec form (teletext_type << 3) | magazine_number is illustrative
    fn teletext_descriptor_multi_emits_two_entries() {
        let descriptor =
            teletext_descriptor_multi(&[(*b"eng", 0x02, 0, 0x88), (*b"spa", 0x02, 0, 0x77)])
                .expect("non-empty entries");
        // tag(1) + length(1) + 2 × 5 bytes per entry = 12 bytes total.
        assert_eq!(descriptor[0], 0x56, "tag");
        assert_eq!(descriptor[1], 0x0A, "length = 10 (2 × 5 byte entries)");
        assert_eq!(&descriptor[2..5], b"eng");
        assert_eq!(descriptor[5], (0x02 << 3) | 0x00); // teletext_type | magazine_number
        assert_eq!(descriptor[6], 0x88);
        assert_eq!(&descriptor[7..10], b"spa");
        assert_eq!(descriptor[10], (0x02 << 3) | 0x00);
        assert_eq!(descriptor[11], 0x77);
        assert_eq!(descriptor.len(), 12);
    }

    #[test]
    fn subtitling_descriptor_single_via_multi_matches_single_helper() {
        // The single-entry helper's output should match a 1-element multi call.
        let single = subtitling_descriptor(*b"eng", 0x10, 1, 1);
        let multi =
            subtitling_descriptor_multi(&[(*b"eng", 0x10, 1, 1)]).expect("non-empty entries");
        assert_eq!(single, multi);
    }

    #[test]
    fn teletext_descriptor_single_via_multi_matches_single_helper() {
        let single = teletext_descriptor(*b"eng", 0x02, 0, 0x88);
        let multi =
            teletext_descriptor_multi(&[(*b"eng", 0x02, 0, 0x88)]).expect("non-empty entries");
        assert_eq!(single, multi);
    }

    // ── Empty-entries rejection (audit Subt-A symmetry fix) ──────────────

    #[test]
    fn subtitling_descriptor_multi_rejects_empty_entries() {
        // Empty entries would produce a degenerate `0x59 0x00` descriptor
        // that the demux parser rejects with ParseError::EmptyInput.
        // Encoder rejects symmetrically.
        let result = subtitling_descriptor_multi(&[]);
        assert_eq!(result, Err(DescriptorError::EmptyEntries { tag: 0x59 }));
    }

    #[test]
    fn teletext_descriptor_multi_rejects_empty_entries() {
        let result = teletext_descriptor_multi(&[]);
        assert_eq!(result, Err(DescriptorError::EmptyEntries { tag: 0x56 }));
    }

    #[test]
    fn subtitling_descriptor_multi_accepts_one_entry() {
        let bytes = subtitling_descriptor_multi(&[(*b"eng", 0x10, 0x0001, 0x0002)])
            .expect("one entry should succeed");
        assert_eq!(bytes[0], 0x59); // descriptor_tag
        assert_eq!(bytes[1], 8); // length = 1 entry × 8 bytes
    }

    #[test]
    fn teletext_descriptor_multi_accepts_one_entry() {
        let bytes = teletext_descriptor_multi(&[(*b"eng", 0x02, 0, 0x88)])
            .expect("one entry should succeed");
        assert_eq!(bytes[0], 0x56); // descriptor_tag
        assert_eq!(bytes[1], 5); // length = 1 entry × 5 bytes
    }
}
