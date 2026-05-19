//! ST 0601 UAS Datalink Local Set typed layer.
//!
//! `UasDatalinkLs` is a flat plain struct that mirrors the wire format
//! directly. Composite types (`GeoPoint`, `Attitude`, `FieldOfView`,
//! `Corners`) are derived read-only views layered on top.
//!
//! Three decode entry points:
//! - [`decode`] — verifies checksum, accepts any UL.
//! - [`decode_unchecked`] — skips checksum verification (useful for known-
//!   broken-checksum captures), accepts any UL.
//! - [`decode_strict`] — verifies checksum + requires the ST 0601 family UL
//!   pattern.
//! - [`decode_strict_compliance`] — adds full ST 0107.5 conformance checks
//!   on top of `decode_strict`.
//!
//! ## Spec coverage
//!
//! [BLOCK ADDED IN TASK 6 — see plan docs/plans/2026-05-19-wave-6-klv-reorg.md Task 6]

pub(crate) mod decode;
pub(crate) mod encode;
pub(crate) mod mapping;
pub(crate) mod model;
pub(crate) mod tags;

#[cfg(test)]
mod tests;

pub use decode::{decode, decode_strict, decode_strict_compliance, decode_unchecked};
pub use encode::{encode, encode_to_vec, encode_with, encoded_len, encoded_len_with};
pub use model::{Attitude, Corners, EncodeConfig, FieldOfView, GeoPoint, UasDatalinkLs};
