//! ST 0806.4 Remote Video Terminal (RVT) Local Set typed layer.
//!
//! The RVT LS is **standalone-capable**: it carries its own 16-byte
//! Universal Label ([`RVT_LS_UL`]) and, per ST 0806.4-02/-04, a
//! timestamp-first Tag 2 plus a CRC-last Tag 1 when transmitted
//! independently. It is also **embeddable**: ST 0601 Tag 73 carries the
//! RVT LS *body* (no UL, no timestamp/CRC-position requirement) — see
//! [`crate::klv::st0601`].
//!
//! The checksum ([`RvtLs::crc32`]) is CRC-32/MPEG-2 (ISO/IEC 13818-1),
//! computed via the crate-private `klv::crc32` substrate — **not** the
//! ST 0601 16-bit running-sum in [`crate::klv::checksum`]. This is a real
//! divergence between the two typed layers, not a copy-paste artifact.
//!
//! Two nested repeatable Local Sets (Tag 12 [`RvtPoi`], Tag 13 [`RvtAoi`])
//! and one nested repeatable User Defined LS (Tag 11 [`RvtUserData`])
//! round out the schema (ST 0806.4 Tables 8-2/8-3/8-4).

pub(crate) mod decode;
pub(crate) mod model;
pub(crate) mod tags;

#[cfg(test)]
mod tests;

pub use decode::{decode, decode_standalone};
pub use model::{
    RVT_AOI_LS_UL, RVT_LS_UL, RVT_POI_LS_UL, RVT_USER_DEFINED_LS_UL, RvtAoi, RvtAoiType, RvtLs,
    RvtPoi, RvtPoiType, RvtUserData, RvtUserDataType,
};
