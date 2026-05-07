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

/// Lenient decode — tolerates malformed input where possible.
pub fn decode(_buf: &[u8]) -> Result<SecurityLs, KlvDecodeError> {
    todo!("Task 4")
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
    // populated in Tasks 2-7
}
