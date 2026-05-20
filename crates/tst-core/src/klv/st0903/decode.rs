//! ST 0903 decode entry points (`decode`, `decode_strict`) and private
//! helpers (`decode_vtarget_series`, `decode_vtarget_series_strict`).

use crate::error::{KlvDecodeError, KlvFieldError};
use crate::klv::pack::OwnedRawField;
use crate::klv::st0903::model::VmtiLs;
use crate::klv::st0903::tags::{Encoding, TAGS, lookup};
use crate::klv::st0903::var_uint;
use crate::klv::st0903::vtarget_pack;

/// Lenient decode of a VMTI Local Set body per ST 0903.6 §10.1.
///
/// Tolerates structurally recoverable malformations: per-field byte-length
/// mismatches, malformed UTF-8, malformed IMAPB ranges, and truncated value
/// bytes are accumulated in [`VmtiLs::field_errors`] without aborting the
/// walk. Tags not in the §10.1 schema (unknown / deprecated, e.g. Tag 7)
/// are preserved in [`VmtiLs::unknown`] per ST 0107.5 §6 future-proof skip
/// rule.
///
/// VTargetSeries (Tag 101) inner packs are dispatched to
/// `vtarget_pack::read_pack`; pack-level errors land in `field_errors`
/// as [`KlvDecodeError::St0903InvalidVTargetPack`] reasons too — but
/// surfaced via the parent-level [`KlvFieldError`] channel (see
/// `decode_vtarget_series`).
///
/// Use [`decode_strict`] for spec-validation use cases that
/// reject any of the above.
///
/// # Errors
/// Returns `Err(KlvDecodeError)` only for unrecoverable framing
/// failures inside [`VTargetSeries`][VmtiLs::targets] (Tag 101) — a
/// VTargetPack header that cannot be parsed surfaces as
/// [`KlvDecodeError::St0903InvalidVTargetPack`]. Tag-level
/// malformations on the parent LS are non-fatal in lenient mode and
/// are captured in [`VmtiLs::field_errors`] instead.
pub fn decode(bytes: &[u8]) -> Result<VmtiLs, KlvDecodeError> {
    use crate::klv::imapb::{ImapbParams, decode_imapb};
    use crate::klv::length::read_ber;

    let mut ls = VmtiLs::default();
    let mut cursor = bytes;

    while !cursor.is_empty() {
        // Single-byte BER-OID tag — ST 0903.6 §10.1 IDs (1..=103) all
        // fit in one byte where the encoded form == raw byte. Matches
        // the substrate-wide convention used by `klv::st0102` /
        // `klv::st0601` LS body walks.
        let tag = cursor[0];
        cursor = &cursor[1..];

        // BER outer length. A framing failure here is unrecoverable
        // for the rest of the buffer (we can't find the next tag), so
        // record it and stop walking — surface partial state to the
        // caller.
        let (declared_len, after_len) = match read_ber(cursor) {
            Ok(x) => x,
            Err(_) => {
                ls.field_errors
                    .push(KlvFieldError::TruncatedField { tag: tag as u32 });
                break;
            }
        };
        cursor = after_len;

        if cursor.len() < declared_len {
            ls.field_errors
                .push(KlvFieldError::TruncatedField { tag: tag as u32 });
            break;
        }
        let value = &cursor[..declared_len];
        cursor = &cursor[declared_len..];

        let Some(spec) = lookup(tag) else {
            // Unknown / deprecated tag — preserve per ST 0107.5 §6.
            ls.unknown.push(OwnedRawField {
                tag: tag as u32,
                value: value.to_vec(),
            });
            continue;
        };

        match spec.encoding {
            Encoding::U16Be => {
                if value.len() != 2 {
                    ls.field_errors.push(KlvFieldError::InvalidLength {
                        tag: tag as u32,
                        expected: 2,
                        got: value.len(),
                    });
                    continue;
                }
                let v = u16::from_be_bytes([value[0], value[1]]);
                debug_assert_eq!(tag, 1, "U16Be reserved for tag 1 (Checksum)");
                ls.checksum = Some(v);
            }
            Encoding::U64Be => {
                if value.len() != 8 {
                    ls.field_errors.push(KlvFieldError::InvalidLength {
                        tag: tag as u32,
                        expected: 8,
                        got: value.len(),
                    });
                    continue;
                }
                // Defense-in-depth: the upstream length check above
                // makes this `try_into` infallible today, but a future
                // tag-table refactor that decouples encoding from
                // length could reach this with a wrong-sized slice.
                // Surface as TruncatedField via field_errors rather
                // than panic.
                let arr: [u8; 8] = match value.try_into() {
                    Ok(a) => a,
                    Err(_) => {
                        ls.field_errors
                            .push(KlvFieldError::TruncatedField { tag: tag as u32 });
                        continue;
                    }
                };
                debug_assert_eq!(tag, 2, "U64Be reserved for tag 2 (Precision Time Stamp)");
                ls.precision_time_stamp = Some(u64::from_be_bytes(arr));
            }
            Encoding::VarUint { max_bytes } => {
                if value.is_empty() || value.len() > max_bytes as usize {
                    ls.field_errors.push(KlvFieldError::InvalidLength {
                        tag: tag as u32,
                        expected: max_bytes as usize,
                        got: value.len(),
                    });
                    continue;
                }
                let v = match var_uint::read_var_u32(value) {
                    Ok(v) => v,
                    Err(_) => {
                        ls.field_errors
                            .push(KlvFieldError::TruncatedField { tag: tag as u32 });
                        continue;
                    }
                };
                match tag {
                    4 => ls.version_number = Some(v as u16), // V2 caps at u16
                    5 => ls.total_targets_in_frame = Some(v),
                    6 => ls.num_targets_reported = Some(v),
                    8 => ls.frame_width = Some(v),
                    9 => ls.frame_height = Some(v),
                    _ => unreachable!("VarUint dispatch missing tag {tag}"),
                }
            }
            Encoding::Utf8 { max_chars } => {
                let s = match std::str::from_utf8(value) {
                    Ok(s) => s,
                    Err(_) => {
                        ls.field_errors
                            .push(KlvFieldError::InvalidUtf8 { tag: tag as u32 });
                        continue;
                    }
                };
                // §10.1.3 / §10.1.10 char caps (V32 / V128 — characters,
                // not bytes; UTF-8 expansion may exceed the byte count).
                // Lenient: surface a field_error if too long, but keep
                // the value (strict mode rejects in Task 6).
                if s.chars().count() > max_chars {
                    ls.field_errors.push(KlvFieldError::InvalidLength {
                        tag: tag as u32,
                        expected: max_chars,
                        got: s.chars().count(),
                    });
                }
                let owned = s.to_string();
                match tag {
                    3 => ls.vmti_system_name = Some(owned),
                    10 => ls.source_sensor = Some(owned),
                    _ => unreachable!("Utf8 dispatch missing tag {tag}"),
                }
            }
            Encoding::ImapbF64 { min, max } => {
                // Top-level Tags 11 + 12 (Horizontal/Vertical FOV) are
                // IMAPB(0, 180, 2) per ST 0903.6 §10.1.11 + §10.1.12 —
                // both fixed length 2. Hardcode length=2 to (a) prevent
                // the imapb substrate's length-0 panic on malformed
                // wire input (`read_signed_be(&[])` underflows
                // `n*8-1` when `n==0`) and (b) match the spec's wire
                // shape. Mirrors the per-tag hardcoded-length pattern
                // in `vtarget_pack::decode_field`.
                let expected_len = 2;
                if value.len() != expected_len {
                    ls.field_errors.push(KlvFieldError::InvalidLength {
                        tag: tag as u32,
                        expected: expected_len,
                        got: value.len(),
                    });
                    continue;
                }
                let params = ImapbParams {
                    min,
                    max,
                    length: expected_len,
                };
                // A7: decode_imapb returns DecodedImapb (special values + bounds
                // check per ST 1201.5 §7.2.2 step 1 + §7.2.3). The lenient
                // top-level walker treats special values and out-of-range as
                // "field unavailable" — surface InvalidLength as a generic
                // field error and continue. Sites that want to differentiate
                // +∞ / NaN / BelowMin / AboveMax can pattern-match the enum.
                let v = match decode_imapb(&params, value) {
                    Ok(decoded) => match decoded.value() {
                        Some(v) => v,
                        None => {
                            ls.field_errors.push(KlvFieldError::InvalidLength {
                                tag: tag as u32,
                                expected: expected_len,
                                got: value.len(),
                            });
                            continue;
                        }
                    },
                    Err(_) => {
                        ls.field_errors.push(KlvFieldError::InvalidLength {
                            tag: tag as u32,
                            expected: expected_len,
                            got: value.len(),
                        });
                        continue;
                    }
                };
                match tag {
                    11 => ls.horizontal_fov = Some(v),
                    12 => ls.vertical_fov = Some(v),
                    _ => unreachable!("ImapbF64 dispatch missing tag {tag}"),
                }
            }
            Encoding::RawBytes => match tag {
                13 => ls.miis_id = Some(value.to_vec()),
                101 => {
                    ls.targets = decode_vtarget_series(value, &mut ls.field_errors);
                }
                102 => ls.algorithm_series = Some(value.to_vec()),
                103 => ls.ontology_series = Some(value.to_vec()),
                _ => unreachable!("RawBytes dispatch missing tag {tag}"),
            },
        }
    }

    Ok(ls)
}

/// Walk a VTargetSeries (Tag 101) payload, dispatching each
/// BER-length-prefixed pack to [`vtarget_pack::read_pack`]. Pack-level
/// failures are recorded in `field_errors` as `OutOfRange` placeholders
/// keyed to tag 101 (until/unless a future variant carrying the typed
/// `VTargetPackError` lands in `KlvFieldError`).
///
/// Series framing failures (truncated outer BER length, value-length
/// overrun) abort the walk and emit a single `TruncatedField { tag: 101 }`.
fn decode_vtarget_series(
    series_bytes: &[u8],
    field_errors: &mut Vec<KlvFieldError>,
) -> Vec<vtarget_pack::VTargetPack> {
    use crate::klv::length::read_ber;

    let mut targets = Vec::new();
    let mut cursor = series_bytes;
    while !cursor.is_empty() {
        let (pack_len, after_len) = match read_ber(cursor) {
            Ok(x) => x,
            Err(_) => {
                field_errors.push(KlvFieldError::TruncatedField { tag: 101 });
                break;
            }
        };
        cursor = after_len;
        if cursor.len() < pack_len {
            field_errors.push(KlvFieldError::TruncatedField { tag: 101 });
            break;
        }
        let pack_bytes = &cursor[..pack_len];
        cursor = &cursor[pack_len..];

        match vtarget_pack::read_pack(pack_bytes) {
            Ok((pack, _)) => targets.push(pack),
            Err(_) => {
                // Pack-level malformation. We don't have a typed
                // `KlvFieldError` arm carrying the `VTargetPackError`
                // shape, so signal via a generic field error keyed to
                // tag 101. Strict-mode will route the typed error via
                // `KlvDecodeError::St0903InvalidVTargetPack`.
                field_errors.push(KlvFieldError::TruncatedField { tag: 101 });
            }
        }
    }
    targets
}

/// Strict decode of a VMTI Local Set body per ST 0903.6 §10.1.
///
/// Rejects spec-violating input rather than recovering: missing
/// unconditionally-required tags (Tags 4 and 6 per ST 0903.5-99 +
/// ST 0903.4-19), duplicate tags, malformed UTF-8, IMAPB length
/// mismatches, VarUint length-zero / overflow, U16Be / U64Be wrong
/// width, BER framing failures, and pack-level malformations
/// (routed via [`KlvDecodeError::St0903InvalidVTargetPack`]).
///
/// Unknown tags are still preserved in [`VmtiLs::unknown`] per
/// ST 0107.5 §6 future-proof skip rule (strict mode is about
/// codepoint legality, not future-spec rejection).
///
/// **Conditional requirements not enforced.** ST 0903.6 marks several
/// tags as required only for specific carriage paths (Tag 1 Checksum
/// required for standalone-VMTI per ST 0903.6-119, prohibited for
/// embedded per -120; Tag 2 PTS required for standalone per -117;
/// Tags 11/12/13 conditional). `decode_strict` does NOT enforce these
/// rules — consumers needing carriage-aware validation can post-
/// validate after a successful decode.
///
/// **UTF-8 char-cap.** Strict rejects strings exceeding the spec's
/// `max_chars` (Tag 3 V32, Tag 10 V128). Lenient accepts but surfaces
/// a `field_error`.
///
/// # Errors
/// - [`KlvDecodeError::DuplicateTag`] if any tag appears more than
///   once.
/// - [`KlvDecodeError::Truncated`] if a TLV's declared length runs
///   past the end of the buffer.
/// - [`KlvDecodeError::St0903MissingRequiredTag`] if a spec-required
///   tag (4 or 6 per ST 0903.6) is absent.
/// - [`KlvDecodeError::St0903InvalidVTargetPack`] for unrecoverable
///   pack-internal malformations under VTargetSeries (Tag 101).
/// - [`KlvDecodeError::FieldError`] wrapping a [`KlvFieldError`] for
///   any per-tag value validation failure (length / range / UTF-8 /
///   IMAPB / codepoint).
/// - [`KlvDecodeError::NonCanonicalLength`] if a TLV uses a non-
///   canonical BER length encoding.
pub fn decode_strict(bytes: &[u8]) -> Result<VmtiLs, KlvDecodeError> {
    use crate::klv::imapb::{ImapbParams, decode_imapb};
    use crate::klv::length::read_ber_strict;

    let mut ls = VmtiLs::default();
    let mut cursor = bytes;
    let mut seen = [false; 256];

    while !cursor.is_empty() {
        let tag = cursor[0];
        cursor = &cursor[1..];

        if seen[tag as usize] {
            return Err(KlvDecodeError::DuplicateTag {
                tag: tag as u32,
                offset: 0, // single-pass walk doesn't track buffer offset
            });
        }
        seen[tag as usize] = true;

        let (declared_len, after_len) = read_ber_strict(cursor)?;
        cursor = after_len;
        if cursor.len() < declared_len {
            return Err(KlvDecodeError::Truncated {
                offset: 0,
                needed: declared_len,
                have: cursor.len(),
            });
        }
        let value = &cursor[..declared_len];
        cursor = &cursor[declared_len..];

        let Some(spec) = lookup(tag) else {
            // ST 0107.5 §6 skip rule — preserve unknown tags.
            ls.unknown.push(OwnedRawField {
                tag: tag as u32,
                value: value.to_vec(),
            });
            continue;
        };

        match spec.encoding {
            Encoding::U16Be => {
                if value.len() != 2 {
                    return Err(KlvDecodeError::FieldError(KlvFieldError::InvalidLength {
                        tag: tag as u32,
                        expected: 2,
                        got: value.len(),
                    }));
                }
                debug_assert_eq!(tag, 1, "U16Be reserved for tag 1 (Checksum)");
                ls.checksum = Some(u16::from_be_bytes([value[0], value[1]]));
            }
            Encoding::U64Be => {
                if value.len() != 8 {
                    return Err(KlvDecodeError::FieldError(KlvFieldError::InvalidLength {
                        tag: tag as u32,
                        expected: 8,
                        got: value.len(),
                    }));
                }
                debug_assert_eq!(tag, 2, "U64Be reserved for tag 2 (Precision Time Stamp)");
                // Defense-in-depth: the upstream length check above
                // makes this `try_into` infallible today, but a future
                // tag-table refactor that decouples encoding from
                // length could reach this with a wrong-sized slice.
                // Surface as TruncatedField rather than panic.
                let arr: [u8; 8] = value.try_into().map_err(|_| {
                    KlvDecodeError::FieldError(KlvFieldError::TruncatedField { tag: tag as u32 })
                })?;
                ls.precision_time_stamp = Some(u64::from_be_bytes(arr));
            }
            Encoding::VarUint { max_bytes } => {
                if value.is_empty() || value.len() > max_bytes as usize {
                    return Err(KlvDecodeError::FieldError(KlvFieldError::InvalidLength {
                        tag: tag as u32,
                        expected: max_bytes as usize,
                        got: value.len(),
                    }));
                }
                let v = var_uint::read_var_u32(value).map_err(KlvDecodeError::FieldError)?;
                match tag {
                    4 => ls.version_number = Some(v as u16), // V2 caps at u16
                    5 => ls.total_targets_in_frame = Some(v),
                    6 => ls.num_targets_reported = Some(v),
                    8 => ls.frame_width = Some(v),
                    9 => ls.frame_height = Some(v),
                    _ => unreachable!("VarUint dispatch missing tag {tag}"),
                }
            }
            Encoding::Utf8 { max_chars } => {
                let s = std::str::from_utf8(value).map_err(|_| {
                    KlvDecodeError::FieldError(KlvFieldError::InvalidUtf8 { tag: tag as u32 })
                })?;
                if s.chars().count() > max_chars {
                    return Err(KlvDecodeError::FieldError(KlvFieldError::InvalidLength {
                        tag: tag as u32,
                        expected: max_chars,
                        got: s.chars().count(),
                    }));
                }
                let owned = s.to_string();
                match tag {
                    3 => ls.vmti_system_name = Some(owned),
                    10 => ls.source_sensor = Some(owned),
                    _ => unreachable!("Utf8 dispatch missing tag {tag}"),
                }
            }
            Encoding::ImapbF64 { min, max } => {
                // Top-level Tags 11 + 12 are IMAPB(0, 180, 2) per
                // ST 0903.6 §10.1.11 + §10.1.12 — both fixed length 2.
                let expected_len = 2;
                if value.len() != expected_len {
                    return Err(KlvDecodeError::FieldError(KlvFieldError::InvalidLength {
                        tag: tag as u32,
                        expected: expected_len,
                        got: value.len(),
                    }));
                }
                let params = ImapbParams {
                    min,
                    max,
                    length: expected_len,
                };
                // A7: strict mode rejects special values + out-of-range as
                // InvalidLength (the most caller-friendly map onto existing
                // error vocabulary). Future tightening could promote these
                // to dedicated KlvFieldError variants.
                let decoded = decode_imapb(&params, value).map_err(KlvDecodeError::FieldError)?;
                let v = decoded.value().ok_or(KlvDecodeError::FieldError(
                    KlvFieldError::InvalidLength {
                        tag: tag as u32,
                        expected: expected_len,
                        got: value.len(),
                    },
                ))?;
                match tag {
                    11 => ls.horizontal_fov = Some(v),
                    12 => ls.vertical_fov = Some(v),
                    _ => unreachable!("ImapbF64 dispatch missing tag {tag}"),
                }
            }
            Encoding::RawBytes => match tag {
                13 => ls.miis_id = Some(value.to_vec()),
                101 => {
                    ls.targets = decode_vtarget_series_strict(value)?;
                }
                102 => ls.algorithm_series = Some(value.to_vec()),
                103 => ls.ontology_series = Some(value.to_vec()),
                _ => unreachable!("RawBytes dispatch missing tag {tag}"),
            },
        }
    }

    // Required-tag validation per Task 2's audit: required = {4, 6}.
    for spec in TAGS {
        if spec.required && !seen[spec.id as usize] {
            return Err(KlvDecodeError::St0903MissingRequiredTag { tag: spec.id });
        }
    }

    Ok(ls)
}

/// Strict variant of [`decode_vtarget_series`]: framing failures and
/// pack-level malformations abort with an `Err`. The pack-level error
/// is routed via the typed [`KlvDecodeError::St0903InvalidVTargetPack`]
/// arm carrying the underlying [`VTargetPackError`].
fn decode_vtarget_series_strict(
    series_bytes: &[u8],
) -> Result<Vec<vtarget_pack::VTargetPack>, KlvDecodeError> {
    use crate::klv::length::read_ber_strict;

    let mut targets = Vec::new();
    let mut cursor = series_bytes;
    let mut offset = 0usize;
    while !cursor.is_empty() {
        let before_len = cursor.len();
        let (pack_len, after_len) = read_ber_strict(cursor)?;
        let len_consumed = before_len - after_len.len();
        cursor = after_len;
        offset += len_consumed;
        if cursor.len() < pack_len {
            return Err(KlvDecodeError::Truncated {
                offset,
                needed: pack_len,
                have: cursor.len(),
            });
        }
        let pack_bytes = &cursor[..pack_len];
        cursor = &cursor[pack_len..];

        let (pack, _) = vtarget_pack::read_pack(pack_bytes)
            .map_err(|reason| KlvDecodeError::St0903InvalidVTargetPack { offset, reason })?;
        targets.push(pack);
        offset += pack_len;
    }
    Ok(targets)
}
