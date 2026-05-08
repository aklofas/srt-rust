//! ST 0903.6 §10.2 Table 10 — VTargetPack typed layer.
//!
//! A VTargetPack is a BER-OID-prefixed ordered Pack. The first 1..=N
//! bytes are the Target ID encoded as BER-OID (V9 per Table 10's
//! "BER-OID V9" cell — i.e. up to 9 wire bytes). The remaining bytes
//! are a Local Set–encoded body using BER-OID tag plus BER short/long
//! length plus value, per ST 0903.6 §9.2 with byte-6 = 0x2B. Each
//! VTargetPack is itself prefixed with a BER outer-length when
//! serialized inside a VTargetSeries (Tag 101) — that outer length
//! is consumed by the series walker, not by `read_pack` /
//! `write_pack`.
#![allow(dead_code)]

use thiserror::Error;

/// Per-pack-tag wire encoding. Mirrors `tags::Encoding` in shape but
/// scoped to the pack's tag set: notably, `Utf8` is absent (no string
/// fields in Table 10), and the U24 RGB special form is added for
/// Tag 8 (`targetColor`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PackEncoding {
    /// Raw 1-byte unsigned (Tags 4, 5, 7, 23 per §10.2.2.5/.6/.8/.24).
    U8,
    /// Variable-length truncated big-endian unsigned, value bytes
    /// 1..=`max_bytes`. Wire form is the value's raw bytes (no length
    /// byte; the LS-encoded BER length supplies it). Mirrors the
    /// top-level `Encoding::VarUint` from `tags.rs`. Used for V2/V3/
    /// V4/V6 fields per Table 10.
    VarUint { max_bytes: u8 },
    /// Raw 3-byte RGB (Tag 8 `targetColor`, fixed length 3 per
    /// §10.2.2.9 — first byte = R, second = G, third = B).
    U24Rgb,
    /// IMAPB-encoded floating-point with linear range. Wire form is
    /// the raw bytes mapped via `klv::imapb::decode`.
    ImapbF64 { min: f64, max: f64 },
    /// Raw bytes (variable length); pass-through. Used for nested
    /// LSes (Tags 101 / 104 / 105), Series payloads (Tags 18 / 106 /
    /// 107), and the Defined Length Truncation Pack at Tag 17
    /// (`targetLocation`). Typed inner layers are deferred.
    RawBytes,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PackTagSpec {
    pub id: u8,
    pub name: &'static str,
    pub encoding: PackEncoding,
}

/// ST 0903.6 §10.2 Table 10 — VTargetPack body items in numeric tag
/// order. Deprecated tags (21, 102, 103) are intentionally omitted;
/// the lenient decoder treats them as unknown tags (preserved per
/// ST 0107.5 §6 future-proof skip rule). The leading BER-OID
/// `targetId` precedes the body and is NOT in this table — it has
/// no Tag (per §10.2.2.1: "has no Tag").
pub(crate) const PACK_TAGS: &[PackTagSpec] = &[
    PackTagSpec {
        id: 1,
        name: "targetCentroid",
        // §10.2.2.2 — uint V6 (range 1..=2^48-1, pixel number).
        encoding: PackEncoding::VarUint { max_bytes: 6 },
    },
    PackTagSpec {
        id: 2,
        name: "boundingBoxTopLeft",
        // §10.2.2.3 — uint V6 (pixel number).
        encoding: PackEncoding::VarUint { max_bytes: 6 },
    },
    PackTagSpec {
        id: 3,
        name: "boundingBoxBottomRight",
        // §10.2.2.4 — uint V6 (pixel number).
        encoding: PackEncoding::VarUint { max_bytes: 6 },
    },
    PackTagSpec {
        id: 4,
        name: "targetPriority",
        // §10.2.2.5 — uint, fixed length 1, valid 1..=255.
        encoding: PackEncoding::U8,
    },
    PackTagSpec {
        id: 5,
        name: "targetConfidenceLevel",
        // §10.2.2.6 — uint, fixed length 1, valid 0..=100 (percent).
        encoding: PackEncoding::U8,
    },
    PackTagSpec {
        id: 6,
        name: "targetHistory",
        // §10.2.2.7 — uint V2 (0..=65535 frames).
        encoding: PackEncoding::VarUint { max_bytes: 2 },
    },
    PackTagSpec {
        id: 7,
        name: "percentageOfTargetPixels",
        // §10.2.2.8 — uint, fixed length 1, valid 1..=100 (percent).
        encoding: PackEncoding::U8,
    },
    PackTagSpec {
        id: 8,
        name: "targetColor",
        // §10.2.2.9 — fixed length 3, R/G/B raw bytes.
        encoding: PackEncoding::U24Rgb,
    },
    PackTagSpec {
        id: 9,
        name: "targetIntensity",
        // §10.2.2.10 — uint V3 (24-bit dynamic range).
        encoding: PackEncoding::VarUint { max_bytes: 3 },
    },
    PackTagSpec {
        id: 10,
        name: "targetLocationOffsetLat",
        // §10.2.2.11 — IMAPB(-19.2, 19.2, 3), units °.
        encoding: PackEncoding::ImapbF64 {
            min: -19.2,
            max: 19.2,
        },
    },
    PackTagSpec {
        id: 11,
        name: "targetLocationOffsetLon",
        // §10.2.2.12 — IMAPB(-19.2, 19.2, 3), units °.
        encoding: PackEncoding::ImapbF64 {
            min: -19.2,
            max: 19.2,
        },
    },
    PackTagSpec {
        id: 12,
        name: "targetHae",
        // §10.2.2.13 — IMAPB(-900, 19000, 2), units m above WGS84.
        encoding: PackEncoding::ImapbF64 {
            min: -900.0,
            max: 19000.0,
        },
    },
    PackTagSpec {
        id: 13,
        name: "boundingBoxTopLeftLatOffset",
        // §10.2.2.14 — IMAPB(-19.2, 19.2, 3), units °.
        encoding: PackEncoding::ImapbF64 {
            min: -19.2,
            max: 19.2,
        },
    },
    PackTagSpec {
        id: 14,
        name: "boundingBoxTopLeftLonOffset",
        // §10.2.2.15 — IMAPB(-19.2, 19.2, 3), units °.
        encoding: PackEncoding::ImapbF64 {
            min: -19.2,
            max: 19.2,
        },
    },
    PackTagSpec {
        id: 15,
        name: "boundingBoxBottomRightLatOffset",
        // §10.2.2.16 — IMAPB(-19.2, 19.2, 3), units °.
        encoding: PackEncoding::ImapbF64 {
            min: -19.2,
            max: 19.2,
        },
    },
    PackTagSpec {
        id: 16,
        name: "boundingBoxBottomRightLonOffset",
        // §10.2.2.17 — IMAPB(-19.2, 19.2, 3), units °.
        encoding: PackEncoding::ImapbF64 {
            min: -19.2,
            max: 19.2,
        },
    },
    PackTagSpec {
        id: 17,
        name: "targetLocation",
        // §10.2.2.18 — Location, Defined Length Truncation Pack (V).
        // Typed inner layer deferred; pass-through bytes for now.
        encoding: PackEncoding::RawBytes,
    },
    PackTagSpec {
        id: 18,
        name: "geospatialContourSeries",
        // §10.2.2.19 — BoundarySeries (Series of Location). Typed
        // inner layer deferred.
        encoding: PackEncoding::RawBytes,
    },
    PackTagSpec {
        id: 19,
        name: "centroidPixRow",
        // §10.2.2.20 — uint V4 (1..=2^32-1).
        encoding: PackEncoding::VarUint { max_bytes: 4 },
    },
    PackTagSpec {
        id: 20,
        name: "centroidPixCol",
        // §10.2.2.21 — uint V4 (1..=2^32-1).
        encoding: PackEncoding::VarUint { max_bytes: 4 },
    },
    // Tag 21 is DEPRECATED in ST 0903.6 (§10.2.2.22). Decoders treat
    // any wire occurrence as an unknown tag (preserved in `unknown`
    // per ST 0107.5 §6); encoders do not emit it.
    PackTagSpec {
        id: 22,
        name: "algorithmId",
        // §10.2.2.23 — uint V3, references an Id from Algorithm Series.
        encoding: PackEncoding::VarUint { max_bytes: 3 },
    },
    PackTagSpec {
        id: 23,
        name: "detectionStatus",
        // §10.2.2.24 — uint, fixed length 1 (5 enumerated states +
        // Inactive). Length confirmed by §7.2 lifecycle table.
        encoding: PackEncoding::U8,
    },
    PackTagSpec {
        id: 101,
        name: "vMask",
        // §10.2.2.25 — VMask LS (V). Typed inner layer deferred.
        encoding: PackEncoding::RawBytes,
    },
    // Tags 102 and 103 are DEPRECATED in ST 0903.6 (§10.2.2.26 /
    // §10.2.2.27). Decoders treat any wire occurrence as an unknown
    // tag; encoders do not emit them.
    PackTagSpec {
        id: 104,
        name: "vTracker",
        // §10.2.2.28 — VTracker LS (V). Typed inner layer deferred.
        encoding: PackEncoding::RawBytes,
    },
    PackTagSpec {
        id: 105,
        name: "vChip",
        // §10.2.2.29 — VChip LS (V). Typed inner layer deferred.
        encoding: PackEncoding::RawBytes,
    },
    PackTagSpec {
        id: 106,
        name: "vChipSeries",
        // §10.2.2.30 — Series of VChip LS (V). Typed inner layer
        // deferred.
        encoding: PackEncoding::RawBytes,
    },
    PackTagSpec {
        id: 107,
        name: "vObjectSeries",
        // §10.2.2.31 — Series of VObject LS (V). Typed inner layer
        // deferred.
        encoding: PackEncoding::RawBytes,
    },
];

pub(crate) fn pack_lookup(tag: u8) -> Option<&'static PackTagSpec> {
    PACK_TAGS.iter().find(|t| t.id == tag)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum VTargetPackError {
    #[error("truncated BER-OID Target ID")]
    TruncatedTargetId,
    #[error("VTargetPack tag {tag}: truncated value")]
    TruncatedField { tag: u8 },
    #[error("VTargetPack tag {tag}: declared length {declared} exceeds available {available}")]
    LengthOverrun {
        tag: u8,
        declared: usize,
        available: usize,
    },
    #[error("VTargetPack tag {tag}: malformed IMAPB value")]
    MalformedImapb { tag: u8 },
    #[error("VTargetPack tag {tag}: malformed UTF-8 string")]
    MalformedUtf8 { tag: u8 },
    #[error("VTargetPack tag {tag}: invalid value length {got} (expected {expected})")]
    InvalidLength {
        tag: u8,
        expected: usize,
        got: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct VTargetPack {
    pub target_id: u32,
    pub centroid_pixel: Option<u32>,
    pub bbox_top_left_pixel: Option<u32>,
    pub bbox_bottom_right_pixel: Option<u32>,
    pub priority: Option<u8>,
    pub confidence_level: Option<u8>,
    pub history: Option<u16>,
    pub percentage_of_target_pixels: Option<u8>,
    pub target_color: Option<[u8; 3]>,
    pub target_intensity: Option<u32>,
    pub centroid_lat_offset: Option<f64>,
    pub centroid_lon_offset: Option<f64>,
    pub centroid_hae: Option<f64>,
    pub width_meters: Option<f64>,
    pub height_meters: Option<f64>,
    pub vmask: Option<Vec<u8>>,
    pub vobject: Option<Vec<u8>>,
    pub vfeature: Option<Vec<u8>>,
    pub vtracker: Option<Vec<u8>>,
    pub vchip: Option<Vec<u8>>,
    pub unknown: Vec<crate::klv::pack::OwnedRawField>,
    pub field_errors: Vec<crate::error::KlvFieldError>,
}

/// Decode a single VTargetPack from `bytes`. Returns the decoded pack
/// and the number of bytes consumed.
#[allow(dead_code, unused_variables)] // Task 4 wires the body
pub(crate) fn read_pack(bytes: &[u8]) -> Result<(VTargetPack, usize), VTargetPackError> {
    todo!("Task 4")
}

/// Encode a single VTargetPack into `out`. Returns bytes written.
#[allow(dead_code, unused_variables, clippy::ptr_arg)] // Task 4 wires the body
pub(crate) fn write_pack(
    pack: &VTargetPack,
    out: &mut Vec<u8>,
) -> Result<usize, crate::error::KlvEncodeError> {
    todo!("Task 4")
}

/// Number of bytes `pack` would occupy when encoded.
#[allow(dead_code, unused_variables)] // Task 4 wires the body
pub(crate) fn encoded_len(pack: &VTargetPack) -> usize {
    todo!("Task 4")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_tags_table_has_unique_ids() {
        let mut ids: Vec<u8> = PACK_TAGS.iter().map(|t| t.id).collect();
        ids.sort();
        let len_before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), len_before, "duplicate pack tag IDs");
    }

    #[test]
    fn pack_tags_lookup_round_trips() {
        for tag in PACK_TAGS {
            assert_eq!(pack_lookup(tag.id), Some(tag));
        }
        assert_eq!(pack_lookup(0), None);
        assert_eq!(pack_lookup(255), None);
        // Deprecated tags per ST 0903.6 §10.2.2.22, §10.2.2.26,
        // §10.2.2.27 — intentionally absent from the table; lenient
        // decoders must treat any wire occurrence as an unknown tag
        // (preserved per ST 0107.5 §6).
        assert_eq!(pack_lookup(21), None);
        assert_eq!(pack_lookup(102), None);
        assert_eq!(pack_lookup(103), None);
    }
}
