//! MISB ST 0903.6 VMTI (Video Moving Target Indicator) Local Set typed layer.
//!
//! Sibling typed parser to [`crate::klv::st0601`]. Consumers who decode
//! a `UasDatalinkLs` and want typed access to the inner VMTI LS call
//! [`decode`] (or [`decode_strict`]) on `record.vmti.as_deref()?`.
//!
//! Two decode entry points:
//! - [`decode`] — lenient: tolerates missing tags, unknown tags
//!   (preserved in `unknown`), malformed sub-records (preserved in
//!   `field_errors`).
//! - [`decode_strict`] — strict: rejects missing required tags
//!   (per ST 0903.6 §6 Table 1), duplicate tags, malformed UTF-8,
//!   pack-level malformations. Unknown tags are still preserved per
//!   ST 0107.5 §6 future-proof skip rule.
//!
//! Encode is symmetric — decode + encode bit-identical round-trips for
//! all spec-conformant input.
//!
//! 7 nested/sibling Local Sets (VMask, VObject, VFeature, VTracker,
//! VChip on each `VTargetPack`; Algorithm Series and Ontology Series at
//! the VMTI top level) stay as `Option<Vec<u8>>` pass-through bytes —
//! typed layers deferred (see `docs/deferred-features.md`).
//!
//! Universal Set form of ST 0903 is out of scope (LS-only on
//! MPEG-TS+KLV streams).
//!
//! # Carriage paths
//!
//! VMTI rides two ways in the wild:
//!
//! 1. **Nested inside ST 0601 as Tag 74** — most common; encoder bundles
//!    VMTI alongside platform telemetry. Consumer pattern:
//!    ```ignore
//!    let uas = klv::st0601::decode(bytes)?;
//!    if let Some(vmti_bytes) = uas.vmti.as_deref() {
//!        let vmti = klv::st0903::decode(vmti_bytes)?;
//!        // ...
//!    }
//!    ```
//! 2. **Standalone on its own KLV PID** — the AU-cell payload is a VMTI
//!    LS with [`VMTI_LS_UL`] as the 16-byte UL prefix. Consumer pattern:
//!    ```ignore
//!    if data.starts_with(&klv::st0903::VMTI_LS_UL) {
//!        let (_outer_len, body) = klv::length::read_ber(&data[16..])?;
//!        let vmti = klv::st0903::decode(body)?;
//!        // ...
//!    }
//!    ```
//!    The demuxer remains UL-agnostic; consumer-side dispatch keeps
//!    new typed-set additions from creating a coupling load on the
//!    demuxer.

pub(crate) mod emit;
pub(crate) mod enums;
pub(crate) mod tags;
pub(crate) mod var_uint;
pub(crate) mod vtarget_pack;

pub use vtarget_pack::{VTargetPack, VTargetPackError};

use crate::error::{KlvDecodeError, KlvEncodeError, KlvFieldError};
use crate::klv::pack::OwnedRawField;

/// MISB ST 0903.6 §6.1 — VMTI Local Set Universal Label.
/// Used by consumers carrying VMTI as its own KLV stream (separate
/// MPEG-TS PID, not nested in an ST 0601 record).
pub const VMTI_LS_UL: [u8; 16] = [
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x03, 0x06, 0x00, 0x00, 0x00,
];

#[derive(Debug, Clone, Default)]
pub struct VmtiLs {
    pub checksum: Option<u16>,
    pub precision_time_stamp: Option<u64>,
    pub vmti_system_name: Option<String>,
    /// Tag 4 `vmtiLsVersionNum` per ST 0903.6 §10.1.4 — V2 (1..=2 wire
    /// bytes, value range 1..=65535) packed BE with leading zeros
    /// stripped. Tracks the spec revision the encoder followed (e.g.
    /// 6 for ST 0903.6).
    pub version_number: Option<u16>,
    pub total_targets_in_frame: Option<u32>,
    pub num_targets_reported: Option<u32>,
    // Tag 7 (motionImageryFrameNumber) is deprecated in ST 0903.6 — no
    // typed field. Wire occurrences land in `unknown` per ST 0107.5 §6.
    pub frame_width: Option<u32>,
    pub frame_height: Option<u32>,
    pub source_sensor: Option<String>,
    pub horizontal_fov: Option<f64>,
    pub vertical_fov: Option<f64>,
    pub miis_id: Option<Vec<u8>>,
    pub targets: Vec<VTargetPack>,
    pub algorithm_series: Option<Vec<u8>>,
    pub ontology_series: Option<Vec<u8>>,
    pub unknown: Vec<OwnedRawField>,
    pub field_errors: Vec<KlvFieldError>,
}

/// Manual `PartialEq` excluding [`VmtiLs::field_errors`]. The
/// `field_errors` vec is a decode-side diagnostic — two LSes that
/// produced identical field values are semantically equal regardless
/// of which fields failed strict-decode validation along the way.
/// Used by round-trip fuzz (decode → encode → decode → assert_eq);
/// `field_errors` is empty on the second decode since encode never
/// emits malformed bytes.
impl PartialEq for VmtiLs {
    fn eq(&self, other: &Self) -> bool {
        self.checksum == other.checksum
            && self.precision_time_stamp == other.precision_time_stamp
            && self.vmti_system_name == other.vmti_system_name
            && self.version_number == other.version_number
            && self.total_targets_in_frame == other.total_targets_in_frame
            && self.num_targets_reported == other.num_targets_reported
            && self.frame_width == other.frame_width
            && self.frame_height == other.frame_height
            && self.source_sensor == other.source_sensor
            && self.horizontal_fov == other.horizontal_fov
            && self.vertical_fov == other.vertical_fov
            && self.miis_id == other.miis_id
            && self.targets == other.targets
            && self.algorithm_series == other.algorithm_series
            && self.ontology_series == other.ontology_series
            && self.unknown == other.unknown
    }
}

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
/// Use [`decode_strict`] (Task 6) for spec-validation use cases that
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
    use tags::{Encoding, lookup};

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
                let v = match decode_imapb(&params, value) {
                    Ok(v) => v,
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
                // tag 101. Strict-mode (Task 6) will route the typed
                // error via `KlvDecodeError::St0903InvalidVTargetPack`.
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
    use tags::{Encoding, TAGS, lookup};

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
                let v = decode_imapb(&params, value).map_err(KlvDecodeError::FieldError)?;
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

/// Symmetric encode of a VMTI Local Set per ST 0903.6 §10.1.
///
/// Fields are emitted in ascending tag order (1, 2, 3, 4, 5, 6, 8, 9,
/// 10, 11, 12, 13, 101, 102, 103); Tag 7 (`motionImageryFrameNumber`)
/// is deprecated in v6 and never emitted. Preserved `unknown` tags are
/// appended last per ST 0107.5 §6 (single-byte tag IDs only — the
/// VMTI LS spec keeps tags ≤107).
///
/// Round-trip property: `decode(encode_to_vec(&ls)?)?` reproduces all
/// typed fields and preserved unknowns of `ls` (modulo IMAPB
/// quantization on `horizontal_fov` / `vertical_fov`). `field_errors`
/// is a decode-time diagnostic and is not emitted on encode.
///
/// # Errors
/// - [`KlvEncodeError::OutOfRange`] if `horizontal_fov` /
///   `vertical_fov` (or any per-target IMAPB float field) falls
///   outside its declared range.
/// - [`KlvEncodeError::RecordTooLarge`] if a TLV's declared length
///   would overflow BER encoding.
pub fn encode(ls: &VmtiLs, out: &mut Vec<u8>) -> Result<(), KlvEncodeError> {
    use crate::klv::length::write_ber;
    use emit::{emit_imapb_n, emit_tlv, emit_var};

    // Ascending tag order. Tag 7 (deprecated) is intentionally skipped
    // — there is no struct field to source it from.
    if let Some(v) = ls.checksum {
        emit_tlv(out, 1, &v.to_be_bytes())?;
    }
    if let Some(v) = ls.precision_time_stamp {
        emit_tlv(out, 2, &v.to_be_bytes())?;
    }
    if let Some(ref s) = ls.vmti_system_name {
        emit_tlv(out, 3, s.as_bytes())?;
    }
    if let Some(v) = ls.version_number {
        emit_var(out, 4, v as u32)?;
    }
    if let Some(v) = ls.total_targets_in_frame {
        emit_var(out, 5, v)?;
    }
    if let Some(v) = ls.num_targets_reported {
        emit_var(out, 6, v)?;
    }
    if let Some(v) = ls.frame_width {
        emit_var(out, 8, v)?;
    }
    if let Some(v) = ls.frame_height {
        emit_var(out, 9, v)?;
    }
    if let Some(ref s) = ls.source_sensor {
        emit_tlv(out, 10, s.as_bytes())?;
    }
    // Top-level FOV tags use IMAPB(0, 180, 2) per §10.1.11 + §10.1.12.
    if let Some(v) = ls.horizontal_fov {
        emit_imapb_n(out, 11, v, 0.0, 180.0, 2)?;
    }
    if let Some(v) = ls.vertical_fov {
        emit_imapb_n(out, 12, v, 0.0, 180.0, 2)?;
    }
    if let Some(ref bytes) = ls.miis_id {
        emit_tlv(out, 13, bytes)?;
    }

    // VTargetSeries (Tag 101). Each pack is BER-length-prefixed inside
    // the series payload (matches `decode_vtarget_series` framing).
    if !ls.targets.is_empty() {
        let mut series = Vec::new();
        for pack in &ls.targets {
            let mut pack_bytes = Vec::new();
            vtarget_pack::write_pack(pack, &mut pack_bytes)?;
            let mut len_buf = [0u8; 9];
            let len_n = write_ber(pack_bytes.len(), &mut len_buf)?;
            series.extend_from_slice(&len_buf[..len_n]);
            series.extend_from_slice(&pack_bytes);
        }
        emit_tlv(out, 101, &series)?;
    }

    if let Some(ref bytes) = ls.algorithm_series {
        emit_tlv(out, 102, bytes)?;
    }
    if let Some(ref bytes) = ls.ontology_series {
        emit_tlv(out, 103, bytes)?;
    }

    // Unknown tags last (preserves them per ST 0107.5 §6). Tag IDs >0xFF
    // are silently dropped — VMTI LS tag IDs are single-byte by spec
    // (highest is 103) so a >0xFF tag here would be a corrupted parse.
    for field in &ls.unknown {
        if field.tag <= 0xFF {
            emit_tlv(out, field.tag as u8, &field.value)?;
        }
    }

    Ok(())
}

/// Encode a VMTI Local Set into a fresh `Vec<u8>`. Convenience over
/// [`encode`] when the caller has no pre-sized buffer.
///
/// # Errors
/// Returns the same [`KlvEncodeError`] variants as [`encode`].
pub fn encode_to_vec(ls: &VmtiLs) -> Result<Vec<u8>, KlvEncodeError> {
    let mut out = Vec::new();
    encode(ls, &mut out)?;
    Ok(out)
}

/// Number of wire bytes that [`encode`] would produce for `ls`. Mirrors
/// `encode`'s field-by-field structure so the two cannot drift.
pub fn encoded_len(ls: &VmtiLs) -> usize {
    use crate::klv::length::ber_len;
    use var_uint::var_u32_len;

    fn tlv_len(value_len: usize) -> usize {
        1 /* tag */ + ber_len(value_len) + value_len
    }

    let mut total = 0usize;
    if ls.checksum.is_some() {
        total += tlv_len(2);
    }
    if ls.precision_time_stamp.is_some() {
        total += tlv_len(8);
    }
    if let Some(ref s) = ls.vmti_system_name {
        total += tlv_len(s.len());
    }
    if let Some(v) = ls.version_number {
        total += tlv_len(var_u32_len(v as u32));
    }
    if let Some(v) = ls.total_targets_in_frame {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(v) = ls.num_targets_reported {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(v) = ls.frame_width {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(v) = ls.frame_height {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(ref s) = ls.source_sensor {
        total += tlv_len(s.len());
    }
    if ls.horizontal_fov.is_some() {
        total += tlv_len(2);
    }
    if ls.vertical_fov.is_some() {
        total += tlv_len(2);
    }
    if let Some(ref bytes) = ls.miis_id {
        total += tlv_len(bytes.len());
    }
    if !ls.targets.is_empty() {
        let mut series_len = 0usize;
        for pack in &ls.targets {
            let pack_len = vtarget_pack::encoded_len(pack);
            series_len += ber_len(pack_len) + pack_len;
        }
        total += tlv_len(series_len);
    }
    if let Some(ref bytes) = ls.algorithm_series {
        total += tlv_len(bytes.len());
    }
    if let Some(ref bytes) = ls.ontology_series {
        total += tlv_len(bytes.len());
    }
    for field in &ls.unknown {
        if field.tag <= 0xFF {
            total += tlv_len(field.value.len());
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal LS containing only Tag 1 (Checksum) + Tag 2 (PTS)
    /// + Tag 4 (Version) — the three commonly-required tags. (Strict
    ///   mode validates a tighter set per Task 6 — see §10.1 carriage
    ///   rules.)
    fn minimal_ls_bytes() -> Vec<u8> {
        // tag 1 (Checksum, U16Be) = 0
        // tag 2 (PTS, U64Be) = 1_700_000_000_000_000
        // tag 4 (Version, V2) = 6 (1-byte truncated big-endian)
        let mut out = Vec::new();
        out.extend_from_slice(&[1, 2, 0, 0]);
        out.extend_from_slice(&[2, 8]);
        out.extend_from_slice(&1_700_000_000_000_000u64.to_be_bytes());
        out.extend_from_slice(&[4, 1, 6]);
        out
    }

    #[test]
    fn decode_minimal_ls() {
        let bytes = minimal_ls_bytes();
        let ls = decode(&bytes).unwrap();
        assert_eq!(ls.checksum, Some(0));
        assert_eq!(ls.precision_time_stamp, Some(1_700_000_000_000_000));
        assert_eq!(ls.version_number, Some(6));
        assert!(ls.targets.is_empty());
        assert!(ls.unknown.is_empty());
        assert!(ls.field_errors.is_empty());
    }

    #[test]
    fn decode_unknown_tag_preserved() {
        let mut bytes = minimal_ls_bytes();
        // Append unknown tag 100 with 3-byte value. (Tag 100 is in the
        // gap between defined tag 13 and tag 101; safe choice for
        // "unknown.")
        bytes.extend_from_slice(&[100, 3, 0xAA, 0xBB, 0xCC]);
        let ls = decode(&bytes).unwrap();
        assert_eq!(ls.unknown.len(), 1);
        assert_eq!(ls.unknown[0].tag, 100);
        assert_eq!(ls.unknown[0].value, vec![0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn decode_truncated_value_lenient_field_error() {
        // tag 4 (Version, V2) declares BER length 5 but only 1 byte
        // of value is present. Lenient decode does not panic;
        // field_errors capture the truncation. Strict mode rejects
        // (Task 6).
        let bytes = [4u8, 5, 0x01];
        let ls = decode(&bytes).unwrap();
        assert!(!ls.field_errors.is_empty());
    }

    /// Regression for Phase 0 Task 1.5: hostile bytes targeting the
    /// U64Be (Tag 2 PTS) decode path must never panic. The upstream
    /// `value.len() != 8` length check intercepts wrong-sized slices
    /// before the `try_into` runs, so the fallible-conversion safety
    /// net added in Task 1.5 is defense-in-depth — both the well-formed
    /// and malformed cases below exercise the contract that decode
    /// returns a value (lenient: with field_errors / strict: Err)
    /// instead of panicking.
    #[test]
    fn decode_tag2_pts_wrong_length_no_panic() {
        // 7-byte PTS instead of 8 — caught by the length check, surfaced
        // as InvalidLength on lenient.
        let bytes = vec![2u8, 7, 0, 0, 0, 0, 0, 0, 1];
        let ls = decode(&bytes).unwrap();
        assert!(ls.precision_time_stamp.is_none());
        assert!(matches!(
            ls.field_errors.as_slice(),
            [
                KlvFieldError::InvalidLength {
                    tag: 2,
                    expected: 8,
                    got: 7,
                },
                ..
            ] | [KlvFieldError::TruncatedField { tag: 2 }, ..]
        ));
    }

    #[test]
    fn strict_decode_tag2_pts_wrong_length_rejected() {
        // 7-byte PTS — strict mode must Err, never panic.
        let bytes = vec![2u8, 7, 0, 0, 0, 0, 0, 0, 1];
        let err = decode_strict(&bytes).unwrap_err();
        assert!(matches!(
            err,
            KlvDecodeError::FieldError(KlvFieldError::InvalidLength {
                tag: 2,
                expected: 8,
                got: 7,
            }) | KlvDecodeError::FieldError(KlvFieldError::TruncatedField { tag: 2 })
        ));
    }

    #[test]
    fn decode_tag2_pts_well_formed_still_works() {
        // Sanity check that the Task 1.5 fix didn't regress the happy
        // path. 8-byte PTS decodes to the expected u64.
        let mut bytes = vec![2u8, 8];
        bytes.extend_from_slice(&1_700_000_000_000_000u64.to_be_bytes());
        let ls = decode(&bytes).unwrap();
        assert_eq!(ls.precision_time_stamp, Some(1_700_000_000_000_000));
        assert!(ls.field_errors.is_empty());
    }

    #[test]
    fn decode_with_one_target() {
        // Build a VTargetSeries (Tag 101) containing one pack.
        // Pack body: target_id=7, centroid_pixel=12345 (Tag 1 VarUint).
        // var_u32_len(12345) = 2 (since 12345 = 0x3039 fits in 2 bytes).
        // Tag 1 TLV inside pack: [0x01, 0x02, 0x30, 0x39].
        let mut pack_body = Vec::new();
        pack_body.push(7); // target_id BER-OID 1-byte
        pack_body.extend_from_slice(&[0x01, 0x02, 0x30, 0x39]);

        let mut series = Vec::new();
        // Each pack is BER-length-prefixed inside the series.
        series.push(pack_body.len() as u8);
        series.extend_from_slice(&pack_body);

        let mut bytes = minimal_ls_bytes();
        bytes.push(101); // tag 101 VTargetSeries
        bytes.push(series.len() as u8);
        bytes.extend_from_slice(&series);

        let ls = decode(&bytes).unwrap();
        assert_eq!(ls.targets.len(), 1);
        assert_eq!(ls.targets[0].target_id, 7);
        assert_eq!(ls.targets[0].centroid_pixel, Some(12345));
    }

    #[test]
    fn decode_version_v2_two_byte() {
        // Version Number as 2-byte VarUint: [0x04, 0x02, 0x01, 0x00] for value 256.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[1, 2, 0, 0]); // checksum
        bytes.extend_from_slice(&[2, 8]); // PTS
        bytes.extend_from_slice(&1_700_000_000_000_000u64.to_be_bytes());
        bytes.extend_from_slice(&[4, 2, 0x01, 0x00]); // version = 256
        let ls = decode(&bytes).unwrap();
        assert_eq!(ls.version_number, Some(256));
    }

    #[test]
    fn decode_with_two_targets() {
        // Two targets in series — verifies the series walker handles
        // multiple BER-prefixed packs in sequence.
        let mut series = Vec::new();
        // Target 1: target_id=1, centroid_pixel=100 (1-byte VarUint).
        let pack1: Vec<u8> = vec![1, 0x01, 0x01, 100];
        series.push(pack1.len() as u8);
        series.extend_from_slice(&pack1);
        // Target 2: target_id=2, priority=5.
        let pack2: Vec<u8> = vec![2, 4, 1, 5];
        series.push(pack2.len() as u8);
        series.extend_from_slice(&pack2);

        let mut bytes = minimal_ls_bytes();
        bytes.push(101);
        bytes.push(series.len() as u8);
        bytes.extend_from_slice(&series);

        let ls = decode(&bytes).unwrap();
        assert_eq!(ls.targets.len(), 2);
        assert_eq!(ls.targets[0].target_id, 1);
        assert_eq!(ls.targets[0].centroid_pixel, Some(100));
        assert_eq!(ls.targets[1].target_id, 2);
        assert_eq!(ls.targets[1].priority, Some(5));
    }

    #[test]
    fn decode_pass_through_tags() {
        // Tags 102 (Algorithm Series), 103 (Ontology Series), 13 (MIIS ID)
        // are pass-through bytes per the design.
        let mut bytes = minimal_ls_bytes();
        bytes.extend_from_slice(&[102, 2, 0xDE, 0xAD]);
        bytes.extend_from_slice(&[103, 2, 0xBE, 0xEF]);
        bytes.extend_from_slice(&[13, 3, 0xCA, 0xFE, 0x00]);
        let ls = decode(&bytes).unwrap();
        assert_eq!(ls.algorithm_series.as_deref(), Some(&[0xDEu8, 0xAD][..]));
        assert_eq!(ls.ontology_series.as_deref(), Some(&[0xBEu8, 0xEF][..]));
        assert_eq!(ls.miis_id.as_deref(), Some(&[0xCAu8, 0xFE, 0x00][..]));
    }

    #[test]
    fn decode_zero_length_imapb_does_not_panic() {
        // Regression: Tag 11 with BER length 0 must surface
        // InvalidLength in field_errors, not panic. Without the
        // hardcoded `length=2` guard, `decode_imapb` calls
        // `read_signed_be(&[])` which underflows `n*8-1` at n==0.
        let mut bytes = minimal_ls_bytes();
        bytes.extend_from_slice(&[11, 0]); // tag 11, length 0
        let ls = decode(&bytes).unwrap();
        assert!(ls.horizontal_fov.is_none());
        assert!(ls.field_errors.iter().any(|e| matches!(
            e,
            KlvFieldError::InvalidLength {
                tag: 11,
                expected: 2,
                got: 0
            }
        )));
    }

    #[test]
    fn decode_imapb_happy_path() {
        // FOV = 90.0° encoded as IMAPB(0, 180, 2) per ST 0903.6
        // §10.1.11 worked example. The spec-correct encoding for
        // 90.0° in this range is the byte pair 0x2D 0x00.
        //
        // Historical note: the pre-fix substrate used a signed-
        // midpoint formula that produced 0xAD 0x00 for the same
        // input (and this test was transcribed from that wrong
        // output). Tasks 1–5 of plan
        // 2026-05-10-klv-wire-format-critical-fixes corrected the
        // substrate; this test now codifies the spec result.
        let mut bytes = minimal_ls_bytes();
        bytes.extend_from_slice(&[11, 2, 0x2D, 0x00]);
        let ls = decode(&bytes).unwrap();
        let fov = ls.horizontal_fov.expect("horizontal_fov decoded");
        assert!((fov - 90.0).abs() < 0.01, "got fov={fov}, expected ~90.0");
        assert!(ls.field_errors.is_empty());
    }

    // ------------------------------------------------------------------
    // Task 6 — `decode_strict` tests.
    // ------------------------------------------------------------------

    /// Build the minimum LS that satisfies `decode_strict`'s required-tag
    /// gate per Task 2's audit: Tag 4 (Version) + Tag 6 (numTargetsReported).
    /// Tags 1/2/11/12/13 are conditional and NOT enforced by `decode_strict`
    /// (consumers needing carriage-aware validation post-validate).
    fn minimal_strict_ls_bytes() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&[4, 1, 6]); // version = 6
        out.extend_from_slice(&[6, 1, 0]); // num_targets_reported = 0
        out
    }

    #[test]
    fn strict_decode_minimal_passes() {
        let bytes = minimal_strict_ls_bytes();
        let ls = decode_strict(&bytes).unwrap();
        assert_eq!(ls.version_number, Some(6));
        assert_eq!(ls.num_targets_reported, Some(0));
    }

    #[test]
    fn strict_decode_with_optional_tags_passes() {
        // All required + several optional tags. Should pass strict.
        let mut bytes = minimal_strict_ls_bytes();
        bytes.extend_from_slice(&[1, 2, 0, 0]); // checksum
        bytes.extend_from_slice(&[2, 8]);
        bytes.extend_from_slice(&1_700_000_000_000_000u64.to_be_bytes());
        bytes.extend_from_slice(&[8, 2, 0x07, 0x80]); // frame_width = 1920
        bytes.extend_from_slice(&[9, 2, 0x04, 0x38]); // frame_height = 1080
        let ls = decode_strict(&bytes).unwrap();
        assert_eq!(ls.checksum, Some(0));
        assert_eq!(ls.precision_time_stamp, Some(1_700_000_000_000_000));
        assert_eq!(ls.frame_width, Some(1920));
        assert_eq!(ls.frame_height, Some(1080));
    }

    #[test]
    fn strict_decode_missing_required_version_rejected() {
        // Tag 4 (Version) omitted, Tag 6 present. Strict should reject.
        let bytes = vec![6, 1, 0];
        let err = decode_strict(&bytes).unwrap_err();
        assert!(matches!(
            err,
            KlvDecodeError::St0903MissingRequiredTag { tag: 4 }
        ));
    }

    #[test]
    fn strict_decode_missing_required_num_targets_rejected() {
        // Tag 4 present, Tag 6 (numTargetsReported) omitted.
        let bytes = vec![4, 1, 6];
        let err = decode_strict(&bytes).unwrap_err();
        assert!(matches!(
            err,
            KlvDecodeError::St0903MissingRequiredTag { tag: 6 }
        ));
    }

    #[test]
    fn strict_decode_duplicate_tag_rejected() {
        let mut bytes = minimal_strict_ls_bytes();
        // Append a second Tag 4 (Version).
        bytes.extend_from_slice(&[4, 1, 7]);
        let err = decode_strict(&bytes).unwrap_err();
        assert!(matches!(err, KlvDecodeError::DuplicateTag { tag: 4, .. }));
    }

    #[test]
    fn strict_decode_invalid_utf8_rejected() {
        let mut bytes = minimal_strict_ls_bytes();
        // Tag 3 (System Name) with bytes [0xFF, 0xFE] (invalid UTF-8).
        bytes.extend_from_slice(&[3, 2, 0xFF, 0xFE]);
        let err = decode_strict(&bytes).unwrap_err();
        assert!(matches!(
            err,
            KlvDecodeError::FieldError(KlvFieldError::InvalidUtf8 { tag: 3 })
        ));
    }

    #[test]
    fn strict_decode_unknown_tag_preserved() {
        let mut bytes = minimal_strict_ls_bytes();
        bytes.extend_from_slice(&[100, 3, 0xAA, 0xBB, 0xCC]);
        // ST 0107.5 §6 skip rule — unknown tags must round-trip through
        // strict mode too.
        let ls = decode_strict(&bytes).unwrap();
        assert_eq!(ls.unknown.len(), 1);
        assert_eq!(ls.unknown[0].tag, 100);
    }

    #[test]
    fn strict_decode_zero_length_imapb_rejected() {
        // Strict mode must surface IMAPB-length errors as Err, not
        // accept them silently. Lenient surfaces in field_errors.
        let mut bytes = minimal_strict_ls_bytes();
        bytes.extend_from_slice(&[11, 0]); // tag 11, BER length 0
        let err = decode_strict(&bytes).unwrap_err();
        assert!(matches!(
            err,
            KlvDecodeError::FieldError(KlvFieldError::InvalidLength {
                tag: 11,
                expected: 2,
                got: 0
            })
        ));
    }

    #[test]
    fn strict_decode_truncated_input_rejected() {
        // Required Tag 4 with declared length 5 but only 1 byte present.
        let bytes = vec![4u8, 5, 0x01];
        let err = decode_strict(&bytes).unwrap_err();
        assert!(matches!(err, KlvDecodeError::Truncated { .. }));
    }

    #[test]
    fn strict_decode_invalid_vtargetpack_rejected() {
        // Tag 101 with malformed pack body. Should route via
        // St0903InvalidVTargetPack typed variant. Pack body = [0x81]:
        // BER-OID continuation byte without a terminator → truncated
        // target_id.
        let mut bytes = minimal_strict_ls_bytes();
        bytes.extend_from_slice(&[101, 2, 1, 0x81]);
        let err = decode_strict(&bytes).unwrap_err();
        assert!(matches!(
            err,
            KlvDecodeError::St0903InvalidVTargetPack { .. }
        ));
    }

    // ------------------------------------------------------------------
    // Task 7 — `encode` + `encoded_len` round-trip + canonical-bytes.
    // ------------------------------------------------------------------

    #[test]
    fn encode_round_trips_minimal() {
        let ls = VmtiLs {
            checksum: Some(0),
            precision_time_stamp: Some(1_700_000_000_000_000),
            version_number: Some(6),
            num_targets_reported: Some(0),
            ..Default::default()
        };

        let bytes = encode_to_vec(&ls).unwrap();
        assert_eq!(bytes.len(), encoded_len(&ls));

        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.checksum, Some(0));
        assert_eq!(decoded.precision_time_stamp, Some(1_700_000_000_000_000));
        assert_eq!(decoded.version_number, Some(6));
        assert_eq!(decoded.num_targets_reported, Some(0));
    }

    #[test]
    fn encode_round_trips_with_targets() {
        let ls = VmtiLs {
            checksum: Some(0xABCD),
            precision_time_stamp: Some(1_700_000_000_000_000),
            version_number: Some(6),
            num_targets_reported: Some(2),
            frame_width: Some(3840),
            frame_height: Some(2160),
            horizontal_fov: Some(45.0),
            vertical_fov: Some(30.0),
            source_sensor: Some("EO/IR Camera 1".to_string()),
            targets: vec![
                VTargetPack {
                    target_id: 1,
                    centroid_pixel: Some(8_294_400),
                    priority: Some(0),
                    confidence_level: Some(95),
                    target_color: Some([0xFF, 0x00, 0x00]),
                    ..Default::default()
                },
                VTargetPack {
                    target_id: 2,
                    centroid_pixel: Some(4_147_200),
                    priority: Some(1),
                    confidence_level: Some(80),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let bytes = encode_to_vec(&ls).unwrap();
        let decoded = decode(&bytes).unwrap();

        assert_eq!(decoded.frame_width, Some(3840));
        assert_eq!(decoded.frame_height, Some(2160));
        assert_eq!(decoded.source_sensor.as_deref(), Some("EO/IR Camera 1"));
        // FOV uses IMAPB(0, 180, 2) — precision is (180-0)/(2^16-1) ≈ 0.00275°
        assert!((decoded.horizontal_fov.unwrap() - 45.0).abs() < 0.01);
        assert!((decoded.vertical_fov.unwrap() - 30.0).abs() < 0.01);
        assert_eq!(decoded.targets.len(), 2);
        assert_eq!(decoded.targets[0].target_id, 1);
        assert_eq!(decoded.targets[0].confidence_level, Some(95));
        assert_eq!(decoded.targets[1].target_id, 2);
    }

    #[test]
    fn encode_preserves_unknown_tags() {
        let ls = VmtiLs {
            checksum: Some(0),
            precision_time_stamp: Some(1_700_000_000_000_000),
            version_number: Some(6),
            num_targets_reported: Some(0),
            unknown: vec![OwnedRawField {
                tag: 100,
                value: vec![0xDE, 0xAD, 0xBE, 0xEF],
            }],
            ..Default::default()
        };

        let bytes = encode_to_vec(&ls).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.unknown.len(), 1);
        assert_eq!(decoded.unknown[0].tag, 100);
        assert_eq!(decoded.unknown[0].value, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn encoded_len_matches_encode() {
        let ls = VmtiLs {
            checksum: Some(0xCAFE),
            precision_time_stamp: Some(1_700_000_000_000_000),
            version_number: Some(6),
            num_targets_reported: Some(1),
            frame_width: Some(1920),
            frame_height: Some(1080),
            targets: vec![VTargetPack {
                target_id: 1,
                centroid_pixel: Some(123),
                ..Default::default()
            }],
            ..Default::default()
        };

        let bytes = encode_to_vec(&ls).unwrap();
        assert_eq!(bytes.len(), encoded_len(&ls));
    }

    #[test]
    fn encode_canonical_byte_layout() {
        // Locks in the wire format for a known LS shape. Catches
        // accidental field-order changes in `encode` (which round-trip
        // tests miss because `decode` is order-agnostic) and catches
        // `encoded_len` drift relative to `encode`.
        let ls = VmtiLs {
            version_number: Some(6),       // Tag 4, V2 → 1 byte [0x06]
            num_targets_reported: Some(0), // Tag 6, V3 → 1 byte [0x00]
            frame_width: Some(1920),       // Tag 8, V3 → 2 bytes [0x07, 0x80]
            ..Default::default()
        };

        let bytes = encode_to_vec(&ls).unwrap();

        // Expected wire form (ascending tag order):
        let expected: Vec<u8> = vec![
            // Tag 4, len 1, value [0x06]
            0x04, 0x01, 0x06, // Tag 6, len 1, value [0x00]
            0x06, 0x01, 0x00, // Tag 8, len 2, value [0x07, 0x80]
            0x08, 0x02, 0x07, 0x80,
        ];
        assert_eq!(
            bytes, expected,
            "encode produced unexpected byte layout — \
             field order changed or TLV bytes are wrong"
        );

        assert_eq!(
            bytes.len(),
            encoded_len(&ls),
            "encoded_len disagrees with encode_to_vec output length"
        );
    }
}
