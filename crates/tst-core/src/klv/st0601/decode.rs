//! ST 0601 decode entry points — 4 modes per the typed-set's
//! lenient/strict/compliance lineage.

use crate::error::{KlvDecodeError, KlvFieldError};
use crate::klv::length::{read_ber, read_ber_strict};
use crate::klv::pack::{Iter, OwnedRawField};
use crate::klv::universal_label::UniversalLabel;

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
/// - ST 0107.5 §6.3.2: outer BER length encoding must be canonical
///   (fewest-bytes). The body iteration via `Iter::local_set` remains
///   permissive on per-tag BER encoding for now.
///
/// Use this only when validating compliance against published
/// captures or reference test vectors. Real-world captures from the
/// corpus often violate -09/-11/-12 in benign ways; prefer `decode`
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
/// - [`KlvDecodeError::NonCanonicalLength`] if the outer BER length is
///   not encoded with the fewest bytes per ST 0107.5 §6.3.2.
pub fn decode_strict_compliance(buf: &[u8]) -> Result<UasDatalinkLs, KlvDecodeError> {
    // Step 1: walk the LS body and record tag order WITHOUT ST 0601
    // typed-decode. We need raw tag positions to enforce ordering.
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
    let mut tag_order: Vec<u32> = Vec::new();
    for r in Iter::local_set(body) {
        let f = r?;
        tag_order.push(f.tag);
    }
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
    // family). All the typed dispatch happens there.
    decode_inner(
        buf, /* verify_checksum */ true, /* strict_ul */ true,
    )
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
        declared_version: ul.version_byte(),
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
            let s = std::str::from_utf8(f.value)
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

pub(super) fn assign_ranged(record: &mut UasDatalinkLs, tag: u32, v: f64) {
    match tag {
        5 => record.platform_heading_deg = Some(v),
        6 => record.platform_pitch_deg = Some(v),
        7 => record.platform_roll_deg = Some(v),
        8 => record.platform_true_airspeed = Some(v),
        9 => record.platform_indicated_airspeed = Some(v),
        13 => record.sensor_lat_deg = Some(v),
        14 => record.sensor_lon_deg = Some(v),
        15 => record.sensor_alt_m = Some(v),
        16 => record.sensor_hfov_deg = Some(v),
        17 => record.sensor_vfov_deg = Some(v),
        18 => record.sensor_rel_az_deg = Some(v),
        19 => record.sensor_rel_el_deg = Some(v),
        20 => record.sensor_rel_roll_deg = Some(v),
        21 => record.slant_range_m = Some(v),
        22 => record.target_width_m = Some(v),
        23 => record.frame_center_lat_deg = Some(v),
        24 => record.frame_center_lon_deg = Some(v),
        25 => record.frame_center_elev_m = Some(v),
        26 => record.corner_lat_offset_p1_deg = Some(v),
        27 => record.corner_lon_offset_p1_deg = Some(v),
        28 => record.corner_lat_offset_p2_deg = Some(v),
        29 => record.corner_lon_offset_p2_deg = Some(v),
        30 => record.corner_lat_offset_p3_deg = Some(v),
        31 => record.corner_lon_offset_p3_deg = Some(v),
        32 => record.corner_lat_offset_p4_deg = Some(v),
        33 => record.corner_lon_offset_p4_deg = Some(v),
        50 => record.platform_angle_of_attack_deg = Some(v),
        82 => record.corner_lat_p1_deg = Some(v),
        83 => record.corner_lon_p1_deg = Some(v),
        84 => record.corner_lat_p2_deg = Some(v),
        85 => record.corner_lon_p2_deg = Some(v),
        86 => record.corner_lat_p3_deg = Some(v),
        87 => record.corner_lon_p3_deg = Some(v),
        88 => record.corner_lat_p4_deg = Some(v),
        89 => record.corner_lon_p4_deg = Some(v),
        75 => record.sensor_ellipsoid_height_m = Some(v),
        78 => record.frame_center_ellipsoid_height_m = Some(v),
        90 => record.platform_pitch_full_deg = Some(v),
        91 => record.platform_roll_full_deg = Some(v),
        _ => {}
    }
}
