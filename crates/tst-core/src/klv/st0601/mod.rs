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
//! - [`patch`] — byte-faithful tag-level patching: re-encode only the
//!   edited tags, copy every other TLV verbatim, recompute the checksum.
//!
//! ## Spec coverage
//!
//! **Standard:** MISB ST 0601.x UAS Datalink Local Set (versions
//! 13 + 17 + 19 covered; declared version preserved for caller
//! introspection).
//!
//! **Tags parsed (typed-modeled):** Tags 1 (checksum), 2 (timestamp),
//! tags 5–9 (platform attitude + heading + yaw), tags 10–19 (sensor
//! position + orientation + image source), tag 20 (range + rate),
//! tags 21–25 (target location + height + TTrue + track + velocity),
//! tags 26–33 (corner offsets + image coordinate), tags 40–47
//! (frame-center + alt + offset + range + slant), tag 48 (security
//! LS bytes — typed via [`crate::klv::st0102`]), tags 50–59 + 65–67
//! (extended platform / sensor / altitude fields), tag 74 (VMTI LS
//! bytes — typed via [`crate::klv::st0903`]), tags 75–91 (ellipsoid
//! heights + full corners + extended attitude + MIIS core identifier).
//!
//! **Tags preserved as `unknown` (`OwnedRawField`):** any tag not
//! in the typed-modeled set above — full payload bytes preserved per
//! ST 0107.5 §6 future-proof skip rule. Consumers reading
//! `record.unknown` can apply downstream-specific typed parsers.
//!
//! **Decode modes:**
//! - [`decode`] — verifies running-sum checksum; accepts any UL.
//! - [`decode_unchecked`] — skips checksum verification (useful for
//!   known-broken-checksum captures from older encoders).
//! - [`decode_strict`] — checksum + ST 0601 family UL pattern
//!   requirement.
//! - [`decode_strict_compliance`] — adds full ST 0107.5 conformance
//!   checks (BER canonicality, no duplicate tags, etc.).
//!
//! **Deferred per `docs/project/deferred-features.md`:** none — ST 0601
//! typed model is the most-complete of the 4 typed sets.

pub(crate) mod decode;
pub(crate) mod encode;
pub(crate) mod mapping;
pub(crate) mod model;
pub(crate) mod patch;
pub(crate) mod tags;

#[cfg(test)]
mod tests;

pub use decode::{decode, decode_strict, decode_strict_compliance, decode_unchecked};
pub use encode::{
    _mandatory_tags, encode, encode_strict_compliance, encode_to_vec, encode_with, encoded_len,
    encoded_len_with,
};
pub use mapping::{St0601SentinelMeaning, st0601_sentinel_meaning};
pub use model::{Attitude, Corners, EncodeConfig, FieldOfView, GeoPoint, UasDatalinkLs};
pub use patch::patch;
