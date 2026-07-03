//! ST 0601 encode entry points — 5 public functions plus private helpers.

use crate::error::KlvEncodeError;
use crate::klv::checksum::checksum_running_sum_16;
use crate::klv::length::{ber_len, ber_oid_len, write_ber};
use crate::klv::pack::{emit_ber_oid_tlv, is_typed_tag};
use alloc::vec::Vec;

use super::mapping::encode_fixed_range;
use super::model::{EncodeConfig, UasDatalinkLs};
use super::tags::{Encoding, TAGS, TagSpec};

/// Encode a UAS Datalink Local Set into the caller-provided buffer per
/// MISB ST 0601. Returns the number of bytes written.
///
/// Use [`encode_to_vec`] to allocate a fresh `Vec<u8>` instead;
/// [`encoded_len`] sizes a buffer in advance.
///
/// # Errors
/// - [`KlvEncodeError::BufferTooSmall`] if `out.len()` is less than the
///   required encoded length (call [`encoded_len`] first).
/// - [`KlvEncodeError::OutOfRange`] if any IMAPB-encoded float field
///   value falls outside its declared range (e.g. `platform_heading_deg`
///   outside `0.0..=360.0`).
/// - [`KlvEncodeError::StringTooLong`] if a UTF-8 string field exceeds
///   the spec-declared byte cap.
/// - [`KlvEncodeError::RecordTooLarge`] if the encoded body would
///   overflow the BER-encodable length limit.
/// - [`KlvEncodeError::ReservedTagInUnknown`] if `record.unknown` carries
///   a tag the typed encoder would emit (any tag in `tags::TAGS`) or a
///   reserved structural tag (`{1, 2, 65}`). The `unknown` vec is only
///   for forward-compat pass-through of tags this encoder does not model.
pub fn encode(record: &UasDatalinkLs, out: &mut [u8]) -> Result<usize, KlvEncodeError> {
    encode_with(record, &EncodeConfig::default(), out)
}

pub fn encode_with(
    record: &UasDatalinkLs,
    opts: &EncodeConfig,
    out: &mut [u8],
) -> Result<usize, KlvEncodeError> {
    // Build the inner body into a temporary Vec, then assemble UL + BER length + body + checksum.
    let mut body: Vec<u8> = Vec::with_capacity(256);
    write_typed_fields(record, opts, &mut body)?;
    write_unknown_fields(record, &mut body)?;

    // Reserve room for Tag 1 (checksum) — 4 bytes (tag=1, len-byte=1, value=2).
    let body_len_with_checksum = body.len() + 4;
    let outer_len_bytes = ber_len(body_len_with_checksum);
    let total = 16 + outer_len_bytes + body_len_with_checksum;

    if out.len() < total {
        return Err(KlvEncodeError::BufferTooSmall {
            needed: total,
            got: out.len(),
        });
    }

    // 1) UL
    out[..16].copy_from_slice(&opts.universal_label.0);
    // 2) Outer BER length
    let written = write_ber(body_len_with_checksum, &mut out[16..])?;
    let body_offset = 16 + written;
    // 3) Body
    out[body_offset..body_offset + body.len()].copy_from_slice(&body);
    // 4) Tag 1 (checksum) tag + len
    let cksum_tag_offset = body_offset + body.len();
    out[cksum_tag_offset] = 0x01; // tag 1
    out[cksum_tag_offset + 1] = 0x02; // len 2
    // 5) Compute checksum across [UL .. start of checksum value]
    let cksum_value_offset = cksum_tag_offset + 2;
    let cksum = checksum_running_sum_16(&out[..cksum_value_offset]);
    out[cksum_value_offset] = (cksum >> 8) as u8;
    out[cksum_value_offset + 1] = cksum as u8;
    Ok(total)
}

/// Encode a UAS Datalink Local Set into a fresh `Vec<u8>`. Convenience
/// over [`encode`] when the caller has no pre-sized buffer.
///
/// # Errors
/// Returns the same [`KlvEncodeError`] variants as [`encode`] —
/// [`KlvEncodeError::OutOfRange`] for IMAPB ranges,
/// [`KlvEncodeError::StringTooLong`] for over-cap UTF-8 fields,
/// [`KlvEncodeError::RecordTooLarge`] for BER-overflow records, and
/// [`KlvEncodeError::ReservedTagInUnknown`] if `record.unknown` carries
/// a reserved or typed tag.
/// (`KlvEncodeError::BufferTooSmall` cannot fire on this path — the
/// buffer is pre-sized via [`encoded_len`].)
pub fn encode_to_vec(record: &UasDatalinkLs) -> Result<Vec<u8>, KlvEncodeError> {
    let n = encoded_len(record);
    let mut buf = vec![0u8; n];
    let written = encode(record, &mut buf)?;
    buf.truncate(written);
    Ok(buf)
}

/// Encode a UAS Datalink Local Set, rejecting records that omit any
/// item the spec marks mandatory. Symmetric with
/// [`super::decode_strict_compliance`], which already rejects on-the-wire
/// instances missing the same mandatory items.
///
/// Per MISB ST 0601.13 §6 (and re-affirmed by ST 0601.24 §6 and
/// ST 0107.5 §6.2 — Local Set conformance rules), every conformant ST
/// 0601 instance must carry Tag 2 (Precision Time Stamp). Tag 1
/// (Checksum) and Tag 65 (LS Version Number) are also mandatory but are
/// auto-emitted by this encoder, so the only caller-supplied mandatory
/// item this function gates is Tag 2.
///
/// The set of enforced tags is exposed (hidden from doc) as
/// [`_mandatory_tags`] for future proptest hookup; production callers
/// should not depend on it.
///
/// # Errors
///
/// - [`KlvEncodeError::MissingMandatoryItem`] if `record.timestamp_us`
///   is `None`. `tag` will be `2` and `name` will be
///   `"Precision Time Stamp"`.
/// - All [`KlvEncodeError`] variants that [`encode_to_vec`] can return,
///   surfaced verbatim from the underlying encode path once the strict
///   precondition gate passes.
pub fn encode_strict_compliance(record: &UasDatalinkLs) -> Result<Vec<u8>, KlvEncodeError> {
    // The only caller-supplied mandatory item under ST 0601.13-22 +
    // ST 0107.5 §6.2 — Tags 1 + 65 auto-emit (see encode.rs:45-73 +
    // 119-166), so they cannot be missing here.
    if record.timestamp_us.is_none() {
        return Err(KlvEncodeError::MissingMandatoryItem {
            tag: 2,
            name: "Precision Time Stamp",
        });
    }
    // ST 0107 §6.3.3.1: strip banned control chars from all string fields
    // before encoding. Stripping runs before the DA-KLV-1 empty-string
    // mapping, so a whitespace-only field strips to "" → encodes as [0x00].
    let mut r = record.clone();
    strip_control_chars_st0601(&mut r);
    encode_to_vec(&r)
}

/// Caller-supplied mandatory ST 0601 tag IDs enforced by
/// [`encode_strict_compliance`]. Tag 1 (Checksum) and Tag 65 (LS Version
/// Number) are deliberately excluded — both are auto-emitted by the
/// encoder. Validate-2 proptests may pivot on this list to fuzz strict
/// vs. lenient encode paths.
#[doc(hidden)]
#[must_use]
pub fn _mandatory_tags() -> &'static [u16] {
    &[2]
}

pub fn encoded_len(record: &UasDatalinkLs) -> usize {
    encoded_len_with(record, &EncodeConfig::default())
}

pub fn encoded_len_with(record: &UasDatalinkLs, opts: &EncodeConfig) -> usize {
    let mut body_len = 0usize;
    each_typed_field(record, opts, |tag, value_len| {
        body_len += ber_oid_len(tag as u32) + ber_len(value_len) + value_len;
    });
    for f in &record.unknown {
        body_len += ber_oid_len(f.tag) + ber_len(f.value.len()) + f.value.len();
    }
    let body_len_with_checksum = body_len + 4; // tag 1 (1 byte) + len byte (1) + value (2 bytes)
    16 + ber_len(body_len_with_checksum) + body_len_with_checksum
}

/// Visit each typed field that will be emitted, calling `visit(tag_id, value_len)`.
/// Used by both `encoded_len_with` (for sizing) and `write_typed_fields` (for emission).
///
/// NOTE: the arm-65 auto-version below sizes Tag 65 even when
/// `uas_ls_version` is `None`, matching the `encode*` entry points'
/// fallback emission. Do NOT reuse this visitor for `patch()` sizing —
/// `patch()` never auto-injects Tag 65 (it builds output `Vec`s
/// directly and never calls this).
fn each_typed_field<F: FnMut(u8, usize)>(
    record: &UasDatalinkLs,
    _opts: &EncodeConfig,
    mut visit: F,
) {
    // Tag 65 auto-emit if not explicitly set.
    let auto_version = record.uas_ls_version.is_none();

    for spec in TAGS {
        if spec.id == 1 {
            continue; // checksum is appended after
        }
        let len = match spec.id {
            2 => record.timestamp_us.map(|_| 8),
            3 => record.mission_id.as_ref().map(|s| str_wire_len(s)),
            4 => record
                .platform_tail_number
                .as_ref()
                .map(|s| str_wire_len(s)),
            10 => record
                .platform_designation
                .as_ref()
                .map(|s| str_wire_len(s)),
            11 => record.image_source_sensor.as_ref().map(|s| str_wire_len(s)),
            12 => record
                .image_coordinate_system
                .as_ref()
                .map(|s| str_wire_len(s)),
            47 => record.generic_flag_data.map(|_| 1),
            48 => record.security_local_set.as_ref().map(|v| v.len()),
            59 => record.platform_call_sign.as_ref().map(|s| str_wire_len(s)),
            65 => record
                .uas_ls_version
                .map(|_| 1)
                .or(if auto_version { Some(1) } else { None }),
            74 => record.vmti.as_ref().map(|v| v.len()),
            // All 39 ranged Option<f64> fields — driven from RANGED_FIELDS so
            // that `byte_length` comes from the single `tags::TAGS` source.
            _ if spec.range.is_some() => super::decode::RANGED_FIELDS
                .iter()
                .find(|e| e.id == spec.id)
                .and_then(|e| (e.get)(record).map(|_| spec.range.as_ref().unwrap().byte_length)),
            _ => None,
        };
        if let Some(len) = len {
            visit(spec.id, len);
        }
    }
}

/// Encode the VALUE bytes for one typed tag from `record`, or `None`
/// when the corresponding field is absent. `version_fallback` supplies
/// the Tag 65 auto-version used by the `encode*` entry points; `patch`
/// passes `None` so only explicitly-set fields are encoded.
pub(super) fn encode_tag_value(
    record: &UasDatalinkLs,
    spec: &TagSpec,
    version_fallback: Option<u8>,
) -> Result<Option<Vec<u8>>, KlvEncodeError> {
    let mut scratch = [0u8; 8];
    Ok(match spec.id {
        2 => record.timestamp_us.map(|t| t.to_be_bytes().to_vec()),
        3 => record
            .mission_id
            .as_ref()
            .map(|s| check_string(3, s, &spec.encoding).map(|_| str_to_bytes(s)))
            .transpose()?,
        4 => record
            .platform_tail_number
            .as_ref()
            .map(|s| check_string(4, s, &spec.encoding).map(|_| str_to_bytes(s)))
            .transpose()?,
        10 => record
            .platform_designation
            .as_ref()
            .map(|s| check_string(10, s, &spec.encoding).map(|_| str_to_bytes(s)))
            .transpose()?,
        11 => record
            .image_source_sensor
            .as_ref()
            .map(|s| check_string(11, s, &spec.encoding).map(|_| str_to_bytes(s)))
            .transpose()?,
        12 => record
            .image_coordinate_system
            .as_ref()
            .map(|s| check_string(12, s, &spec.encoding).map(|_| str_to_bytes(s)))
            .transpose()?,
        47 => record.generic_flag_data.map(|b| vec![b]),
        48 => record.security_local_set.clone(),
        59 => record
            .platform_call_sign
            .as_ref()
            .map(|s| check_string(59, s, &spec.encoding).map(|_| str_to_bytes(s)))
            .transpose()?,
        65 => match (record.uas_ls_version, version_fallback) {
            (Some(v), _) => Some(vec![v]),
            (None, Some(fallback)) => Some(vec![fallback]),
            (None, None) => None,
        },
        74 => record.vmti.clone(),
        // All 39 ranged Option<f64> fields — driven from RANGED_FIELDS so the
        // tag→field mapping is the single source of truth across decode + encode.
        _ if spec.range.is_some() => {
            if let Some(entry) = super::decode::RANGED_FIELDS
                .iter()
                .find(|e| e.id == spec.id)
            {
                encode_ranged((entry.get)(record), spec, &mut scratch)?
            } else {
                None
            }
        }
        _ => None,
    })
}

fn write_typed_fields(
    record: &UasDatalinkLs,
    opts: &EncodeConfig,
    body: &mut Vec<u8>,
) -> Result<(), KlvEncodeError> {
    for spec in TAGS {
        if spec.id == 1 {
            continue;
        }
        if let Some(value) = encode_tag_value(record, spec, Some(opts.version))? {
            emit_ber_oid_tlv(spec.id as u32, &value, body)?;
        }
    }
    Ok(())
}

fn write_unknown_fields(record: &UasDatalinkLs, body: &mut Vec<u8>) -> Result<(), KlvEncodeError> {
    for f in &record.unknown {
        // Reject reserved/typed tags before emitting. Without this guard,
        // a caller-constructed Tag 2 in `unknown` would produce a duplicate
        // Precision Time Stamp; a Tag 1 would produce a bogus mid-stream
        // pseudo-checksum followed by the real final checksum; a Tag 65
        // would produce a duplicate UAS LS Version Number; and any typed
        // tag (5-91 modeled) would produce a non-conformant duplicate. The
        // `unknown` vec is for forward-compat pass-through only.
        if is_typed_tag(f.tag, super::tags::lookup) {
            return Err(KlvEncodeError::ReservedTagInUnknown { tag: f.tag });
        }
        emit_ber_oid_tlv(f.tag, &f.value, body)?;
    }
    Ok(())
}

fn encode_ranged(
    value: Option<f64>,
    spec: &super::tags::TagSpec,
    scratch: &mut [u8; 8],
) -> Result<Option<Vec<u8>>, KlvEncodeError> {
    let Some(v) = value else { return Ok(None) };
    let r = spec
        .range
        .as_ref()
        .expect("ranged tag must have LinearRange");
    encode_fixed_range(r, spec.id as u32, v, &mut scratch[..r.byte_length])?;
    Ok(Some(scratch[..r.byte_length].to_vec()))
}

/// Wire byte count for a UTF-8 string: empty → 1 (NUL signal), else `s.len()`.
fn str_wire_len(s: &str) -> usize {
    if s.is_empty() { 1 } else { s.len() }
}

/// Encode a UTF-8 string per ST 0107.5 §6.3.3.2:
/// empty string → `[0x00]` (single NUL), non-empty → `s.as_bytes()`.
fn str_to_bytes(s: &str) -> Vec<u8> {
    if s.is_empty() {
        vec![0x00]
    } else {
        s.as_bytes().to_vec()
    }
}

fn check_string(tag: u32, s: &str, enc: &Encoding) -> Result<(), KlvEncodeError> {
    if let Encoding::Utf8 { max_bytes } = enc {
        if s.len() > *max_bytes {
            return Err(KlvEncodeError::StringTooLong {
                tag,
                max: *max_bytes,
            });
        }
    }
    Ok(())
}

/// Strip ST 0107 §6.3.3.1 control characters from all string fields of a
/// record before strict-compliance encode. Runs BEFORE the empty-string
/// mapping (a whitespace-only string strips to `""` → encodes as `[0x00]`).
///
/// Control chars removed: U+0000–U+0008, U+000B, U+000C, U+000E–U+001F, U+007F.
/// Tab (U+0009), LF (U+000A), CR (U+000D) are intentionally kept — they are
/// horizontal whitespace or line endings, not C0 control chars the spec bans.
pub(crate) fn strip_control_chars_st0601(r: &mut super::model::UasDatalinkLs) {
    strip_opt_str(&mut r.mission_id);
    strip_opt_str(&mut r.platform_tail_number);
    strip_opt_str(&mut r.platform_designation);
    strip_opt_str(&mut r.image_source_sensor);
    strip_opt_str(&mut r.image_coordinate_system);
    strip_opt_str(&mut r.platform_call_sign);
}

fn strip_opt_str(s: &mut Option<alloc::string::String>) {
    if let Some(s) = s {
        *s = strip_st0107_control_chars(s);
    }
}

/// Remove ST 0107 §6.3.3.1 banned control characters from a string.
pub(crate) fn strip_st0107_control_chars(s: &str) -> alloc::string::String {
    s.chars().filter(|&c| !is_st0107_control_char(c)).collect()
}

fn is_st0107_control_char(c: char) -> bool {
    matches!(c, '\x00'..='\x08' | '\x0B' | '\x0C' | '\x0E'..='\x1F' | '\x7F')
}
