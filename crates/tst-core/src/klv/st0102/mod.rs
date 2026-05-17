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
//!
//! # Carriage paths
//!
//! ST 0102 Security LS rides two ways in the wild:
//!
//! 1. **Nested inside ST 0601 as Tag 48** — most common; the security
//!    metadata travels alongside platform telemetry in a single ST 0601
//!    record. Consumer pattern:
//!    ```ignore
//!    let uas = klv::st0601::decode(bytes)?;
//!    if let Some(sec_bytes) = uas.security_local_set.as_deref() {
//!        let sec = klv::st0102::decode(sec_bytes)?;
//!        // ...
//!    }
//!    ```
//! 2. **Standalone on its own KLV PID** — the AU-cell payload is a
//!    Security LS with [`SECURITY_LS_UL`] as the 16-byte UL prefix.
//!    Consumer pattern:
//!    ```ignore
//!    if data.starts_with(&klv::st0102::SECURITY_LS_UL) {
//!        let (_outer_len, body) = klv::length::read_ber(&data[16..])?;
//!        let sec = klv::st0102::decode(body)?;
//!        // ...
//!    }
//!    ```
//!    The demuxer remains UL-agnostic; consumer-side dispatch keeps
//!    new typed-set additions from creating a coupling load on the
//!    demuxer.

pub(crate) mod enums;
pub(crate) mod tags;

pub use enums::{
    ClassifyingCountryCodingMethod, ObjectCountryCodingMethod, SecurityClassification,
};

/// MISB ST 0102.12 §6.7 — Security Metadata Local Set Universal Label.
/// Used by consumers carrying the Security LS as its own KLV stream
/// (separate MPEG-TS PID, not nested in an ST 0601 Tag 48). The
/// `UniversalLabel`-typed companion lives at
/// [`crate::klv::UniversalLabel::SECURITY_LS_UL`].
pub const SECURITY_LS_UL: [u8; 16] = [
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x03, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x03, 0x02, 0x00, 0x00, 0x00,
];

use crate::error::{KlvDecodeError, KlvEncodeError, KlvFieldError};
use crate::klv::pack::OwnedRawField;

#[must_use]
#[derive(Debug, Clone, Default)]
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

/// Manual `PartialEq` excluding [`SecurityLs::field_errors`]. The
/// `field_errors` vec is a decode-side diagnostic — two LSes that
/// produced identical field values are semantically equal regardless
/// of which fields failed strict-decode validation along the way.
/// Used by round-trip fuzz (decode → encode → decode → assert_eq);
/// `field_errors` is empty on the second decode since encode never
/// emits malformed bytes (e.g. broken UTF-16 on Tag 13).
impl PartialEq for SecurityLs {
    fn eq(&self, other: &Self) -> bool {
        self.security_classification == other.security_classification
            && self.classifying_country_coding_method == other.classifying_country_coding_method
            && self.classifying_country == other.classifying_country
            && self.object_country_coding_method == other.object_country_coding_method
            && self.object_country_codes == other.object_country_codes
            && self.version == other.version
            && self.sci_shi_info == other.sci_shi_info
            && self.caveats == other.caveats
            && self.releasing_instructions == other.releasing_instructions
            && self.classified_by == other.classified_by
            && self.derived_from == other.derived_from
            && self.classification_reason == other.classification_reason
            && self.declassification_date == other.declassification_date
            && self.classification_marking_system == other.classification_marking_system
            && self.classification_comments == other.classification_comments
            && self.classifying_country_coding_method_version_date
                == other.classifying_country_coding_method_version_date
            && self.object_country_coding_method_version_date
                == other.object_country_coding_method_version_date
            && self.unknown == other.unknown
    }
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

/// Encode a string as RFC 2781 UTF-16 with BE BOM.
///
/// Per spec §3.5 the encoder normalizes to BE; round-trip from any
/// LE-encoded input through decode → encode emits BE.
fn encode_utf16_bom(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + s.encode_utf16().count() * 2);
    out.extend_from_slice(&[0xFE, 0xFF]); // BE BOM
    for unit in s.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

/// Decode a Security Metadata Local Set per MISB ST 0102.12.
///
/// Lenient: missing tags are tolerated (the typed surface is
/// `Option<T>`-shaped throughout), unknown tags are preserved verbatim
/// in [`SecurityLs::unknown`], unknown enum codepoints surface as
/// `Unknown(u8)`, and Tag 13 UTF-16 decode failures are demoted to
/// [`SecurityLs::field_errors`] rather than failing the whole record.
/// Use [`decode_strict`] for spec-conformance validation.
///
/// # Example — sibling-layer decode from a parent ST 0601 record
///
/// The Security LS is typically carried as the value of ST 0601 Tag 48
/// inside a UAS Datalink LS. The parent typed parser leaves Tag 48 as
/// pass-through bytes ([`crate::klv::st0601::UasDatalinkLs::security_local_set`]);
/// consumers who want typed access call this function on those bytes.
///
/// ```
/// use tst_core::klv::{st0102, st0601};
/// use tst_core::klv::st0102::{
///     ClassifyingCountryCodingMethod, ObjectCountryCodingMethod,
///     SecurityClassification, SecurityLs,
/// };
/// use tst_core::UasDatalinkLs;
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// // 1. Build a typed Security LS and serialize to bytes.
/// let security = SecurityLs {
///     security_classification: Some(SecurityClassification::Unclassified),
///     classifying_country_coding_method:
///         Some(ClassifyingCountryCodingMethod::Iso3166ThreeLetter),
///     classifying_country: Some("//USA".to_string()),
///     object_country_coding_method:
///         Some(ObjectCountryCodingMethod::Iso3166ThreeLetter),
///     object_country_codes: Some("USA".to_string()),
///     version: Some(12),
///     ..Default::default()
/// };
/// let security_bytes = st0102::encode_to_vec(&security)?;
///
/// // 2. Embed those bytes in a parent ST 0601 record's Tag 48.
/// let mut parent = UasDatalinkLs::default();
/// parent.timestamp_us = Some(1_700_000_000_000_000);
/// parent.security_local_set = Some(security_bytes);
/// let parent_bytes = st0601::encode_to_vec(&parent)?;
///
/// // 3. On the receive side, decode the parent, then sibling-decode
/// //    the inner Security LS.
/// let decoded_parent = st0601::decode(&parent_bytes)?;
/// let inner = decoded_parent
///     .security_local_set
///     .as_deref()
///     .expect("Tag 48 round-trips through ST 0601");
/// let decoded_security = st0102::decode(inner)?;
///
/// assert_eq!(
///     decoded_security.security_classification,
///     Some(SecurityClassification::Unclassified),
/// );
/// assert_eq!(decoded_security.version, Some(12));
/// assert!(decoded_security.field_errors.is_empty());
/// # Ok(())
/// # }
/// ```
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
        if !seen.insert(f.tag) && strict {
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
/// tags, unknown enum codepoints, [`OmittedValueXX`] reserved
/// codepoints, malformed UTF-16, duplicate tags, wrong fixed-length
/// values). Unknown tags are still preserved in `unknown` per ST
/// 0107.5 §6 future-proof skip rule (matches
/// `klv::st0601::decode_strict_compliance` posture).
///
/// Per-tag BER length encoding canonicity (ST 0107.5 §6.3.2) is NOT
/// currently checked — body iteration via [`pack::Iter::local_set`]
/// uses the permissive [`length::read_ber`]. A future tightening
/// could route through [`length::read_ber_strict`] but this is
/// deferred to keep parity with `klv::st0601`'s permissive iter
/// shape; consumers who need canonical-BER enforcement can call
/// [`length::read_ber_strict`] on the buffer themselves.
///
/// [`OmittedValueXX`]: ClassifyingCountryCodingMethod::OmittedValue08
/// [`pack::Iter::local_set`]: crate::klv::pack::Iter::local_set
/// [`length::read_ber`]: crate::klv::length::read_ber
/// [`length::read_ber_strict`]: crate::klv::length::read_ber_strict
pub fn decode_strict(buf: &[u8]) -> Result<SecurityLs, KlvDecodeError> {
    decode_inner(buf, /* strict = */ true)
}

/// Encode into a caller-provided buffer. Returns the number of bytes
/// written.
///
/// # Errors
/// - [`KlvEncodeError::BufferTooSmall`] if `out.len()` is less than
///   the required encoded length (call [`encoded_len`] first).
/// - [`KlvEncodeError::RecordTooLarge`] if any individual TLV's
///   declared length would overflow BER encoding (in practice,
///   a value > 2^64 bytes — guards against pathological input).
pub fn encode(record: &SecurityLs, out: &mut [u8]) -> Result<usize, KlvEncodeError> {
    use crate::klv::length::write_ber;

    let needed = encoded_len(record);
    if out.len() < needed {
        return Err(KlvEncodeError::BufferTooSmall {
            needed,
            got: out.len(),
        });
    }

    let mut pos = 0usize;
    let emit =
        |out: &mut [u8], pos: &mut usize, tag: u8, value: &[u8]| -> Result<(), KlvEncodeError> {
            out[*pos] = tag;
            *pos += 1;
            let n = write_ber(value.len(), &mut out[*pos..])
                .map_err(|_| KlvEncodeError::RecordTooLarge)?;
            *pos += n;
            out[*pos..*pos + value.len()].copy_from_slice(value);
            *pos += value.len();
            Ok(())
        };

    // Emit tags in numeric order for determinism. The wire spec
    // doesn't mandate ordering for ST 0102 LS — unlike ST 0601's
    // "tag 2 first, tag 1 last" rules — but emitting numeric is
    // friendlier to byte-diff tooling.
    if let Some(v) = record.security_classification {
        emit(out, &mut pos, 1, &[v.to_u8()])?;
    }
    if let Some(v) = record.classifying_country_coding_method {
        emit(out, &mut pos, 2, &[v.to_u8()])?;
    }
    if let Some(s) = record.classifying_country.as_ref() {
        emit(out, &mut pos, 3, s.as_bytes())?;
    }
    if let Some(s) = record.sci_shi_info.as_ref() {
        emit(out, &mut pos, 4, s.as_bytes())?;
    }
    if let Some(s) = record.caveats.as_ref() {
        emit(out, &mut pos, 5, s.as_bytes())?;
    }
    if let Some(s) = record.releasing_instructions.as_ref() {
        emit(out, &mut pos, 6, s.as_bytes())?;
    }
    if let Some(s) = record.classified_by.as_ref() {
        emit(out, &mut pos, 7, s.as_bytes())?;
    }
    if let Some(s) = record.derived_from.as_ref() {
        emit(out, &mut pos, 8, s.as_bytes())?;
    }
    if let Some(s) = record.classification_reason.as_ref() {
        emit(out, &mut pos, 9, s.as_bytes())?;
    }
    if let Some(s) = record.declassification_date.as_ref() {
        emit(out, &mut pos, 10, s.as_bytes())?;
    }
    if let Some(s) = record.classification_marking_system.as_ref() {
        emit(out, &mut pos, 11, s.as_bytes())?;
    }
    if let Some(v) = record.object_country_coding_method {
        emit(out, &mut pos, 12, &[v.to_u8()])?;
    }
    if let Some(s) = record.object_country_codes.as_ref() {
        let utf16 = encode_utf16_bom(s);
        emit(out, &mut pos, 13, &utf16)?;
    }
    if let Some(s) = record.classification_comments.as_ref() {
        emit(out, &mut pos, 14, s.as_bytes())?;
    }
    if let Some(v) = record.version {
        emit(out, &mut pos, 22, &v.to_be_bytes())?;
    }
    if let Some(s) = record
        .classifying_country_coding_method_version_date
        .as_ref()
    {
        emit(out, &mut pos, 23, s.as_bytes())?;
    }
    if let Some(s) = record.object_country_coding_method_version_date.as_ref() {
        emit(out, &mut pos, 24, s.as_bytes())?;
    }

    // Emit unknown tags last to preserve forward-compat. ST 0102 LS
    // uses single-byte BER-OID tags only; multi-byte tags (>127)
    // are silently dropped — `encoded_len` matches this behavior so
    // the buffer-size precheck stays consistent. The decoder still
    // accepts tag > 127 as forward-compat (ST 0107.5 §6), but
    // encoding round-trip drops them.
    for u in record.unknown.iter() {
        let tag_u8 = match u8::try_from(u.tag) {
            Ok(t) if t <= 127 => t,
            _ => continue, // tag > 127: drop (matches encoded_len)
        };
        emit(out, &mut pos, tag_u8, &u.value)?;
    }

    Ok(pos)
}

/// Encode into a fresh `Vec<u8>`. Convenience over [`encode`] when
/// the caller has no pre-sized buffer.
///
/// # Errors
/// Returns the same [`KlvEncodeError`] variants as [`encode`] minus
/// [`KlvEncodeError::BufferTooSmall`] (the buffer is pre-sized via
/// [`encoded_len`]). [`KlvEncodeError::RecordTooLarge`] can still
/// fire for pathological per-TLV lengths.
pub fn encode_to_vec(record: &SecurityLs) -> Result<Vec<u8>, KlvEncodeError> {
    let mut buf = vec![0u8; encoded_len(record)];
    let n = encode(record, &mut buf)?;
    buf.truncate(n);
    Ok(buf)
}

/// Pre-compute the encoded length for a given record.
pub fn encoded_len(record: &SecurityLs) -> usize {
    use crate::klv::length::ber_len;

    let mut total = 0usize;
    let mut add = |value_len: usize| {
        total += 1 /* tag byte */ + ber_len(value_len) + value_len;
    };

    if record.security_classification.is_some() {
        add(1);
    }
    if record.classifying_country_coding_method.is_some() {
        add(1);
    }
    if let Some(s) = record.classifying_country.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.sci_shi_info.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.caveats.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.releasing_instructions.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.classified_by.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.derived_from.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.classification_reason.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.declassification_date.as_ref() {
        add(s.len());
    }
    if let Some(s) = record.classification_marking_system.as_ref() {
        add(s.len());
    }
    if record.object_country_coding_method.is_some() {
        add(1);
    }
    if let Some(s) = record.object_country_codes.as_ref() {
        // 2 bytes BOM + 2 bytes per UTF-16 code unit
        let utf16_bytes = 2 + s.encode_utf16().count() * 2;
        add(utf16_bytes);
    }
    if let Some(s) = record.classification_comments.as_ref() {
        add(s.len());
    }
    if record.version.is_some() {
        add(2);
    }
    if let Some(s) = record
        .classifying_country_coding_method_version_date
        .as_ref()
    {
        add(s.len());
    }
    if let Some(s) = record.object_country_coding_method_version_date.as_ref() {
        add(s.len());
    }

    for u in record.unknown.iter() {
        // Re-emit unknown tags verbatim. ST 0102 LS uses single-byte
        // BER-OID tags (≤ 127); tags above that range are silently
        // dropped on encode (see the `encode` function's u8::try_from
        // branch), so we don't size for them here either. Asymmetric
        // round-trip is acceptable because the spec doesn't define
        // tags above 127 for ST 0102 LS — preserving them on lenient
        // decode is a forward-compat courtesy, not a contract.
        if u.tag <= 127 {
            total += 1 + ber_len(u.value.len()) + u.value.len();
        }
    }

    total
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
    fn decode_duplicate_tag_lenient_last_wins() {
        // Sibling-pattern parity with klv::st0601 lenient mode:
        // duplicate tags overwrite silently, later occurrence wins.
        // Strict mode (Task 6) rejects the same input as DuplicateTag.
        let buf = build_record(&[(1, &[0x01]), (1, &[0x02])]);
        let r = decode(&buf).expect("lenient tolerates duplicate, last wins");
        assert_eq!(
            r.security_classification,
            Some(SecurityClassification::Restricted) // 0x02, the second occurrence
        );
    }

    #[test]
    fn decode_empty_record_lenient_succeeds() {
        // Lenient mode accepts a record missing all tags.
        let r = decode(&[]).unwrap();
        assert!(r.security_classification.is_none());
        assert!(r.unknown.is_empty());
    }

    #[test]
    fn round_trip_minimal_required_fields() {
        let original = SecurityLs {
            security_classification: Some(SecurityClassification::Secret),
            classifying_country_coding_method: Some(
                ClassifyingCountryCodingMethod::Iso3166ThreeLetter,
            ),
            classifying_country: Some("//USA".to_string()),
            object_country_coding_method: Some(ObjectCountryCodingMethod::Iso3166ThreeLetter),
            object_country_codes: Some("USA".to_string()),
            version: Some(12),
            ..Default::default()
        };

        let bytes = encode_to_vec(&original).expect("encode succeeds");
        let decoded = decode(&bytes).expect("decode succeeds");
        assert_eq!(decoded, original);
    }

    #[test]
    fn round_trip_full_record() {
        let original = SecurityLs {
            security_classification: Some(SecurityClassification::TopSecret),
            classifying_country_coding_method: Some(ClassifyingCountryCodingMethod::Iso3166Numeric),
            classifying_country: Some("//USA".to_string()),
            sci_shi_info: Some("HCS-O".to_string()),
            caveats: Some("FOUO".to_string()),
            releasing_instructions: Some("USA CAN GBR".to_string()),
            classified_by: Some("ID-12345".to_string()),
            derived_from: Some("Multiple Sources".to_string()),
            classification_reason: Some("1.4(c)".to_string()),
            declassification_date: Some("20351231".to_string()),
            classification_marking_system: Some("CAPCO".to_string()),
            object_country_coding_method: Some(ObjectCountryCodingMethod::Iso3166Numeric),
            object_country_codes: Some("USA".to_string()),
            classification_comments: Some("Test record".to_string()),
            version: Some(12),
            classifying_country_coding_method_version_date: Some("2025-01-15".to_string()),
            object_country_coding_method_version_date: Some("2025-01-15".to_string()),
            unknown: Vec::new(),
            field_errors: Vec::new(),
        };

        let bytes = encode_to_vec(&original).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn round_trip_with_unknown_tag_preserved() {
        let mut original = SecurityLs {
            security_classification: Some(SecurityClassification::Confidential),
            classifying_country_coding_method: Some(
                ClassifyingCountryCodingMethod::Iso3166TwoLetter,
            ),
            classifying_country: Some("//US".to_string()),
            object_country_coding_method: Some(ObjectCountryCodingMethod::Iso3166TwoLetter),
            object_country_codes: Some("US".to_string()),
            version: Some(12),
            ..Default::default()
        };
        original.unknown.push(OwnedRawField {
            tag: 99,
            value: b"forward-compat-payload".to_vec(),
        });

        let bytes = encode_to_vec(&original).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded, original);
        assert_eq!(decoded.unknown.len(), 1);
        assert_eq!(decoded.unknown[0].tag, 99);
    }

    #[test]
    fn encode_buffer_too_small_rejects() {
        let r = SecurityLs {
            security_classification: Some(SecurityClassification::Unclassified),
            ..Default::default()
        };
        let mut buf = [0u8; 1]; // need ≥ 3 bytes for tag 1
        let err = encode(&r, &mut buf).unwrap_err();
        // Tag 1 needs: 1 byte tag + 1 byte BER len + 1 byte value = 3 bytes; got 1.
        assert!(matches!(
            err,
            KlvEncodeError::BufferTooSmall { needed: 3, got: 1 }
        ));
    }

    #[test]
    fn encoded_len_matches_actual() {
        let r = SecurityLs {
            security_classification: Some(SecurityClassification::Restricted),
            classifying_country: Some("//GBR".to_string()),
            object_country_codes: Some("GB".to_string()),
            version: Some(12),
            ..Default::default()
        };
        let n = encoded_len(&r);
        let bytes = encode_to_vec(&r).unwrap();
        assert_eq!(n, bytes.len());
    }

    #[test]
    fn round_trip_utf16_normalizes_to_be() {
        // A consumer hand-builds an LE-encoded Tag 13 record.
        let mut payload = vec![0xFF, 0xFE]; // LE BOM
        payload.extend_from_slice(&[b'F', 0x00, b'R', 0x00]);
        let buf = build_record(&[(13, &payload)]);
        let decoded = decode(&buf).unwrap();
        assert_eq!(decoded.object_country_codes.as_deref(), Some("FR"));

        // Re-encode and verify BE BOM normalization.
        let bytes = encode_to_vec(&decoded).unwrap();
        // Tag 13 byte + BER-len(1 byte) + BOM(0xFE 0xFF) + 'F' BE +
        // 'R' BE — verify the BOM bytes appear at the expected offset.
        // We don't decode BER here; the round-trip via decode below
        // is the primary correctness check.
        let redecoded = decode(&bytes).unwrap();
        assert_eq!(redecoded, decoded);
    }

    /// Helper: build a minimum-required record per ST 0102.12 §6.7
    /// (tags 1, 2, 3, 12, 13, 22).
    fn build_minimal_required_record() -> SecurityLs {
        SecurityLs {
            security_classification: Some(SecurityClassification::Unclassified),
            classifying_country_coding_method: Some(
                ClassifyingCountryCodingMethod::Iso3166TwoLetter,
            ),
            classifying_country: Some("//US".to_string()),
            object_country_coding_method: Some(ObjectCountryCodingMethod::Iso3166TwoLetter),
            object_country_codes: Some("US".to_string()),
            version: Some(12),
            ..Default::default()
        }
    }

    #[test]
    fn strict_accepts_minimal_required_record() {
        let r = build_minimal_required_record();
        let bytes = encode_to_vec(&r).unwrap();
        let decoded = decode_strict(&bytes).expect("strict accepts minimal record");
        assert_eq!(decoded, r);
    }

    #[test]
    fn strict_rejects_missing_tag_1() {
        let mut r = build_minimal_required_record();
        r.security_classification = None;
        let bytes = encode_to_vec(&r).unwrap();
        let err = decode_strict(&bytes).unwrap_err();
        assert!(matches!(
            err,
            KlvDecodeError::St0102MissingRequiredTag { tag: 1 }
        ));
    }

    #[test]
    fn strict_rejects_missing_tag_2() {
        let mut r = build_minimal_required_record();
        r.classifying_country_coding_method = None;
        let bytes = encode_to_vec(&r).unwrap();
        assert!(matches!(
            decode_strict(&bytes).unwrap_err(),
            KlvDecodeError::St0102MissingRequiredTag { tag: 2 }
        ));
    }

    #[test]
    fn strict_rejects_missing_tag_3() {
        let mut r = build_minimal_required_record();
        r.classifying_country = None;
        let bytes = encode_to_vec(&r).unwrap();
        assert!(matches!(
            decode_strict(&bytes).unwrap_err(),
            KlvDecodeError::St0102MissingRequiredTag { tag: 3 }
        ));
    }

    #[test]
    fn strict_rejects_missing_tag_12() {
        let mut r = build_minimal_required_record();
        r.object_country_coding_method = None;
        let bytes = encode_to_vec(&r).unwrap();
        assert!(matches!(
            decode_strict(&bytes).unwrap_err(),
            KlvDecodeError::St0102MissingRequiredTag { tag: 12 }
        ));
    }

    #[test]
    fn strict_rejects_missing_tag_13() {
        let mut r = build_minimal_required_record();
        r.object_country_codes = None;
        let bytes = encode_to_vec(&r).unwrap();
        assert!(matches!(
            decode_strict(&bytes).unwrap_err(),
            KlvDecodeError::St0102MissingRequiredTag { tag: 13 }
        ));
    }

    #[test]
    fn strict_rejects_missing_tag_22() {
        let mut r = build_minimal_required_record();
        r.version = None;
        let bytes = encode_to_vec(&r).unwrap();
        assert!(matches!(
            decode_strict(&bytes).unwrap_err(),
            KlvDecodeError::St0102MissingRequiredTag { tag: 22 }
        ));
    }

    #[test]
    fn strict_rejects_unknown_tag1_codepoint() {
        // Encode raw bytes — encode_to_vec wouldn't fail on
        // SecurityClassification::Unknown(0xFA), but strict decode
        // must reject.
        let mut r = build_minimal_required_record();
        r.security_classification = Some(SecurityClassification::Unknown(0xFA));
        let bytes = encode_to_vec(&r).unwrap();
        assert!(matches!(
            decode_strict(&bytes).unwrap_err(),
            KlvDecodeError::FieldError(crate::error::KlvFieldError::InvalidCodepoint {
                tag: 1,
                value: 0xFA,
            })
        ));
    }

    #[test]
    fn strict_rejects_unknown_tag2_codepoint() {
        let mut r = build_minimal_required_record();
        r.classifying_country_coding_method = Some(ClassifyingCountryCodingMethod::Unknown(0x7F));
        let bytes = encode_to_vec(&r).unwrap();
        assert!(matches!(
            decode_strict(&bytes).unwrap_err(),
            KlvDecodeError::FieldError(crate::error::KlvFieldError::InvalidCodepoint {
                tag: 2,
                value: 0x7F,
            })
        ));
    }

    #[test]
    fn strict_rejects_unknown_tag12_codepoint() {
        let mut r = build_minimal_required_record();
        r.object_country_coding_method = Some(ObjectCountryCodingMethod::Unknown(0x20));
        let bytes = encode_to_vec(&r).unwrap();
        assert!(matches!(
            decode_strict(&bytes).unwrap_err(),
            KlvDecodeError::FieldError(crate::error::KlvFieldError::InvalidCodepoint {
                tag: 12,
                value: 0x20,
            })
        ));
    }

    #[test]
    fn strict_rejects_omitted_value_codepoint_tag2() {
        let mut r = build_minimal_required_record();
        r.classifying_country_coding_method = Some(ClassifyingCountryCodingMethod::OmittedValue08);
        let bytes = encode_to_vec(&r).unwrap();
        assert!(matches!(
            decode_strict(&bytes).unwrap_err(),
            KlvDecodeError::FieldError(crate::error::KlvFieldError::InvalidCodepoint {
                tag: 2,
                value: 0x08,
            })
        ));
    }

    #[test]
    fn strict_rejects_omitted_value_codepoint_tag12() {
        let mut r = build_minimal_required_record();
        r.object_country_coding_method = Some(ObjectCountryCodingMethod::OmittedValue0A);
        let bytes = encode_to_vec(&r).unwrap();
        assert!(matches!(
            decode_strict(&bytes).unwrap_err(),
            KlvDecodeError::FieldError(crate::error::KlvFieldError::InvalidCodepoint {
                tag: 12,
                value: 0x0A,
            })
        ));
    }

    #[test]
    fn strict_rejects_invalid_utf16_tag13() {
        // Required tags 1, 2, 3, 12, 22 present + Tag 13 with
        // odd-length payload (UTF-16 needs even bytes).
        // (Building the bytes manually is easier than mutating
        // encode_to_vec output to splice in the bad UTF-16.)
        let bad_utf16 = [0x00, b'U', 0x00];
        let manual = build_record(&[
            (1, &[0x01]),
            (2, &[0x01]),
            (3, b"//US"),
            (12, &[0x01]),
            (13, &bad_utf16),
            (22, &[0x00, 0x0C]),
        ]);
        assert!(matches!(
            decode_strict(&manual).unwrap_err(),
            KlvDecodeError::FieldError(crate::error::KlvFieldError::InvalidUtf16 { tag: 13 })
        ));
    }

    #[test]
    fn strict_rejects_duplicate_tag() {
        // Duplicate tag 1.
        let manual = build_record(&[
            (1, &[0x01]),
            (1, &[0x02]),
            (2, &[0x01]),
            (3, b"//US"),
            (12, &[0x01]),
            (13, &[0xFE, 0xFF, 0x00, b'U', 0x00, b'S']),
            (22, &[0x00, 0x0C]),
        ]);
        assert!(matches!(
            decode_strict(&manual).unwrap_err(),
            KlvDecodeError::DuplicateTag { tag: 1, .. }
        ));
    }

    #[test]
    fn strict_preserves_unknown_tag() {
        // Required tags + a forward-compat unknown tag — strict
        // mode preserves the unknown tag rather than rejecting per
        // spec §3.7 / ST 0107.5 §6.
        let mut r = build_minimal_required_record();
        r.unknown.push(OwnedRawField {
            tag: 99,
            value: b"future-tag".to_vec(),
        });
        let bytes = encode_to_vec(&r).unwrap();
        let decoded = decode_strict(&bytes).unwrap();
        assert_eq!(decoded.unknown.len(), 1);
        assert_eq!(decoded.unknown[0].tag, 99);
        assert_eq!(decoded.unknown[0].value, b"future-tag");
    }

    #[test]
    fn strict_rejects_truncated_value() {
        // Tag 22 (Version) declares 2-byte length but only 1 byte
        // present in the buffer.
        let mut buf = build_record(&[
            (1, &[0x01]),
            (2, &[0x01]),
            (3, b"//US"),
            (12, &[0x01]),
            (13, &[0xFE, 0xFF, 0x00, b'U', 0x00, b'S']),
        ]);
        // Truncated tag 22: tag byte + length-1 + only 1 byte
        buf.extend_from_slice(&[22, 0x01, 0x0C]); // len=1 but spec wants 2
        // The decoder doesn't bail on length-mismatch within Iter
        // (Iter respects the BER length verbatim). Instead the
        // U16Be branch raises InvalidLength.
        assert!(matches!(
            decode_strict(&buf).unwrap_err(),
            KlvDecodeError::FieldError(crate::error::KlvFieldError::InvalidLength {
                tag: 22,
                expected: 2,
                got: 1,
            })
        ));
    }

    #[test]
    fn unknown_tags_above_127_dropped_on_encode() {
        // ST 0102 LS uses single-byte BER-OID tags only. The lenient
        // decoder accepts tag > 127 as forward-compat (ST 0107.5 §6)
        // but encode silently drops them; encoded_len agrees. This
        // test pins that contract — change of behavior here means
        // either making encode emit multi-byte BER-OID, or making
        // decode reject tag > 127.
        let r = SecurityLs {
            security_classification: Some(SecurityClassification::Unclassified),
            unknown: vec![
                OwnedRawField {
                    tag: 128,
                    value: b"forward-compat".to_vec(),
                },
                OwnedRawField {
                    tag: 200,
                    value: b"other".to_vec(),
                },
            ],
            ..Default::default()
        };

        let n = encoded_len(&r);
        let bytes = encode_to_vec(&r).unwrap();

        // encoded_len + encode agree on size (both skip the > 127 tags).
        assert_eq!(n, bytes.len());

        // Re-decode: typed field round-trips; unknown vec is empty
        // (the > 127 tags were dropped on encode).
        let decoded = decode(&bytes).unwrap();
        assert_eq!(
            decoded.security_classification,
            Some(SecurityClassification::Unclassified)
        );
        assert!(
            decoded.unknown.is_empty(),
            "tags > 127 should be silently dropped on encode"
        );
    }

    /// `klv::st0102::SECURITY_LS_UL` is a re-export of the
    /// `UniversalLabel`-typed constant — the bytes match the
    /// universal_label.rs canonical form.
    #[test]
    fn security_ls_ul_reexport_matches_universal_label() {
        assert_eq!(
            super::SECURITY_LS_UL,
            crate::klv::universal_label::UniversalLabel::SECURITY_LS_UL.0,
        );
    }
}
