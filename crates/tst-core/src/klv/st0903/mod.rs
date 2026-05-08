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

pub mod enums;
pub mod tags;
pub mod vtarget_pack;

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
    pub version_number: Option<u8>,
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

#[allow(unused_variables)] // Task 5 wires the body
pub fn decode(bytes: &[u8]) -> Result<VmtiLs, KlvDecodeError> {
    todo!("Task 5")
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
    // tests are added in Tasks 5, 6, 7
}
