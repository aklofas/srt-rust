//! KLV codec — generic substrate plus the typed ST 0601 UAS Datalink Local Set.
//!
//! Two layers:
//!
//! - **Generic substrate** (`universal_label`, `length`, `pack`, `imapb`,
//!   `checksum`) — handles raw KLV machinery: 16-byte SMPTE Universal Labels,
//!   BER short/long and BER-OID length encodings, IMAPB integer↔float
//!   mapping per ST 1201 §7, ST 0601 16-bit running-sum checksum, and
//!   generic local-set / universal-set pack-and-iterate.
//! - **Typed ST 0601 layer** (`st0601`) — the curated working subset of
//!   ST 0601 tags as a flat `UasDatalinkLs` struct with eager `decode` /
//!   free-function `encode`. Anything not typed-modeled passes through as
//!   `OwnedRawField` in `record.unknown`.
//! - **Typed ST 0102 layer** (`st0102`) — the Security Metadata Local Set
//!   as a flat `SecurityLs` struct with lenient/strict `decode` /
//!   free-function `encode`. Sibling typed parser to `st0601`; consumers
//!   typically reach this from `UasDatalinkLs::security_local_set`.
//!   Anything not typed-modeled passes through as `OwnedRawField` in
//!   `record.unknown`.
//!
//! MPEG-TS sync-metadata AU cell carriage lives at
//! [`crate::mpegts::au_cell`] (per ITU-T H.222.0 V9 §2.12.4.2 — that's
//! a TS-systems-layer concern, not a KLV substrate concern; the muxer
//! auto-wraps for `KlvStreamType::SynchronousMetadata` streams).
//!
//! Top-level re-exports (substrate types likely useful to consumers) live in
//! the crate root via `crate::lib.rs`.

pub mod checksum;
pub(crate) mod crc32;
pub mod imapb;
pub mod length;
pub mod pack;
pub mod st0102;
pub mod st0601;
pub mod st0605;
pub mod st0805;
pub mod st0806;
pub mod st0903;
pub mod st1010;
pub mod st1204;
pub mod universal_label;

pub use imapb::ImapbSpecial;
pub use pack::{OwnedRawField, RawField};
pub use st0102::{
    ClassifyingCountryCodingMethod, ObjectCountryCodingMethod, SECURITY_LS_UL,
    SecurityClassification, SecurityLs,
};
pub use st0605::{PrecisionTimeStampPack, TimeStatus};
pub use st0805::{CotConfig, platform_uid, spi_uid};
pub use st0806::{
    RVT_AOI_LS_UL, RVT_LS_UL, RVT_POI_LS_UL, RVT_USER_DEFINED_LS_UL, RvtAoi, RvtAoiType, RvtLs,
    RvtPoi, RvtPoiType, RvtUserData, RvtUserDataType,
};
pub use st0903::{VMTI_LS_UL, VTargetPack, VTargetPackError, VmtiLs};
pub use st1010::{SdccFlp, decode_sdcc_flp, encode_sdcc_flp_mode2};
pub use universal_label::UniversalLabel;
