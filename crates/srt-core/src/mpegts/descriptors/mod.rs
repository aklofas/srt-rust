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

/// Maximum total descriptor-loop length per ES PID, bounded by the
/// PMT-fits-in-one-TS-packet rule. Computed as: 183 PMT payload bytes −
/// 17 PMT fixed overhead = 166 bytes available for the entire ES loop
/// (header + descriptors) summed across all streams in the same PMT.
/// `Config::validate` returns `MuxError::PmtTooLarge` when the actual
/// sum exceeds this.
pub const MAX_DESCRIPTOR_LOOP_PER_PMT: usize = 166;

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
/// `srt-rust`'s demuxer surfaces it both via
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
    let mut out = Vec::with_capacity(10);
    out.push(0x59); // tag
    out.push(0x08); // length
    out.extend_from_slice(&language);
    out.push(subtitling_type);
    out.extend_from_slice(&composition_page_id.to_be_bytes());
    out.extend_from_slice(&ancillary_page_id.to_be_bytes());
    out
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
    let mut out = Vec::with_capacity(7);
    out.push(0x56); // tag
    out.push(0x05); // length
    out.extend_from_slice(&language);
    out.push(((teletext_type & 0x1F) << 3) | (magazine_number & 0x07));
    out.push(page_number);
    out
}

/// `registration_descriptor` (tag 0x05) carrying ASCII format_identifier
/// `"VTTC"` — the marker for WebVTT-in-MPEG-TS per Apple's HLS
/// authoring spec (matches ffmpeg's `mpegtsenc` emitter).
pub fn format_identifier_vttc() -> Vec<u8> {
    vec![0x05, 0x04, b'V', b'T', b'T', b'C']
}

/// `registration_descriptor` (tag 0x05) carrying ASCII format_identifier
/// `"GA94"` — the ATSC A/53 marker, used here as the best-effort
/// signal for CEA-708 caption data carried as a separate elementary
/// stream (rather than embedded in H.264 / H.265 SEI).
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
}
