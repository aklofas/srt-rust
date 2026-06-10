//! ST 0601 encode entry points — 5 public functions plus private helpers.

use crate::error::KlvEncodeError;
use crate::klv::checksum::checksum_running_sum_16;
use crate::klv::length::{ber_len, ber_oid_len, write_ber, write_ber_oid};
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
    encode_to_vec(record)
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
            3 => record.mission_id.as_ref().map(|s| s.len()),
            4 => record.platform_tail_number.as_ref().map(|s| s.len()),
            5 => record.platform_heading_deg.map(|_| 2),
            6 => record.platform_pitch_deg.map(|_| 2),
            7 => record.platform_roll_deg.map(|_| 2),
            8 => record.platform_true_airspeed.map(|_| 1),
            9 => record.platform_indicated_airspeed.map(|_| 1),
            10 => record.platform_designation.as_ref().map(|s| s.len()),
            11 => record.image_source_sensor.as_ref().map(|s| s.len()),
            12 => record.image_coordinate_system.as_ref().map(|s| s.len()),
            13 => record.sensor_lat_deg.map(|_| 4),
            14 => record.sensor_lon_deg.map(|_| 4),
            15 => record.sensor_alt_m.map(|_| 2),
            16 => record.sensor_hfov_deg.map(|_| 2),
            17 => record.sensor_vfov_deg.map(|_| 2),
            18 => record.sensor_rel_az_deg.map(|_| 4),
            19 => record.sensor_rel_el_deg.map(|_| 4),
            20 => record.sensor_rel_roll_deg.map(|_| 4),
            21 => record.slant_range_m.map(|_| 4),
            22 => record.target_width_m.map(|_| 2),
            23 => record.frame_center_lat_deg.map(|_| 4),
            24 => record.frame_center_lon_deg.map(|_| 4),
            25 => record.frame_center_elev_m.map(|_| 2),
            26 => record.corner_lat_offset_p1_deg.map(|_| 2),
            27 => record.corner_lon_offset_p1_deg.map(|_| 2),
            28 => record.corner_lat_offset_p2_deg.map(|_| 2),
            29 => record.corner_lon_offset_p2_deg.map(|_| 2),
            30 => record.corner_lat_offset_p3_deg.map(|_| 2),
            31 => record.corner_lon_offset_p3_deg.map(|_| 2),
            32 => record.corner_lat_offset_p4_deg.map(|_| 2),
            33 => record.corner_lon_offset_p4_deg.map(|_| 2),
            47 => record.generic_flag_data.map(|_| 1),
            48 => record.security_local_set.as_ref().map(|v| v.len()),
            50 => record.platform_angle_of_attack_deg.map(|_| 2),
            59 => record.platform_call_sign.as_ref().map(|s| s.len()),
            65 => record
                .uas_ls_version
                .map(|_| 1)
                .or(if auto_version { Some(1) } else { None }),
            74 => record.vmti.as_ref().map(|v| v.len()),
            82 => record.corner_lat_p1_deg.map(|_| 4),
            83 => record.corner_lon_p1_deg.map(|_| 4),
            84 => record.corner_lat_p2_deg.map(|_| 4),
            85 => record.corner_lon_p2_deg.map(|_| 4),
            86 => record.corner_lat_p3_deg.map(|_| 4),
            87 => record.corner_lon_p3_deg.map(|_| 4),
            88 => record.corner_lat_p4_deg.map(|_| 4),
            89 => record.corner_lon_p4_deg.map(|_| 4),
            75 => record.sensor_ellipsoid_height_m.map(|_| 2),
            78 => record.frame_center_ellipsoid_height_m.map(|_| 2),
            90 => record.platform_pitch_full_deg.map(|_| 4),
            91 => record.platform_roll_full_deg.map(|_| 4),
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
            .map(|s| check_string(3, s, &spec.encoding).map(|_| s.as_bytes().to_vec()))
            .transpose()?,
        4 => record
            .platform_tail_number
            .as_ref()
            .map(|s| check_string(4, s, &spec.encoding).map(|_| s.as_bytes().to_vec()))
            .transpose()?,
        5 => encode_ranged(record.platform_heading_deg, spec, &mut scratch)?,
        6 => encode_ranged(record.platform_pitch_deg, spec, &mut scratch)?,
        7 => encode_ranged(record.platform_roll_deg, spec, &mut scratch)?,
        8 => encode_ranged(record.platform_true_airspeed, spec, &mut scratch)?,
        9 => encode_ranged(record.platform_indicated_airspeed, spec, &mut scratch)?,
        10 => record
            .platform_designation
            .as_ref()
            .map(|s| check_string(10, s, &spec.encoding).map(|_| s.as_bytes().to_vec()))
            .transpose()?,
        11 => record
            .image_source_sensor
            .as_ref()
            .map(|s| check_string(11, s, &spec.encoding).map(|_| s.as_bytes().to_vec()))
            .transpose()?,
        12 => record
            .image_coordinate_system
            .as_ref()
            .map(|s| check_string(12, s, &spec.encoding).map(|_| s.as_bytes().to_vec()))
            .transpose()?,
        13 => encode_ranged(record.sensor_lat_deg, spec, &mut scratch)?,
        14 => encode_ranged(record.sensor_lon_deg, spec, &mut scratch)?,
        15 => encode_ranged(record.sensor_alt_m, spec, &mut scratch)?,
        16 => encode_ranged(record.sensor_hfov_deg, spec, &mut scratch)?,
        17 => encode_ranged(record.sensor_vfov_deg, spec, &mut scratch)?,
        18 => encode_ranged(record.sensor_rel_az_deg, spec, &mut scratch)?,
        19 => encode_ranged(record.sensor_rel_el_deg, spec, &mut scratch)?,
        20 => encode_ranged(record.sensor_rel_roll_deg, spec, &mut scratch)?,
        21 => encode_ranged(record.slant_range_m, spec, &mut scratch)?,
        22 => encode_ranged(record.target_width_m, spec, &mut scratch)?,
        23 => encode_ranged(record.frame_center_lat_deg, spec, &mut scratch)?,
        24 => encode_ranged(record.frame_center_lon_deg, spec, &mut scratch)?,
        25 => encode_ranged(record.frame_center_elev_m, spec, &mut scratch)?,
        26 => encode_ranged(record.corner_lat_offset_p1_deg, spec, &mut scratch)?,
        27 => encode_ranged(record.corner_lon_offset_p1_deg, spec, &mut scratch)?,
        28 => encode_ranged(record.corner_lat_offset_p2_deg, spec, &mut scratch)?,
        29 => encode_ranged(record.corner_lon_offset_p2_deg, spec, &mut scratch)?,
        30 => encode_ranged(record.corner_lat_offset_p3_deg, spec, &mut scratch)?,
        31 => encode_ranged(record.corner_lon_offset_p3_deg, spec, &mut scratch)?,
        32 => encode_ranged(record.corner_lat_offset_p4_deg, spec, &mut scratch)?,
        33 => encode_ranged(record.corner_lon_offset_p4_deg, spec, &mut scratch)?,
        47 => record.generic_flag_data.map(|b| vec![b]),
        48 => record.security_local_set.clone(),
        50 => encode_ranged(record.platform_angle_of_attack_deg, spec, &mut scratch)?,
        59 => record
            .platform_call_sign
            .as_ref()
            .map(|s| check_string(59, s, &spec.encoding).map(|_| s.as_bytes().to_vec()))
            .transpose()?,
        65 => match (record.uas_ls_version, version_fallback) {
            (Some(v), _) => Some(vec![v]),
            (None, Some(fallback)) => Some(vec![fallback]),
            (None, None) => None,
        },
        74 => record.vmti.clone(),
        82 => encode_ranged(record.corner_lat_p1_deg, spec, &mut scratch)?,
        83 => encode_ranged(record.corner_lon_p1_deg, spec, &mut scratch)?,
        84 => encode_ranged(record.corner_lat_p2_deg, spec, &mut scratch)?,
        85 => encode_ranged(record.corner_lon_p2_deg, spec, &mut scratch)?,
        86 => encode_ranged(record.corner_lat_p3_deg, spec, &mut scratch)?,
        87 => encode_ranged(record.corner_lon_p3_deg, spec, &mut scratch)?,
        88 => encode_ranged(record.corner_lat_p4_deg, spec, &mut scratch)?,
        89 => encode_ranged(record.corner_lon_p4_deg, spec, &mut scratch)?,
        75 => encode_ranged(record.sensor_ellipsoid_height_m, spec, &mut scratch)?,
        78 => encode_ranged(record.frame_center_ellipsoid_height_m, spec, &mut scratch)?,
        90 => encode_ranged(record.platform_pitch_full_deg, spec, &mut scratch)?,
        91 => encode_ranged(record.platform_roll_full_deg, spec, &mut scratch)?,
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
            let mut tag_buf = [0u8; 8];
            let n = write_ber_oid(spec.id as u32, &mut tag_buf)?;
            body.extend_from_slice(&tag_buf[..n]);
            let mut len_buf = [0u8; 16];
            let m = write_ber(value.len(), &mut len_buf)?;
            body.extend_from_slice(&len_buf[..m]);
            body.extend_from_slice(&value);
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
        if is_reserved_or_typed_tag(f.tag) {
            return Err(KlvEncodeError::ReservedTagInUnknown { tag: f.tag });
        }
        let mut tag_buf = [0u8; 8];
        let n = write_ber_oid(f.tag, &mut tag_buf)?;
        body.extend_from_slice(&tag_buf[..n]);
        let mut len_buf = [0u8; 16];
        let m = write_ber(f.value.len(), &mut len_buf)?;
        body.extend_from_slice(&len_buf[..m]);
        body.extend_from_slice(&f.value);
    }
    Ok(())
}

/// True iff `tag` is a structural reserved tag (1 = Checksum, 2 = PTS,
/// 65 = UAS LS Version) or a tag the typed encoder would emit (anything
/// in `tags::TAGS`). Used by `write_unknown_fields` to fail-fast on
/// caller-constructed `unknown` entries that would produce a duplicate
/// or non-conformant Local Set.
///
/// The typed table is u8-keyed; `OwnedRawField.tag` is u32, so any tag
/// > 255 is by definition not in the typed table and not structural.
fn is_reserved_or_typed_tag(tag: u32) -> bool {
    if tag > u8::MAX as u32 {
        return false;
    }
    super::tags::lookup(tag as u8).is_some()
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
