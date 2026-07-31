//! MISB ST 0102.12 Security Metadata Local Set typed layer.
//!
//! **Stability: Provisional** — see the
//! [API stability reference](https://github.com/aklofas/ts-transformer/blob/main/docs/reference/api-stability.md).
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
//!
//! ## Spec coverage
//!
//! **Standard:** MISB ST 0102.12 Security Metadata Local Set
//! (LS form only — Universal Set form deferred).
//!
//! **Tags parsed (typed-modeled):** 1 (security classification),
//! 2 (classifying country coding method), 3 (classifying country),
//! 4 (security-SCI/SHI information), 5–11 (caveats + releasing
//! instructions + classified-by + derived-from + classification
//! reason + declassification date + marking system), 12 (object
//! country coding method), 13 (object country names), 14
//! (classification comments), 22 (LS version), 23–25
//! (PMS-classifying-by + date-by + comments). Required tags per
//! ST 0102.12 §6 Table 1: 1, 2, 3, 12, 13, 22 — [`decode_strict`]
//! rejects records missing any of these.
//!
//! **Tags preserved as `unknown`:** any tag not in the
//! typed-modeled set above — per ST 0107.5 §6.
//!
//! **Decode modes:**
//! - [`decode`] — lenient: tolerates missing required tags, unknown
//!   enum codepoints (decoded as `Unknown(u8)`), Tag 13 UTF-16
//!   decode failures (raw bytes preserved in `unknown`).
//! - [`decode_strict`] — strict: rejects missing required tags,
//!   unknown enum codepoints, `OmittedValueXX` codepoints,
//!   non-canonical BER, duplicate tags, malformed UTF-16.
//!
//! **Deferred per `docs/project/deferred-features.md`:** Universal Set form
//! of ST 0102 (LS-only on MPEG-TS+KLV streams).

pub(crate) mod decode;
pub(crate) mod encode;
pub(crate) mod enums;
pub(crate) mod model;
pub(crate) mod tags;

#[cfg(test)]
mod tests;

pub use decode::{decode, decode_strict};
pub use encode::{encode, encode_strict_compliance, encode_to_vec, encoded_len};
pub use enums::{
    ClassifyingCountryCodingMethod, ObjectCountryCodingMethod, SecurityClassification,
};
pub use model::SecurityLs;

/// MISB ST 0102.12 §6.7 — Security Metadata Local Set Universal Label.
/// Used by consumers carrying the Security LS as its own KLV stream
/// (separate MPEG-TS PID, not nested in an ST 0601 Tag 48). The
/// `UniversalLabel`-typed companion lives at
/// [`crate::klv::UniversalLabel::SECURITY_LS_UL`].
pub const SECURITY_LS_UL: [u8; 16] = [
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x03, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x03, 0x02, 0x00, 0x00, 0x00,
];
