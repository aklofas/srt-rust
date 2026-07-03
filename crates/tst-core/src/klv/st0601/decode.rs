//! ST 0601 decode entry points — 4 modes per the typed-set's
//! lenient/strict/compliance lineage.

use crate::error::{KlvDecodeError, KlvFieldError};
use crate::klv::length::{read_ber, read_ber_oid_strict, read_ber_strict};
use crate::klv::pack::{Iter, OwnedRawField};
use crate::klv::universal_label::UniversalLabel;
use alloc::borrow::ToOwned;
use alloc::vec::Vec;

use super::mapping::decode_fixed_range;
use super::model::UasDatalinkLs;
use super::tags::{Encoding, lookup};

/// Decode a UAS Datalink Local Set per MISB ST 0601.
///
/// Lenient: any 16-byte UL is accepted, the running-sum 16-bit checksum
/// (Tag 1) is verified, unknown tags are preserved verbatim in
/// [`UasDatalinkLs::unknown`], and per-tag value-validation failures are
/// collected in [`UasDatalinkLs::field_errors`] rather than failing the
/// whole record. Use [`decode_strict`] to additionally require the
/// ST 0601 family UL pattern, or [`decode_strict_compliance`] to enforce
/// the spec's mandatory tag-ordering rules.
///
/// # Example — decode a fixture and inspect typed fields
///
/// ```
/// use tst_core::klv::st0601;
/// use tst_core::UasDatalinkLs;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // Build a known-good fixture by round-tripping through the encoder.
/// let mut original = UasDatalinkLs::default();
/// original.timestamp_us = Some(1_700_000_000_000_000);
/// original.platform_heading_deg = Some(125.5);
/// original.sensor_lat_deg = Some(34.05);
/// original.sensor_lon_deg = Some(-118.25);
///
/// let bytes = st0601::encode_to_vec(&original)?;
/// let decoded = st0601::decode(&bytes)?;
///
/// // Integer tags round-trip exactly; IMAPB-encoded floats round-trip
/// // within the ST 0601 spec's bit-width quantization (~0.0055° at
/// // 16-bit heading resolution).
/// assert_eq!(decoded.timestamp_us, Some(1_700_000_000_000_000));
/// assert!((decoded.platform_heading_deg.unwrap() - 125.5).abs() < 0.01);
///
/// // No per-field decode failures on a well-formed record.
/// assert!(decoded.field_errors.is_empty());
/// // No unknown tags either — every emitted tag is in the typed table.
/// assert!(decoded.unknown.is_empty());
/// # Ok(())
/// # }
/// ```
pub fn decode(buf: &[u8]) -> Result<UasDatalinkLs, KlvDecodeError> {
    decode_inner(
        buf, /* verify_checksum */ true, /* strict_ul */ false,
    )
}

/// Decode without verifying the running-sum 16 checksum (Tag 1) and
/// without ST 0601-family UL gating. Use only when ingesting bytes that
/// are known to be intra-sender consistent (e.g. fixture round-trips
/// in tests).
///
/// # Errors
/// Returns the same structural [`KlvDecodeError`] variants as
/// [`decode`] — [`KlvDecodeError::Truncated`],
/// [`KlvDecodeError::MalformedLength`], [`KlvDecodeError::MalformedTag`],
/// [`KlvDecodeError::LengthOverflow`] — but never
/// [`KlvDecodeError::ChecksumMismatch`] or
/// [`KlvDecodeError::UnexpectedUniversalLabel`].
pub fn decode_unchecked(buf: &[u8]) -> Result<UasDatalinkLs, KlvDecodeError> {
    decode_inner(buf, false, false)
}

/// Strict decode: verify the running-sum 16 checksum (Tag 1) AND require
/// the buffer's 16-byte UL to fall within the ST 0601-family UL pattern.
///
/// # Errors
/// In addition to the structural variants from [`decode`]:
/// - [`KlvDecodeError::ChecksumMismatch`] when the declared Tag 1 value
///   does not match the recomputed running-sum.
/// - [`KlvDecodeError::UnexpectedUniversalLabel`] when the leading
///   16-byte UL is not in the ST 0601 family.
pub fn decode_strict(buf: &[u8]) -> Result<UasDatalinkLs, KlvDecodeError> {
    decode_inner(buf, true, true)
}

/// Strict ST 0601 compliance decode. In addition to checksum
/// verification (`decode`) and ST 0601-family UL gating (same
/// restriction as `decode_strict`), this also enforces the spec's
/// mandatory structure rules:
///
/// - ST 0601.8-09: Tag 2 (Precision Time Stamp) must be the first
///   element in the Local Set body.
/// - ST 0601.8-11: Tag 1 (Checksum) must be the last element.
/// - ST 0601.8-12: Tag 65 (UAS LS Version) must be present.
/// - ST 0601.13-24: each non-multiple ST 0601 item appears at most
///   once per packet. The strict walker rejects duplicate
///   occurrences of known typed tags via
///   [`KlvDecodeError::DuplicateTag`]. Unknown tags (outside the
///   typed table) are allowed to repeat: ST 0601.13 only mandates
///   once-per-packet for defined items.
/// - ST 0107.5 §6.3.1 / §6.3.2: BER-OID tag bytes and BER length
///   bytes must use canonical (fewest-bytes) encoding both for the
///   outer total-length and for every per-item TLV inside the body.
///   The strict walker uses [`read_ber_oid_strict`] +
///   [`read_ber_strict`] so a non-canonical encoding anywhere
///   inside the body trips
///   [`KlvDecodeError::NonCanonicalTag`] / [`KlvDecodeError::NonCanonicalLength`].
///
/// Use this only when validating compliance against published
/// captures or reference test vectors. Real-world captures from the
/// corpus often violate -09/-11/-12/-24 in benign ways; prefer `decode`
/// for production parsing.
///
/// # Errors
/// In addition to all variants from [`decode_strict`]:
/// - [`KlvDecodeError::Tag2NotFirst`] if Tag 2 is missing or not the
///   first body element.
/// - [`KlvDecodeError::Tag1NotLast`] if Tag 1 is missing or not the
///   final body element.
/// - [`KlvDecodeError::MissingTag65`] if the UAS LS Version tag is
///   absent.
/// - [`KlvDecodeError::DuplicateTag`] if a known typed tag appears
///   more than once in the body.
/// - [`KlvDecodeError::NonCanonicalLength`] or
///   [`KlvDecodeError::NonCanonicalTag`] if any BER or BER-OID
///   encoding in the outer length or the body is non-canonical.
pub fn decode_strict_compliance(buf: &[u8]) -> Result<UasDatalinkLs, KlvDecodeError> {
    // Step 1: walk the LS body strictly and record tag order WITHOUT
    // ST 0601 typed-decode. The walker uses the canonical-encoding
    // strict BER + BER-OID readers and tracks duplicate occurrences
    // of known typed tags. We need raw tag positions to enforce
    // ordering (ST 0601.8-09 / -11) and once-per-packet (ST 0601.13-24).
    if buf.len() < 16 {
        return Err(KlvDecodeError::Truncated {
            offset: 0,
            needed: 16,
            have: buf.len(),
        });
    }
    let (declared_len, after_len) = read_ber_strict(&buf[16..])?;
    if after_len.len() < declared_len {
        return Err(KlvDecodeError::Truncated {
            offset: buf.len() - after_len.len(),
            needed: declared_len,
            have: after_len.len(),
        });
    }
    let body = &after_len[..declared_len];
    // Offset of `body[0]` inside the original `buf`, so DuplicateTag/
    // truncation errors report a buf-relative offset matching the
    // permissive path.
    let body_offset_in_buf = buf.len() - after_len.len();
    let tag_order = strict_body_walk(body, body_offset_in_buf)?;
    if tag_order.first() != Some(&2) {
        return Err(KlvDecodeError::Tag2NotFirst);
    }
    if tag_order.last() != Some(&1) {
        return Err(KlvDecodeError::Tag1NotLast);
    }
    if !tag_order.contains(&65) {
        return Err(KlvDecodeError::MissingTag65);
    }
    // Step 2: delegate to existing strict decode (verifies checksum + UL
    // family). All the typed dispatch happens there. The body has
    // already cleared the strict-BER + duplicate gates above, so the
    // permissive iterator inside `decode_inner` will see the same
    // bytes without surfacing a stricter error.
    decode_inner(
        buf, /* verify_checksum */ true, /* strict_ul */ true,
    )
}

/// Walk an ST 0601 LS body using the canonical-encoding strict BER
/// readers, rejecting duplicate occurrences of known typed tags.
/// Returns the in-order list of tags encountered (used by the
/// caller for Tag 2/Tag 1/Tag 65 ordering checks).
///
/// `body_offset_in_buf` is the offset of `body[0]` within the
/// original outer buffer; surfaced through `DuplicateTag.offset`
/// and `Truncated.offset` so callers can locate the violation.
///
/// Duplicate detection is gated on `lookup(tag_u8).is_some()` —
/// ST 0601.13-24's once-per-packet rule applies only to defined
/// items. Unknown tags (including 2-byte BER-OID encoded tag IDs
/// beyond the 1-byte universe) are walked but never tracked.
fn strict_body_walk(body: &[u8], body_offset_in_buf: usize) -> Result<Vec<u32>, KlvDecodeError> {
    let mut tag_order: Vec<u32> = Vec::new();
    let mut seen = [false; 256];
    let mut offset = 0usize;
    while offset < body.len() {
        let item_start = offset;
        let rest = &body[item_start..];
        // Strict BER-OID tag.
        let (tag, after_tag) = match read_ber_oid_strict(rest) {
            Ok(v) => v,
            Err(mut e) => {
                if let KlvDecodeError::Truncated { offset: o, .. } = &mut e {
                    *o += body_offset_in_buf + item_start;
                }
                if let KlvDecodeError::NonCanonicalTag { offset: o } = &mut e {
                    *o += body_offset_in_buf + item_start;
                }
                if let KlvDecodeError::MalformedTag { offset: o } = &mut e {
                    *o += body_offset_in_buf + item_start;
                }
                return Err(e);
            }
        };
        let consumed_tag = rest.len() - after_tag.len();
        // Strict BER length.
        let (len, after_len) = match read_ber_strict(after_tag) {
            Ok(v) => v,
            Err(mut e) => {
                let inner_off = body_offset_in_buf + item_start + consumed_tag;
                if let KlvDecodeError::Truncated { offset: o, .. } = &mut e {
                    *o += inner_off;
                }
                if let KlvDecodeError::NonCanonicalLength { offset: o } = &mut e {
                    *o += inner_off;
                }
                if let KlvDecodeError::MalformedLength { offset: o } = &mut e {
                    *o += inner_off;
                }
                return Err(e);
            }
        };
        let consumed_len = after_tag.len() - after_len.len();
        if after_len.len() < len {
            return Err(KlvDecodeError::Truncated {
                offset: body_offset_in_buf + item_start + consumed_tag + consumed_len,
                needed: len,
                have: after_len.len(),
            });
        }
        // Duplicate-tag check (E1) — only meaningful for typed tags
        // that fit in u8 AND are present in the ST 0601 table.
        if let Ok(tag_u8) = u8::try_from(tag) {
            if lookup(tag_u8).is_some() {
                if seen[tag_u8 as usize] {
                    return Err(KlvDecodeError::DuplicateTag {
                        tag,
                        offset: body_offset_in_buf + item_start,
                    });
                }
                seen[tag_u8 as usize] = true;
            }
        }
        tag_order.push(tag);
        offset = item_start + consumed_tag + consumed_len + len;
    }
    Ok(tag_order)
}

fn decode_inner(
    buf: &[u8],
    verify_checksum: bool,
    strict_ul: bool,
) -> Result<UasDatalinkLs, KlvDecodeError> {
    if buf.len() < 16 {
        return Err(KlvDecodeError::Truncated {
            offset: 0,
            needed: 16,
            have: buf.len(),
        });
    }
    let mut ul_bytes = [0u8; 16];
    ul_bytes.copy_from_slice(&buf[..16]);
    let ul = UniversalLabel::new(ul_bytes);

    if strict_ul && !ul.is_st0601_family() {
        return Err(KlvDecodeError::UnexpectedUniversalLabel {
            expected: UniversalLabel::ST_0601_LS,
            found: ul,
        });
    }

    // Outer BER length
    let (declared_len, after_len) = read_ber(&buf[16..])?;
    let body_offset = buf.len() - after_len.len();
    if after_len.len() < declared_len {
        return Err(KlvDecodeError::Truncated {
            offset: body_offset,
            needed: declared_len,
            have: after_len.len(),
        });
    }
    let body = &after_len[..declared_len];

    let mut record = UasDatalinkLs {
        universal_label: ul,
        declared_version: ul.st0601_version_byte(),
        ..UasDatalinkLs::default()
    };

    let mut declared_checksum: Option<(u16, usize)> = None; // (value, offset_into_buf_of_value)

    for r in Iter::local_set(body) {
        let f = r?;
        if f.tag == 1 {
            // Checksum: capture for later verification.
            if f.value.len() != 2 {
                return Err(KlvDecodeError::Truncated {
                    offset: 0,
                    needed: 2,
                    have: f.value.len(),
                });
            }
            let cksum = u16::from_be_bytes([f.value[0], f.value[1]]);
            // Compute the byte offset of f.value within buf for checksum coverage.
            let value_offset_in_buf =
                (f.value.as_ptr() as usize).wrapping_sub(buf.as_ptr() as usize);
            declared_checksum = Some((cksum, value_offset_in_buf));
            continue;
        }
        if let Err(field_err) = apply_typed_tag(&mut record, &f) {
            record.field_errors.push(field_err);
        }
    }

    if verify_checksum {
        if let Some((expected, value_offset)) = declared_checksum {
            let computed = crate::klv::checksum::checksum_running_sum_16(&buf[..value_offset]);
            if computed != expected {
                return Err(KlvDecodeError::ChecksumMismatch {
                    expected,
                    found: computed,
                });
            }
        } else {
            // ST 0601 mandates Tag 1; treat absence as a structural error in
            // verifying modes. Permissive `decode_unchecked` skips this check.
            return Err(KlvDecodeError::Truncated {
                offset: body_offset,
                needed: 3,
                have: 0,
            });
        }
    }

    Ok(record)
}

fn apply_typed_tag(
    record: &mut UasDatalinkLs,
    f: &crate::klv::pack::RawField<'_>,
) -> Result<(), KlvFieldError> {
    let tag = f.tag;
    // Per MISB ST 0107.3-04 the decoder shall skip unknown LS values
    // without impacting the decoding of known items. ST 0107.5 §6.3.1
    // specifies BER-OID tags so the wire-format tag space is unlimited —
    // multi-byte tags up to u32 already arrive here via
    // `Iter::local_set`. Narrow only after rejecting tags outside the
    // typed table's u8 universe; otherwise a future tag 258 (= 0x102,
    // encoded `0x82 0x02`) would `as u8`-narrow to 2 and silently
    // collide with Tag 2 (Precision Time Stamp). The sibling ST 0102
    // decoder uses the same `u8::try_from` gate.
    let Ok(tag_u8) = u8::try_from(tag) else {
        record.unknown.push(OwnedRawField::from(f.clone()));
        return Ok(());
    };
    let Some(spec) = lookup(tag_u8) else {
        // Unknown tag — pass through.
        record.unknown.push(OwnedRawField::from(f.clone()));
        return Ok(());
    };
    match spec.encoding {
        Encoding::U8 => {
            if f.value.len() != 1 {
                return Err(KlvFieldError::InvalidLength {
                    tag,
                    expected: 1,
                    got: f.value.len(),
                });
            }
            let v = f.value[0];
            match tag {
                47 => record.generic_flag_data = Some(v),
                65 => record.uas_ls_version = Some(v),
                _ => unreachable!(),
            }
        }
        Encoding::U64 => {
            if f.value.len() != 8 {
                return Err(KlvFieldError::InvalidLength {
                    tag,
                    expected: 8,
                    got: f.value.len(),
                });
            }
            let mut a = [0u8; 8];
            a.copy_from_slice(f.value);
            let v = u64::from_be_bytes(a);
            match tag {
                2 => record.timestamp_us = Some(v),
                _ => unreachable!(),
            }
        }
        Encoding::Utf8 { max_bytes } => {
            if f.value.len() > max_bytes {
                return Err(KlvFieldError::InvalidLength {
                    tag,
                    expected: max_bytes,
                    got: f.value.len(),
                });
            }
            let s = core::str::from_utf8(f.value)
                .map_err(|_| KlvFieldError::InvalidUtf8 { tag })?
                .to_owned();
            match tag {
                3 => record.mission_id = Some(s),
                4 => record.platform_tail_number = Some(s),
                10 => record.platform_designation = Some(s),
                11 => record.image_source_sensor = Some(s),
                12 => record.image_coordinate_system = Some(s),
                59 => record.platform_call_sign = Some(s),
                _ => unreachable!(),
            }
        }
        Encoding::RawBytes => match tag {
            48 => record.security_local_set = Some(f.value.to_vec()),
            74 => record.vmti = Some(f.value.to_vec()),
            _ => unreachable!(),
        },
        Encoding::U8Range
        | Encoding::U16Range
        | Encoding::I16Range
        | Encoding::U32Range
        | Encoding::I32Range => {
            let r = spec.range.as_ref().expect("ranged tag has range");
            let v = decode_fixed_range(r, tag, f.value)?;
            assign_ranged(record, tag, v);
        }
    }
    Ok(())
}

/// Per-tag accessor for the 39 uniform `Option<f64>` ranged fields.
///
/// Drives `assign_ranged` (decode) and the ranged arms of
/// `encode_tag_value` / `each_typed_field` (encode) from one place,
/// eliminating three parallel 39-arm dispatch tables. `fn` pointers are
/// `const`-safe and work in `no_std` context; field names remain greppable.
pub(super) struct RangedEntry {
    pub(super) id: u8,
    pub(super) get: fn(&UasDatalinkLs) -> Option<f64>,
    pub(super) set: fn(&mut UasDatalinkLs, f64),
}

/// All 39 ST 0601 ranged `Option<f64>` fields in tag-ascending order.
/// The encode path derives value_len from `tags::TAGS[id].range.byte_length`;
/// the decode path calls `set`; the encode path calls `get`.
pub(super) static RANGED_FIELDS: &[RangedEntry] = &[
    RangedEntry { id: 5,  get: |r| r.platform_heading_deg,               set: |r, v| r.platform_heading_deg = Some(v) },
    RangedEntry { id: 6,  get: |r| r.platform_pitch_deg,                 set: |r, v| r.platform_pitch_deg = Some(v) },
    RangedEntry { id: 7,  get: |r| r.platform_roll_deg,                  set: |r, v| r.platform_roll_deg = Some(v) },
    RangedEntry { id: 8,  get: |r| r.platform_true_airspeed,             set: |r, v| r.platform_true_airspeed = Some(v) },
    RangedEntry { id: 9,  get: |r| r.platform_indicated_airspeed,        set: |r, v| r.platform_indicated_airspeed = Some(v) },
    RangedEntry { id: 13, get: |r| r.sensor_lat_deg,                     set: |r, v| r.sensor_lat_deg = Some(v) },
    RangedEntry { id: 14, get: |r| r.sensor_lon_deg,                     set: |r, v| r.sensor_lon_deg = Some(v) },
    RangedEntry { id: 15, get: |r| r.sensor_alt_m,                       set: |r, v| r.sensor_alt_m = Some(v) },
    RangedEntry { id: 16, get: |r| r.sensor_hfov_deg,                    set: |r, v| r.sensor_hfov_deg = Some(v) },
    RangedEntry { id: 17, get: |r| r.sensor_vfov_deg,                    set: |r, v| r.sensor_vfov_deg = Some(v) },
    RangedEntry { id: 18, get: |r| r.sensor_rel_az_deg,                  set: |r, v| r.sensor_rel_az_deg = Some(v) },
    RangedEntry { id: 19, get: |r| r.sensor_rel_el_deg,                  set: |r, v| r.sensor_rel_el_deg = Some(v) },
    RangedEntry { id: 20, get: |r| r.sensor_rel_roll_deg,                set: |r, v| r.sensor_rel_roll_deg = Some(v) },
    RangedEntry { id: 21, get: |r| r.slant_range_m,                      set: |r, v| r.slant_range_m = Some(v) },
    RangedEntry { id: 22, get: |r| r.target_width_m,                     set: |r, v| r.target_width_m = Some(v) },
    RangedEntry { id: 23, get: |r| r.frame_center_lat_deg,               set: |r, v| r.frame_center_lat_deg = Some(v) },
    RangedEntry { id: 24, get: |r| r.frame_center_lon_deg,               set: |r, v| r.frame_center_lon_deg = Some(v) },
    RangedEntry { id: 25, get: |r| r.frame_center_elev_m,                set: |r, v| r.frame_center_elev_m = Some(v) },
    RangedEntry { id: 26, get: |r| r.corner_lat_offset_p1_deg,           set: |r, v| r.corner_lat_offset_p1_deg = Some(v) },
    RangedEntry { id: 27, get: |r| r.corner_lon_offset_p1_deg,           set: |r, v| r.corner_lon_offset_p1_deg = Some(v) },
    RangedEntry { id: 28, get: |r| r.corner_lat_offset_p2_deg,           set: |r, v| r.corner_lat_offset_p2_deg = Some(v) },
    RangedEntry { id: 29, get: |r| r.corner_lon_offset_p2_deg,           set: |r, v| r.corner_lon_offset_p2_deg = Some(v) },
    RangedEntry { id: 30, get: |r| r.corner_lat_offset_p3_deg,           set: |r, v| r.corner_lat_offset_p3_deg = Some(v) },
    RangedEntry { id: 31, get: |r| r.corner_lon_offset_p3_deg,           set: |r, v| r.corner_lon_offset_p3_deg = Some(v) },
    RangedEntry { id: 32, get: |r| r.corner_lat_offset_p4_deg,           set: |r, v| r.corner_lat_offset_p4_deg = Some(v) },
    RangedEntry { id: 33, get: |r| r.corner_lon_offset_p4_deg,           set: |r, v| r.corner_lon_offset_p4_deg = Some(v) },
    RangedEntry { id: 50, get: |r| r.platform_angle_of_attack_deg,       set: |r, v| r.platform_angle_of_attack_deg = Some(v) },
    RangedEntry { id: 75, get: |r| r.sensor_ellipsoid_height_m,          set: |r, v| r.sensor_ellipsoid_height_m = Some(v) },
    RangedEntry { id: 78, get: |r| r.frame_center_ellipsoid_height_m,    set: |r, v| r.frame_center_ellipsoid_height_m = Some(v) },
    RangedEntry { id: 82, get: |r| r.corner_lat_p1_deg,                  set: |r, v| r.corner_lat_p1_deg = Some(v) },
    RangedEntry { id: 83, get: |r| r.corner_lon_p1_deg,                  set: |r, v| r.corner_lon_p1_deg = Some(v) },
    RangedEntry { id: 84, get: |r| r.corner_lat_p2_deg,                  set: |r, v| r.corner_lat_p2_deg = Some(v) },
    RangedEntry { id: 85, get: |r| r.corner_lon_p2_deg,                  set: |r, v| r.corner_lon_p2_deg = Some(v) },
    RangedEntry { id: 86, get: |r| r.corner_lat_p3_deg,                  set: |r, v| r.corner_lat_p3_deg = Some(v) },
    RangedEntry { id: 87, get: |r| r.corner_lon_p3_deg,                  set: |r, v| r.corner_lon_p3_deg = Some(v) },
    RangedEntry { id: 88, get: |r| r.corner_lat_p4_deg,                  set: |r, v| r.corner_lat_p4_deg = Some(v) },
    RangedEntry { id: 89, get: |r| r.corner_lon_p4_deg,                  set: |r, v| r.corner_lon_p4_deg = Some(v) },
    RangedEntry { id: 90, get: |r| r.platform_pitch_full_deg,            set: |r, v| r.platform_pitch_full_deg = Some(v) },
    RangedEntry { id: 91, get: |r| r.platform_roll_full_deg,             set: |r, v| r.platform_roll_full_deg = Some(v) },
];

/// Write a decoded ranged-float value to the matching field in `record`.
/// Replaces a 39-arm match — the table is the single source of the tag→field mapping.
pub(super) fn assign_ranged(record: &mut UasDatalinkLs, tag: u32, v: f64) {
    if let Some(entry) = RANGED_FIELDS.iter().find(|e| e.id as u32 == tag) {
        (entry.set)(record, v);
    }
}
