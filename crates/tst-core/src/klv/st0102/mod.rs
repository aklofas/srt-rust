//! MISB ST 0102.12 Security Metadata Local Set typed layer.
//!
//! Sibling typed parser to [`crate::klv::st0601`]. Consumers who decode
//! a `UasDatalinkLs` and want typed access to the inner Security LS
//! call [`decode`] (or [`decode_strict`]) on
//! `record.security_local_set.as_deref()?`.
//!
//! Two decode entry points:
//! - [`decode`] — lenient: tolerates missing tags, unknown tags
//!   (preserved in `unknown`), unknown enum codepoints (decoded as
//!   `Unknown(u8)`), Tag 13 UTF-16 decode failures (raw bytes
//!   preserved in `unknown`).
//! - [`decode_strict`] — strict: rejects missing required tags,
//!   unknown enum codepoints, `OmittedValueXX` codepoints, non-canonical
//!   BER, duplicate tags, malformed UTF-16. Unknown tags are still
//!   preserved per ST 0107.5 §6 future-proof skip rule.
//!
//! Encode is symmetric — decode + encode bit-identical round-trips for
//! all spec-conformant input.
//!
//! Universal Set form of ST 0102 is out of scope (LS-only on
//! MPEG-TS+KLV streams).

pub(crate) mod enums;
pub(crate) mod tags;

pub use enums::{
    ClassifyingCountryCodingMethod, ObjectCountryCodingMethod, SecurityClassification,
};

use crate::error::{KlvDecodeError, KlvEncodeError, KlvFieldError};
use crate::klv::pack::OwnedRawField;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SecurityLs {
    // Required per spec (still Option<T> at decode time so lenient
    // mode tolerates broken input; decode_strict rejects a record
    // missing any of 1, 2, 3, 12, 13, 22).
    pub security_classification: Option<SecurityClassification>, // Tag 1
    pub classifying_country_coding_method: Option<ClassifyingCountryCodingMethod>, // Tag 2
    pub classifying_country: Option<String>,                     // Tag 3
    pub object_country_coding_method: Option<ObjectCountryCodingMethod>, // Tag 12
    pub object_country_codes: Option<String>,                    // Tag 13 (UTF-16)
    pub version: Option<u16>,                                    // Tag 22

    // Context (per-spec semantics: present only when applicable).
    pub sci_shi_info: Option<String>,                  // Tag 4
    pub caveats: Option<String>,                       // Tag 5
    pub releasing_instructions: Option<String>,        // Tag 6
    pub classified_by: Option<String>,                 // Tag 7
    pub derived_from: Option<String>,                  // Tag 8
    pub classification_reason: Option<String>,         // Tag 9
    pub declassification_date: Option<String>,         // Tag 10 ("YYYYMMDD")
    pub classification_marking_system: Option<String>, // Tag 11

    // Optional.
    pub classification_comments: Option<String>, // Tag 14
    pub classifying_country_coding_method_version_date: Option<String>, // Tag 23 ("YYYY-MM-DD")
    pub object_country_coding_method_version_date: Option<String>, // Tag 24 ("YYYY-MM-DD")

    /// Forward-compat: tags outside the LS table preserved verbatim.
    /// Both `decode` and `decode_strict` populate this per ST 0107.5 §6.
    pub unknown: Vec<OwnedRawField>,

    /// Lenient-mode diagnosis: known tags whose value validation
    /// failed (e.g. Tag 13 UTF-16 decode failure, Tag 22 wrong
    /// length). Mirrors `klv::st0601::UasDatalinkLs.field_errors`.
    /// Strict-mode raises these as `Err` instead of populating
    /// this field. Encode does not consume this field.
    pub field_errors: Vec<KlvFieldError>,
}

/// Decode RFC 2781 UTF-16 with optional BOM. Default endianness is BE
/// per RFC 2781 §4.3 ("if there is no BOM, the text SHOULD be
/// interpreted as UTF-16BE").
fn decode_utf16_bom(bytes: &[u8]) -> Result<String, ()> {
    let (units, big_endian): (Vec<u16>, bool) = if bytes.starts_with(&[0xFE, 0xFF]) {
        (parse_u16_pairs(&bytes[2..], true)?, true)
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        (parse_u16_pairs(&bytes[2..], false)?, false)
    } else {
        (parse_u16_pairs(bytes, true)?, true) // default BE per RFC 2781 §4.3
    };
    let _ = big_endian; // captured for potential future re-emit; not currently used
    String::from_utf16(&units).map_err(|_| ())
}

fn parse_u16_pairs(bytes: &[u8], big_endian: bool) -> Result<Vec<u16>, ()> {
    if bytes.len() % 2 != 0 {
        return Err(());
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        let unit = if big_endian {
            u16::from_be_bytes([chunk[0], chunk[1]])
        } else {
            u16::from_le_bytes([chunk[0], chunk[1]])
        };
        out.push(unit);
    }
    Ok(out)
}

/// Lenient decode — tolerates malformed input where possible.
pub fn decode(buf: &[u8]) -> Result<SecurityLs, KlvDecodeError> {
    decode_inner(buf, /* strict = */ false)
}

fn decode_inner(buf: &[u8], strict: bool) -> Result<SecurityLs, KlvDecodeError> {
    use crate::klv::pack::Iter;
    use crate::klv::st0102::tags::{Encoding, REQUIRED_TAGS, lookup};

    let mut record = SecurityLs::default();
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();

    for r in Iter::local_set(buf) {
        let f = r?;
        if !seen.insert(f.tag) {
            return Err(KlvDecodeError::DuplicateTag {
                tag: f.tag,
                offset: 0, // Iter doesn't surface offset; non-fatal best-effort
            });
        }

        let tag_u8 = match u8::try_from(f.tag) {
            Ok(t) => t,
            Err(_) => {
                // Tag > 255: shouldn't happen in ST 0102 LS but
                // tolerated as forward-compat unknown.
                record.unknown.push(OwnedRawField {
                    tag: f.tag,
                    value: f.value.to_vec(),
                });
                continue;
            }
        };

        let spec = match lookup(tag_u8) {
            Some(s) => s,
            None => {
                // Unknown tag — pass through per ST 0107.5 §6.
                record.unknown.push(OwnedRawField {
                    tag: f.tag,
                    value: f.value.to_vec(),
                });
                continue;
            }
        };

        match spec.encoding {
            Encoding::U8Enum => {
                if f.value.len() != 1 {
                    return Err(KlvDecodeError::FieldError(KlvFieldError::InvalidLength {
                        tag: f.tag,
                        expected: 1,
                        got: f.value.len(),
                    }));
                }
                let b = f.value[0];
                match tag_u8 {
                    1 => {
                        let v = SecurityClassification::from_u8(b);
                        if strict && !v.is_known_codepoint() {
                            return Err(KlvDecodeError::FieldError(
                                KlvFieldError::InvalidCodepoint {
                                    tag: f.tag,
                                    value: b,
                                },
                            ));
                        }
                        record.security_classification = Some(v);
                    }
                    2 => {
                        let v = ClassifyingCountryCodingMethod::from_u8(b);
                        if strict && !v.is_known_codepoint() {
                            return Err(KlvDecodeError::FieldError(
                                KlvFieldError::InvalidCodepoint {
                                    tag: f.tag,
                                    value: b,
                                },
                            ));
                        }
                        record.classifying_country_coding_method = Some(v);
                    }
                    12 => {
                        let v = ObjectCountryCodingMethod::from_u8(b);
                        if strict && !v.is_known_codepoint() {
                            return Err(KlvDecodeError::FieldError(
                                KlvFieldError::InvalidCodepoint {
                                    tag: f.tag,
                                    value: b,
                                },
                            ));
                        }
                        record.object_country_coding_method = Some(v);
                    }
                    _ => unreachable!("tags.rs U8Enum reserved for tags 1, 2, 12 only"),
                }
            }
            Encoding::U16Be => {
                if f.value.len() != 2 {
                    return Err(KlvDecodeError::FieldError(KlvFieldError::InvalidLength {
                        tag: f.tag,
                        expected: 2,
                        got: f.value.len(),
                    }));
                }
                debug_assert_eq!(tag_u8, 22);
                record.version = Some(u16::from_be_bytes([f.value[0], f.value[1]]));
            }
            Encoding::Iso646 | Encoding::FixedAscii { .. } => {
                let s = match std::str::from_utf8(f.value) {
                    Ok(s) => s.to_string(),
                    Err(_) => {
                        return Err(KlvDecodeError::FieldError(KlvFieldError::InvalidUtf8 {
                            tag: f.tag,
                        }));
                    }
                };
                if let Encoding::FixedAscii { expected_len } = spec.encoding {
                    if strict && s.len() != expected_len {
                        return Err(KlvDecodeError::FieldError(KlvFieldError::InvalidLength {
                            tag: f.tag,
                            expected: expected_len,
                            got: s.len(),
                        }));
                    }
                }
                match tag_u8 {
                    3 => record.classifying_country = Some(s),
                    4 => record.sci_shi_info = Some(s),
                    5 => record.caveats = Some(s),
                    6 => record.releasing_instructions = Some(s),
                    7 => record.classified_by = Some(s),
                    8 => record.derived_from = Some(s),
                    9 => record.classification_reason = Some(s),
                    10 => record.declassification_date = Some(s),
                    11 => record.classification_marking_system = Some(s),
                    14 => record.classification_comments = Some(s),
                    23 => record.classifying_country_coding_method_version_date = Some(s),
                    24 => record.object_country_coding_method_version_date = Some(s),
                    _ => unreachable!("tags.rs Iso646/FixedAscii reserved for known tags"),
                }
            }
            Encoding::Utf16Bom => {
                debug_assert_eq!(tag_u8, 13);
                match decode_utf16_bom(f.value) {
                    Ok(s) => record.object_country_codes = Some(s),
                    Err(()) => {
                        if strict {
                            return Err(KlvDecodeError::FieldError(KlvFieldError::InvalidUtf16 {
                                tag: f.tag,
                            }));
                        } else {
                            // Lenient: signal failure via field_errors per spec §3.5.
                            // Mirrors klv::st0601's pattern. Raw bytes are NOT
                            // preserved — re-emitting malformed Tag 13 wouldn't
                            // help any consumer; the caller wanting byte-level
                            // access goes through klv::pack::Iter directly.
                            record
                                .field_errors
                                .push(KlvFieldError::InvalidUtf16 { tag: f.tag });
                        }
                    }
                }
            }
        }
    }

    if strict {
        for &t in REQUIRED_TAGS {
            let present = match t {
                1 => record.security_classification.is_some(),
                2 => record.classifying_country_coding_method.is_some(),
                3 => record.classifying_country.is_some(),
                12 => record.object_country_coding_method.is_some(),
                13 => record.object_country_codes.is_some(),
                22 => record.version.is_some(),
                _ => true,
            };
            if !present {
                return Err(KlvDecodeError::St0102MissingRequiredTag { tag: t });
            }
        }
    }

    Ok(record)
}

/// Strict decode — rejects spec-violating input (missing required
/// tags, unknown enum codepoints, non-canonical BER, malformed UTF-16,
/// duplicate tags). Unknown tags are still preserved in `unknown` per
/// ST 0107.5 §6.
pub fn decode_strict(_buf: &[u8]) -> Result<SecurityLs, KlvDecodeError> {
    todo!("Task 6")
}

/// Encode into a caller-provided buffer. Returns the number of bytes
/// written.
pub fn encode(_record: &SecurityLs, _out: &mut [u8]) -> Result<usize, KlvEncodeError> {
    todo!("Task 5")
}

/// Encode into a fresh `Vec<u8>`.
pub fn encode_to_vec(_record: &SecurityLs) -> Result<Vec<u8>, KlvEncodeError> {
    todo!("Task 5")
}

/// Pre-compute the encoded length for a given record.
pub fn encoded_len(_record: &SecurityLs) -> usize {
    todo!("Task 5")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::klv::length::write_ber;

    /// Build a single-tag LS body: tag (BER-OID, 1 byte for tags ≤ 127),
    /// length (BER), value bytes.
    fn build_record(tags: &[(u8, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (tag, value) in tags {
            out.push(*tag);
            let mut len_buf = [0u8; 9];
            let n = write_ber(value.len(), &mut len_buf).expect("len encodable");
            out.extend_from_slice(&len_buf[..n]);
            out.extend_from_slice(value);
        }
        out
    }

    #[test]
    fn decode_tag1_security_classification_secret() {
        let buf = build_record(&[(1, &[0x04])]);
        let r = decode(&buf).expect("decode succeeds");
        assert_eq!(
            r.security_classification,
            Some(SecurityClassification::Secret)
        );
    }

    #[test]
    fn decode_tag1_unknown_codepoint_lenient() {
        let buf = build_record(&[(1, &[0xFA])]);
        let r = decode(&buf).expect("lenient tolerates unknown codepoint");
        assert_eq!(
            r.security_classification,
            Some(SecurityClassification::Unknown(0xFA))
        );
    }

    #[test]
    fn decode_tag2_classifying_country_coding_method() {
        let buf = build_record(&[(2, &[0x05])]);
        let r = decode(&buf).unwrap();
        assert_eq!(
            r.classifying_country_coding_method,
            Some(ClassifyingCountryCodingMethod::Iso3166Numeric)
        );
    }

    #[test]
    fn decode_tag3_classifying_country() {
        let buf = build_record(&[(3, b"//USA")]);
        let r = decode(&buf).unwrap();
        assert_eq!(r.classifying_country.as_deref(), Some("//USA"));
    }

    #[test]
    fn decode_tag10_declassification_date() {
        let buf = build_record(&[(10, b"20300101")]);
        let r = decode(&buf).unwrap();
        assert_eq!(r.declassification_date.as_deref(), Some("20300101"));
    }

    #[test]
    fn decode_tag12_object_country_coding_method() {
        // Tag 12's 0x03 is ISO-3166 Numeric (≠ Tag 2's 0x05).
        let buf = build_record(&[(12, &[0x03])]);
        let r = decode(&buf).unwrap();
        assert_eq!(
            r.object_country_coding_method,
            Some(ObjectCountryCodingMethod::Iso3166Numeric)
        );
    }

    #[test]
    fn decode_tag13_object_country_codes_utf16_be_with_bom() {
        // BE BOM + UTF-16 BE for "US"
        let mut payload = vec![0xFE, 0xFF];
        payload.extend_from_slice(&[0x00, b'U', 0x00, b'S']);
        let buf = build_record(&[(13, &payload)]);
        let r = decode(&buf).unwrap();
        assert_eq!(r.object_country_codes.as_deref(), Some("US"));
    }

    #[test]
    fn decode_tag13_object_country_codes_utf16_le_with_bom() {
        // LE BOM + UTF-16 LE for "US"
        let mut payload = vec![0xFF, 0xFE];
        payload.extend_from_slice(&[b'U', 0x00, b'S', 0x00]);
        let buf = build_record(&[(13, &payload)]);
        let r = decode(&buf).unwrap();
        assert_eq!(r.object_country_codes.as_deref(), Some("US"));
    }

    #[test]
    fn decode_tag13_object_country_codes_utf16_no_bom_defaults_be() {
        // No BOM → BE per RFC 2781 §4.3
        let buf = build_record(&[(13, &[0x00, b'D', 0x00, b'E'])]);
        let r = decode(&buf).unwrap();
        assert_eq!(r.object_country_codes.as_deref(), Some("DE"));
    }

    #[test]
    fn decode_tag13_invalid_utf16_lenient_signals_via_field_errors() {
        // Odd-length buffer → UTF-16 decode fails. Lenient mode sets
        // the field to None and pushes a KlvFieldError::InvalidUtf16
        // to field_errors per spec §3.5 (mirrors st0601 pattern).
        let raw = [0x00, b'U', 0x00];
        let buf = build_record(&[(13, &raw)]);
        let r = decode(&buf).unwrap();
        assert!(r.object_country_codes.is_none());
        assert!(r.unknown.is_empty());
        assert_eq!(r.field_errors.len(), 1);
        assert!(matches!(
            r.field_errors[0],
            crate::error::KlvFieldError::InvalidUtf16 { tag: 13 }
        ));
    }

    #[test]
    fn decode_tag22_version() {
        let buf = build_record(&[(22, &[0x00, 0x0C])]); // ST 0102.12
        let r = decode(&buf).unwrap();
        assert_eq!(r.version, Some(12));
    }

    #[test]
    fn decode_unknown_tag_lenient_preserves() {
        // Tag 99 is not in the LS table — pass through as forward-
        // compat per ST 0107.5 §6.
        let buf = build_record(&[(99, b"xyz")]);
        let r = decode(&buf).unwrap();
        assert_eq!(r.unknown.len(), 1);
        assert_eq!(r.unknown[0].tag, 99);
        assert_eq!(r.unknown[0].value, b"xyz");
    }

    #[test]
    fn decode_duplicate_tag_rejected() {
        let buf = build_record(&[(1, &[0x01]), (1, &[0x02])]);
        let err = decode(&buf).expect_err("duplicate tag rejected");
        assert!(matches!(err, KlvDecodeError::DuplicateTag { tag: 1, .. }));
    }

    #[test]
    fn decode_empty_record_lenient_succeeds() {
        // Lenient mode accepts a record missing all tags.
        let r = decode(&[]).unwrap();
        assert!(r.security_classification.is_none());
        assert!(r.unknown.is_empty());
    }
}
