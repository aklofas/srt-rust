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

pub(crate) mod decode;
pub(crate) mod emit;
pub(crate) mod encode;
pub(crate) mod enums;
pub(crate) mod model;
pub(crate) mod tags;
pub(crate) mod var_uint;
pub(crate) mod vtarget_pack;

#[cfg(test)]
mod tests;

pub use decode::{decode, decode_strict};
pub use encode::{
    encode, encode_standalone, encode_to_vec, encode_to_vec_standalone, encoded_len,
    encoded_len_standalone,
};
pub use model::VmtiLs;
pub use vtarget_pack::{VTargetPack, VTargetPackError};

/// 16-byte Universal Label for the ST 0903 VMTI Local Set, per MISB ST 0903.6.
/// Used by consumers carrying VMTI as its own KLV stream (separate
/// MPEG-TS PID, not nested in an ST 0601 record).
pub const VMTI_LS_UL: [u8; 16] = [
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x03, 0x06, 0x00, 0x00, 0x00,
];
