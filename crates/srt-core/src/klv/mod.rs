//! KLV codec — generic substrate plus the typed ST 0601 UAS Datalink Local Set.
//!
//! Two layers:
//!
//! - **Generic substrate** (`universal_label`, `length`, `pack`, `imapb`,
//!   `checksum`) — handles raw KLV machinery: 16-byte SMPTE Universal Labels,
//!   BER short/long and BER-OID length encodings, IMAPB integer↔float
//!   mapping per ST 1201 §8, ST 0601 16-bit running-sum checksum, and
//!   generic local-set / universal-set pack-and-iterate.
//! - **Typed ST 0601 layer** (`st0601`) — the curated working subset of
//!   ST 0601 tags as a flat `UasDatalinkLs` struct with eager `decode` /
//!   free-function `encode`. Anything not typed-modeled passes through as
//!   `OwnedRawField` in `record.unknown`.
//!
//! Top-level re-exports (substrate types likely useful to consumers) live in
//! the crate root via `crate::lib.rs`.

pub mod checksum;
pub mod imapb;
pub mod length;
pub mod pack;
pub mod st0601;
pub mod st0605;
pub mod st1910;
pub mod universal_label;

pub use pack::{Iter, OwnedRawField, RawField};
pub use st0605::{PrecisionTimeStampPack, TimeStatus};
pub use st1910::{unwrap_au_cell, wrap_au_cell};
pub use universal_label::UniversalLabel;
