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

#[derive(Debug, Clone, PartialEq, Default)]
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
    pub frame_number: Option<u32>,
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
/// [`vtarget_pack::read_pack`]; pack-level errors land in `field_errors`
/// as [`KlvDecodeError::St0903InvalidVTargetPack`] reasons too — but
/// surfaced via the parent-level [`KlvFieldError`] channel (see
/// [`decode_vtarget_series`]).
///
/// Use [`decode_strict`] (Task 6) for spec-validation use cases that
/// reject any of the above.
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
                let v = u64::from_be_bytes(value.try_into().unwrap());
                debug_assert_eq!(tag, 2, "U64Be reserved for tag 2 (Precision Time Stamp)");
                ls.precision_time_stamp = Some(v);
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
                let params = ImapbParams {
                    min,
                    max,
                    length: value.len(),
                };
                let v = match decode_imapb(&params, value) {
                    Ok(v) => v,
                    Err(_) => {
                        ls.field_errors.push(KlvFieldError::OutOfRange {
                            tag: tag as u32,
                            value: 0.0,
                            min,
                            max,
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

#[allow(unused_variables)] // Task 6 wires the body
pub fn decode_strict(bytes: &[u8]) -> Result<VmtiLs, KlvDecodeError> {
    todo!("Task 6")
}

#[allow(unused_variables, clippy::ptr_arg)] // Task 7 wires the body
pub fn encode(ls: &VmtiLs, out: &mut Vec<u8>) -> Result<(), KlvEncodeError> {
    todo!("Task 7")
}

pub fn encode_to_vec(ls: &VmtiLs) -> Result<Vec<u8>, KlvEncodeError> {
    let mut out = Vec::new();
    encode(ls, &mut out)?;
    Ok(out)
}

#[allow(unused_variables)] // Task 7 wires the body
pub fn encoded_len(ls: &VmtiLs) -> Result<usize, KlvEncodeError> {
    todo!("Task 7")
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
}
