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

/// MISB ST 0903.6 §10.2 Table 10 — typed VTargetPack.
///
/// The wire form is a leading BER-OID-encoded `targetId` (no Tag —
/// per §10.2.2.1 "has no Tag") followed by a Local Set–encoded body
/// using BER-OID tag + BER short/long length + value tuples.
///
/// `target_id` caps at `u32::MAX`. The spec allows the BER-OID Target
/// ID up to 9 wire bytes (~63-bit range, V9 in §10.2 Table 10). The
/// substrate's `read_ber_oid` returns `u32`, so this typed layer
/// inherits a u32 cap. Real-world streams stay well within u32.
///
/// V6 pixel-number fields (`centroid_pixel`, `bbox_top_left_pixel`,
/// `bbox_bottom_right_pixel`) are spec'd up to 6 wire bytes (~48-bit
/// range) but capped here at u32 — an 8K (33 MP) frame's pixel count
/// fits comfortably.
///
/// `detection_status` is the raw §10.2.2.24 / §7.2 codepoint:
/// 0=Active-Moving, 1=Active-Stopped, 2=Active-Coasting, 3=Inactive,
/// 4=Dropped. Typed enum deferred — stays as raw `u8`.
///
/// 7 nested/sibling Local Sets (`target_location`,
/// `geospatial_contour_series`, `vmask`, `vtracker`, `vchip`,
/// `vchip_series`, `vobject_series`) stay as `Option<Vec<u8>>`
/// pass-through bytes — typed inner layers deferred.
#[must_use]
#[derive(Debug, Clone, Default)]
pub struct VTargetPack {
    /// BER-OID `targetId` per §10.2.2.1. Capped at `u32::MAX` —
    /// see struct doc-comment for the spec-vs-substrate width
    /// mismatch rationale.
    pub target_id: u32,
    /// Tag 1 `targetCentroid` per §10.2.2.2 — pixel number, V6
    /// truncated big-endian.
    pub centroid_pixel: Option<u32>,
    /// Tag 2 `boundingBoxTopLeft` per §10.2.2.3 — pixel number, V6.
    pub bbox_top_left_pixel: Option<u32>,
    /// Tag 3 `boundingBoxBottomRight` per §10.2.2.4 — pixel number, V6.
    pub bbox_bottom_right_pixel: Option<u32>,
    /// Tag 4 `targetPriority` per §10.2.2.5 — fixed-length 1, valid 1..=255.
    pub priority: Option<u8>,
    /// Tag 5 `targetConfidenceLevel` per §10.2.2.6 — fixed-length 1,
    /// valid 0..=100 (percent).
    pub confidence_level: Option<u8>,
    /// Tag 6 `targetHistory` per §10.2.2.7 — V2 (frame count, 0..=65535).
    pub history: Option<u16>,
    /// Tag 7 `percentageOfTargetPixels` per §10.2.2.8 — fixed-length 1,
    /// valid 1..=100 (percent).
    pub percentage_of_target_pixels: Option<u8>,
    /// Tag 8 `targetColor` per §10.2.2.9 — fixed 3 bytes [R, G, B].
    pub target_color: Option<[u8; 3]>,
    /// Tag 9 `targetIntensity` per §10.2.2.10 — V3 (24-bit dynamic range).
    pub target_intensity: Option<u32>,
    /// Tag 10 `targetLocationOffsetLat` per §10.2.2.11 —
    /// IMAPB(-19.2°, 19.2°, 3 bytes).
    pub centroid_lat_offset: Option<f64>,
    /// Tag 11 `targetLocationOffsetLon` per §10.2.2.12 — IMAPB(-19.2°, 19.2°, 3).
    pub centroid_lon_offset: Option<f64>,
    /// Tag 12 `targetHae` per §10.2.2.13 — IMAPB(-900 m, 19000 m, 2).
    pub centroid_hae: Option<f64>,
    /// Tag 13 `boundingBoxTopLeftLatOffset` per §10.2.2.14 —
    /// IMAPB(-19.2°, 19.2°, 3).
    pub bbox_top_left_lat_offset: Option<f64>,
    /// Tag 14 `boundingBoxTopLeftLonOffset` per §10.2.2.15 — IMAPB(-19.2°, 19.2°, 3).
    pub bbox_top_left_lon_offset: Option<f64>,
    /// Tag 15 `boundingBoxBottomRightLatOffset` per §10.2.2.16 —
    /// IMAPB(-19.2°, 19.2°, 3).
    pub bbox_bottom_right_lat_offset: Option<f64>,
    /// Tag 16 `boundingBoxBottomRightLonOffset` per §10.2.2.17 —
    /// IMAPB(-19.2°, 19.2°, 3).
    pub bbox_bottom_right_lon_offset: Option<f64>,
    /// Tag 17 `targetLocation` per §10.2.2.18 — Defined Length
    /// Truncation Pack pass-through bytes (typed inner deferred).
    pub target_location: Option<Vec<u8>>,
    /// Tag 18 `geospatialContourSeries` per §10.2.2.19 — Series of
    /// Location pass-through bytes (typed inner deferred).
    pub geospatial_contour_series: Option<Vec<u8>>,
    /// Tag 19 `centroidPixRow` per §10.2.2.20 — V4 (1..=2^32-1).
    pub centroid_pix_row: Option<u32>,
    /// Tag 20 `centroidPixCol` per §10.2.2.21 — V4 (1..=2^32-1).
    pub centroid_pix_col: Option<u32>,
    /// Tag 22 `algorithmId` per §10.2.2.23 — V3 reference into
    /// the parent VMTI LS Algorithm Series.
    pub algorithm_id: Option<u32>,
    /// Tag 23 `detectionStatus` per §10.2.2.24 — fixed-length 1.
    /// See struct-level doc for the 5 spec codepoints.
    pub detection_status: Option<u8>,
    /// Tag 101 `vMask` per §10.2.2.25 — VMask LS pass-through bytes
    /// (typed inner deferred).
    pub vmask: Option<Vec<u8>>,
    /// Tag 104 `vTracker` per §10.2.2.28 — VTracker LS pass-through
    /// bytes (typed inner deferred).
    pub vtracker: Option<Vec<u8>>,
    /// Tag 105 `vChip` per §10.2.2.29 — VChip LS pass-through bytes
    /// (typed inner deferred).
    pub vchip: Option<Vec<u8>>,
    /// Tag 106 `vChipSeries` per §10.2.2.30 — Series of VChip LS
    /// pass-through bytes (typed inner deferred).
    pub vchip_series: Option<Vec<u8>>,
    /// Tag 107 `vObjectSeries` per §10.2.2.31 — Series of VObject
    /// LS pass-through bytes (typed inner deferred).
    pub vobject_series: Option<Vec<u8>>,
    /// Tags not in `PACK_TAGS` (deprecated 21/102/103, future
    /// additions). Preserved per ST 0107.5 §6 future-proof skip rule.
    pub unknown: Vec<crate::klv::pack::OwnedRawField>,
    /// Per-field validation errors (e.g. malformed IMAPB) accumulated
    /// during lenient decode. Mirrors the
    /// `klv::st0102::SecurityLs::field_errors` pattern.
    pub field_errors: Vec<crate::error::KlvFieldError>,
}

/// Manual `PartialEq` excluding [`VTargetPack::field_errors`]. Same
/// rationale as [`super::VmtiLs`]'s manual impl — `field_errors` is a
/// decode-side diagnostic, not part of the pack's semantic value.
/// Required for the ST 0903 round-trip fuzz target since `VTargetPack`
/// participates in `VmtiLs::targets`.
impl PartialEq for VTargetPack {
    fn eq(&self, other: &Self) -> bool {
        self.target_id == other.target_id
            && self.centroid_pixel == other.centroid_pixel
            && self.bbox_top_left_pixel == other.bbox_top_left_pixel
            && self.bbox_bottom_right_pixel == other.bbox_bottom_right_pixel
            && self.priority == other.priority
            && self.confidence_level == other.confidence_level
            && self.history == other.history
            && self.percentage_of_target_pixels == other.percentage_of_target_pixels
            && self.target_color == other.target_color
            && self.target_intensity == other.target_intensity
            && self.centroid_lat_offset == other.centroid_lat_offset
            && self.centroid_lon_offset == other.centroid_lon_offset
            && self.centroid_hae == other.centroid_hae
            && self.bbox_top_left_lat_offset == other.bbox_top_left_lat_offset
            && self.bbox_top_left_lon_offset == other.bbox_top_left_lon_offset
            && self.bbox_bottom_right_lat_offset == other.bbox_bottom_right_lat_offset
            && self.bbox_bottom_right_lon_offset == other.bbox_bottom_right_lon_offset
            && self.target_location == other.target_location
            && self.geospatial_contour_series == other.geospatial_contour_series
            && self.centroid_pix_row == other.centroid_pix_row
            && self.centroid_pix_col == other.centroid_pix_col
            && self.algorithm_id == other.algorithm_id
            && self.detection_status == other.detection_status
            && self.vmask == other.vmask
            && self.vtracker == other.vtracker
            && self.vchip == other.vchip
            && self.vchip_series == other.vchip_series
            && self.vobject_series == other.vobject_series
            && self.unknown == other.unknown
    }
}

/// Decode a single VTargetPack from `bytes`. Returns the decoded pack
/// and the number of bytes consumed.
///
/// Wire form per ST 0903.6 §10.2 Table 10:
/// - Leading BER-OID `targetId` (no Tag, per §10.2.2.1).
/// - Then a Local Set–encoded body where each field is
///   `[1-byte tag][BER short/long length][value bytes]`.
///
/// Unknown / deprecated tags (e.g. 21, 102, 103) are preserved in
/// `pack.unknown` per ST 0107.5 §6 future-proof skip rule.
pub(crate) fn read_pack(bytes: &[u8]) -> Result<(VTargetPack, usize), VTargetPackError> {
    use crate::klv::length::{read_ber, read_ber_oid};

    // 1. Read the leading BER-OID Target ID.
    let (target_id, rest) = read_ber_oid(bytes).map_err(|_| VTargetPackError::TruncatedTargetId)?;
    let header_consumed = bytes.len() - rest.len();

    let mut pack = VTargetPack {
        target_id,
        ..Default::default()
    };

    // 2. Walk the LS-encoded body. Each field is a single-byte tag
    //    (PACK_TAGS only uses 1..=107) + BER-encoded length + value.
    let mut cursor = rest;
    let mut consumed = header_consumed;
    while !cursor.is_empty() {
        let tag = cursor[0];
        cursor = &cursor[1..];
        consumed += 1;

        let (declared_len, after_len) =
            read_ber(cursor).map_err(|_| VTargetPackError::TruncatedField { tag })?;
        let len_consumed = cursor.len() - after_len.len();
        cursor = after_len;
        consumed += len_consumed;

        if cursor.len() < declared_len {
            return Err(VTargetPackError::LengthOverrun {
                tag,
                declared: declared_len,
                available: cursor.len(),
            });
        }
        let value = &cursor[..declared_len];
        cursor = &cursor[declared_len..];
        consumed += declared_len;

        decode_field(tag, value, &mut pack)?;
    }

    Ok((pack, consumed))
}

/// Dispatch a single TLV field's value bytes to the matching
/// `VTargetPack` field based on the spec's encoding for that tag.
/// Unknown / deprecated tags fall through to `pack.unknown` per
/// ST 0107.5 §6.
fn decode_field(tag: u8, value: &[u8], pack: &mut VTargetPack) -> Result<(), VTargetPackError> {
    use super::var_uint::read_var_u32;
    use crate::klv::imapb::{ImapbParams, decode_imapb};
    use crate::klv::pack::OwnedRawField;

    let Some(spec) = pack_lookup(tag) else {
        // ST 0107.5 §6 skip rule — preserve unknown / deprecated tags.
        pack.unknown.push(OwnedRawField {
            tag: tag as u32,
            value: value.to_vec(),
        });
        return Ok(());
    };

    match spec.encoding {
        PackEncoding::U8 => {
            if value.len() != 1 {
                return Err(VTargetPackError::InvalidLength {
                    tag,
                    expected: 1,
                    got: value.len(),
                });
            }
            let v = value[0];
            match tag {
                4 => pack.priority = Some(v),
                5 => pack.confidence_level = Some(v),
                7 => pack.percentage_of_target_pixels = Some(v),
                23 => pack.detection_status = Some(v),
                _ => unreachable!("U8 dispatch missing tag {tag}"),
            }
        }
        PackEncoding::VarUint { max_bytes } => {
            if value.is_empty() || value.len() > max_bytes as usize {
                return Err(VTargetPackError::InvalidLength {
                    tag,
                    expected: max_bytes as usize,
                    got: value.len(),
                });
            }
            // VarUint codec returns u32; per-tag downcasts handled below.
            let v = read_var_u32(value).map_err(|_| VTargetPackError::TruncatedField { tag })?;
            match tag {
                1 => pack.centroid_pixel = Some(v),
                2 => pack.bbox_top_left_pixel = Some(v),
                3 => pack.bbox_bottom_right_pixel = Some(v),
                6 => pack.history = Some(v as u16), // V2 caps at u16
                9 => pack.target_intensity = Some(v),
                19 => pack.centroid_pix_row = Some(v),
                20 => pack.centroid_pix_col = Some(v),
                22 => pack.algorithm_id = Some(v),
                _ => unreachable!("VarUint dispatch missing tag {tag}"),
            }
        }
        PackEncoding::U24Rgb => {
            if value.len() != 3 {
                return Err(VTargetPackError::InvalidLength {
                    tag,
                    expected: 3,
                    got: value.len(),
                });
            }
            pack.target_color = Some([value[0], value[1], value[2]]);
        }
        PackEncoding::ImapbF64 { min, max } => {
            // Tag 12 (`targetHae`) uses 2-byte IMAPB; all other IMAPB
            // pack tags use 3-byte IMAPB per §10.2.2.11–.17.
            let length = if tag == 12 { 2 } else { 3 };
            let params = ImapbParams { min, max, length };
            let v = decode_imapb(&params, value)
                .map_err(|_| VTargetPackError::MalformedImapb { tag })?;
            match tag {
                10 => pack.centroid_lat_offset = Some(v),
                11 => pack.centroid_lon_offset = Some(v),
                12 => pack.centroid_hae = Some(v),
                13 => pack.bbox_top_left_lat_offset = Some(v),
                14 => pack.bbox_top_left_lon_offset = Some(v),
                15 => pack.bbox_bottom_right_lat_offset = Some(v),
                16 => pack.bbox_bottom_right_lon_offset = Some(v),
                _ => unreachable!("ImapbF64 dispatch missing tag {tag}"),
            }
        }
        PackEncoding::RawBytes => {
            let bytes = value.to_vec();
            match tag {
                17 => pack.target_location = Some(bytes),
                18 => pack.geospatial_contour_series = Some(bytes),
                101 => pack.vmask = Some(bytes),
                104 => pack.vtracker = Some(bytes),
                105 => pack.vchip = Some(bytes),
                106 => pack.vchip_series = Some(bytes),
                107 => pack.vobject_series = Some(bytes),
                _ => unreachable!("RawBytes dispatch missing tag {tag}"),
            }
        }
    }
    Ok(())
}

/// Encode a single VTargetPack into `out`. Returns bytes written.
///
/// Fields are emitted in ascending tag order (1, 2, 3, ..., 23, 101,
/// 104, 105, 106, 107), then any preserved `unknown` tags last per
/// ST 0107.5 §6.
pub(crate) fn write_pack(
    pack: &VTargetPack,
    out: &mut Vec<u8>,
) -> Result<usize, crate::error::KlvEncodeError> {
    use super::emit::{emit_imapb_n, emit_tlv, emit_var};
    use crate::klv::length::write_ber_oid;

    let start = out.len();

    // 1. BER-OID Target ID (5 bytes covers up to u32::MAX).
    let mut buf = [0u8; 5];
    let n = write_ber_oid(pack.target_id, &mut buf)?;
    out.extend_from_slice(&buf[..n]);

    if let Some(v) = pack.centroid_pixel {
        emit_var(out, 1, v)?;
    }
    if let Some(v) = pack.bbox_top_left_pixel {
        emit_var(out, 2, v)?;
    }
    if let Some(v) = pack.bbox_bottom_right_pixel {
        emit_var(out, 3, v)?;
    }
    if let Some(v) = pack.priority {
        emit_tlv(out, 4, &[v])?;
    }
    if let Some(v) = pack.confidence_level {
        emit_tlv(out, 5, &[v])?;
    }
    if let Some(v) = pack.history {
        emit_var(out, 6, v as u32)?;
    }
    if let Some(v) = pack.percentage_of_target_pixels {
        emit_tlv(out, 7, &[v])?;
    }
    if let Some(v) = pack.target_color {
        emit_tlv(out, 8, &v)?;
    }
    if let Some(v) = pack.target_intensity {
        emit_var(out, 9, v)?;
    }

    // IMAPB fields. Tags 10/11/13/14/15/16 use 3-byte IMAPB per
    // §10.2.2.11/.12/.14/.15/.16/.17 over [-19.2°, 19.2°]. Tag 12
    // uses 2-byte IMAPB per §10.2.2.13 over [-900 m, 19000 m].
    if let Some(v) = pack.centroid_lat_offset {
        emit_imapb_n(out, 10, v, -19.2, 19.2, 3)?;
    }
    if let Some(v) = pack.centroid_lon_offset {
        emit_imapb_n(out, 11, v, -19.2, 19.2, 3)?;
    }
    if let Some(v) = pack.centroid_hae {
        emit_imapb_n(out, 12, v, -900.0, 19000.0, 2)?;
    }
    if let Some(v) = pack.bbox_top_left_lat_offset {
        emit_imapb_n(out, 13, v, -19.2, 19.2, 3)?;
    }
    if let Some(v) = pack.bbox_top_left_lon_offset {
        emit_imapb_n(out, 14, v, -19.2, 19.2, 3)?;
    }
    if let Some(v) = pack.bbox_bottom_right_lat_offset {
        emit_imapb_n(out, 15, v, -19.2, 19.2, 3)?;
    }
    if let Some(v) = pack.bbox_bottom_right_lon_offset {
        emit_imapb_n(out, 16, v, -19.2, 19.2, 3)?;
    }

    if let Some(ref bytes) = pack.target_location {
        emit_tlv(out, 17, bytes)?;
    }
    if let Some(ref bytes) = pack.geospatial_contour_series {
        emit_tlv(out, 18, bytes)?;
    }
    if let Some(v) = pack.centroid_pix_row {
        emit_var(out, 19, v)?;
    }
    if let Some(v) = pack.centroid_pix_col {
        emit_var(out, 20, v)?;
    }
    if let Some(v) = pack.algorithm_id {
        emit_var(out, 22, v)?;
    }
    if let Some(v) = pack.detection_status {
        emit_tlv(out, 23, &[v])?;
    }
    if let Some(ref bytes) = pack.vmask {
        emit_tlv(out, 101, bytes)?;
    }
    if let Some(ref bytes) = pack.vtracker {
        emit_tlv(out, 104, bytes)?;
    }
    if let Some(ref bytes) = pack.vchip {
        emit_tlv(out, 105, bytes)?;
    }
    if let Some(ref bytes) = pack.vchip_series {
        emit_tlv(out, 106, bytes)?;
    }
    if let Some(ref bytes) = pack.vobject_series {
        emit_tlv(out, 107, bytes)?;
    }

    // Unknown tags preserved last (ST 0107.5 §6). Tag IDs >0xFF are
    // silently dropped — VTargetPack tag IDs are single-byte by spec
    // (highest is 107) so a >0xFF tag here would be a corrupted parse.
    for field in &pack.unknown {
        if field.tag <= 0xFF {
            emit_tlv(out, field.tag as u8, &field.value)?;
        }
    }

    Ok(out.len() - start)
}

/// Number of bytes `pack` would occupy when encoded. Mirrors
/// `write_pack`'s field-by-field structure.
pub(crate) fn encoded_len(pack: &VTargetPack) -> usize {
    use super::var_uint::var_u32_len;
    use crate::klv::length::{ber_len, ber_oid_len};

    fn tlv_len(value_len: usize) -> usize {
        1 /* tag */ + ber_len(value_len) + value_len
    }

    let mut total = ber_oid_len(pack.target_id);
    if let Some(v) = pack.centroid_pixel {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(v) = pack.bbox_top_left_pixel {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(v) = pack.bbox_bottom_right_pixel {
        total += tlv_len(var_u32_len(v));
    }
    if pack.priority.is_some() {
        total += tlv_len(1);
    }
    if pack.confidence_level.is_some() {
        total += tlv_len(1);
    }
    if let Some(v) = pack.history {
        total += tlv_len(var_u32_len(v as u32));
    }
    if pack.percentage_of_target_pixels.is_some() {
        total += tlv_len(1);
    }
    if pack.target_color.is_some() {
        total += tlv_len(3);
    }
    if let Some(v) = pack.target_intensity {
        total += tlv_len(var_u32_len(v));
    }
    if pack.centroid_lat_offset.is_some() {
        total += tlv_len(3);
    }
    if pack.centroid_lon_offset.is_some() {
        total += tlv_len(3);
    }
    if pack.centroid_hae.is_some() {
        total += tlv_len(2);
    }
    if pack.bbox_top_left_lat_offset.is_some() {
        total += tlv_len(3);
    }
    if pack.bbox_top_left_lon_offset.is_some() {
        total += tlv_len(3);
    }
    if pack.bbox_bottom_right_lat_offset.is_some() {
        total += tlv_len(3);
    }
    if pack.bbox_bottom_right_lon_offset.is_some() {
        total += tlv_len(3);
    }
    if let Some(ref b) = pack.target_location {
        total += tlv_len(b.len());
    }
    if let Some(ref b) = pack.geospatial_contour_series {
        total += tlv_len(b.len());
    }
    if let Some(v) = pack.centroid_pix_row {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(v) = pack.centroid_pix_col {
        total += tlv_len(var_u32_len(v));
    }
    if let Some(v) = pack.algorithm_id {
        total += tlv_len(var_u32_len(v));
    }
    if pack.detection_status.is_some() {
        total += tlv_len(1);
    }
    if let Some(ref b) = pack.vmask {
        total += tlv_len(b.len());
    }
    if let Some(ref b) = pack.vtracker {
        total += tlv_len(b.len());
    }
    if let Some(ref b) = pack.vchip {
        total += tlv_len(b.len());
    }
    if let Some(ref b) = pack.vchip_series {
        total += tlv_len(b.len());
    }
    if let Some(ref b) = pack.vobject_series {
        total += tlv_len(b.len());
    }
    for field in &pack.unknown {
        if field.tag <= 0xFF {
            total += tlv_len(field.value.len());
        }
    }
    total
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

    #[test]
    fn empty_pack_round_trips() {
        let pack = VTargetPack {
            target_id: 1,
            ..Default::default()
        };
        let mut bytes = Vec::new();
        let written = write_pack(&pack, &mut bytes).unwrap();
        assert_eq!(written, bytes.len());
        assert_eq!(written, encoded_len(&pack));
        let (decoded, consumed) = read_pack(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded.target_id, 1);
        // Default fields should round-trip as None / empty.
        assert!(decoded.centroid_pixel.is_none());
        assert!(decoded.target_color.is_none());
        assert!(decoded.unknown.is_empty());
        assert!(decoded.field_errors.is_empty());
    }

    #[test]
    fn populated_pack_round_trips() {
        let pack = VTargetPack {
            target_id: 42,
            centroid_pixel: Some(8_294_400),
            bbox_top_left_pixel: Some(8_293_000),
            bbox_bottom_right_pixel: Some(8_295_800),
            priority: Some(1),
            confidence_level: Some(95),
            history: Some(0),
            percentage_of_target_pixels: Some(60),
            target_color: Some([0xFF, 0x80, 0x40]),
            target_intensity: Some(220),
            centroid_lat_offset: Some(0.001234),
            centroid_lon_offset: Some(-0.005678),
            centroid_hae: Some(150.0),
            bbox_top_left_lat_offset: Some(0.000123),
            bbox_top_left_lon_offset: Some(-0.000456),
            bbox_bottom_right_lat_offset: Some(0.000789),
            bbox_bottom_right_lon_offset: Some(-0.001234),
            target_location: Some(vec![0xAA, 0xBB]),
            geospatial_contour_series: Some(vec![0xCC, 0xDD]),
            centroid_pix_row: Some(1080),
            centroid_pix_col: Some(1920),
            algorithm_id: Some(7),
            detection_status: Some(1),
            vmask: Some(vec![0xDE, 0xAD]),
            vtracker: Some(vec![0x42]),
            vchip: None,
            vchip_series: Some(vec![0x01, 0x02]),
            vobject_series: Some(vec![0x03, 0x04]),
            unknown: vec![],
            field_errors: vec![],
        };
        let bytes = {
            let mut b = Vec::new();
            write_pack(&pack, &mut b).unwrap();
            b
        };
        let (decoded, _consumed) = read_pack(&bytes).unwrap();
        assert_eq!(decoded.target_id, 42);
        assert_eq!(decoded.centroid_pixel, Some(8_294_400));
        assert_eq!(decoded.priority, Some(1));
        assert_eq!(decoded.target_color, Some([0xFF, 0x80, 0x40]));
        assert_eq!(decoded.vmask, Some(vec![0xDE, 0xAD]));
        assert_eq!(decoded.detection_status, Some(1));
        assert_eq!(decoded.algorithm_id, Some(7));
        assert!((decoded.centroid_lat_offset.unwrap() - 0.001234).abs() < 1e-5);
        assert!((decoded.centroid_lon_offset.unwrap() - (-0.005678)).abs() < 1e-5);
        assert!((decoded.bbox_top_left_lat_offset.unwrap() - 0.000123).abs() < 1e-5);
        assert!((decoded.bbox_bottom_right_lon_offset.unwrap() - (-0.001234)).abs() < 1e-5);
    }

    #[test]
    fn target_id_multibyte_round_trips() {
        // Target ID = 200 fits in 2 BER-OID bytes (0x81 0x48).
        let pack = VTargetPack {
            target_id: 200,
            ..Default::default()
        };
        let mut bytes = Vec::new();
        write_pack(&pack, &mut bytes).unwrap();
        assert_eq!(bytes[0], 0x81);
        assert_eq!(bytes[1], 0x48);
        let (decoded, _) = read_pack(&bytes).unwrap();
        assert_eq!(decoded.target_id, 200);
    }

    #[test]
    fn truncated_target_id_rejected() {
        // 0x81 alone signals "more bytes follow" but buffer is empty.
        let bytes = [0x81u8];
        let err = read_pack(&bytes).unwrap_err();
        assert!(matches!(err, VTargetPackError::TruncatedTargetId));
    }

    #[test]
    fn truncated_field_value_rejected() {
        // Target ID = 1 (1 byte 0x01), then tag 6 (Target History)
        // declares length 2 but provides 1 byte.
        let bytes = [0x01, 6, 2, 0xFF];
        let err = read_pack(&bytes).unwrap_err();
        assert!(matches!(
            err,
            VTargetPackError::LengthOverrun { tag: 6, .. }
        ));
    }

    #[test]
    fn unknown_tags_preserved() {
        // Build by hand: target_id=1, then unknown tag 200 with 3 bytes
        // [0xAA 0xBB 0xCC].
        let bytes = [0x01u8, 200, 3, 0xAA, 0xBB, 0xCC];
        let (decoded, _) = read_pack(&bytes).unwrap();
        assert_eq!(decoded.target_id, 1);
        assert_eq!(decoded.unknown.len(), 1);
        assert_eq!(decoded.unknown[0].tag, 200);
        assert_eq!(decoded.unknown[0].value, vec![0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn deprecated_tag_preserved_as_unknown() {
        // Per ST 0107.5 §6, deprecated tag IDs (e.g., 21) should round-
        // trip as unknown bytes — the decoder doesn't reject them, just
        // treats them as opaque.
        let bytes = [0x01u8, 21, 2, 0xDE, 0xAD];
        let (decoded, _) = read_pack(&bytes).unwrap();
        assert_eq!(decoded.unknown.len(), 1);
        assert_eq!(decoded.unknown[0].tag, 21);
        assert_eq!(decoded.unknown[0].value, vec![0xDE, 0xAD]);
    }

    /// Locks in the canonical wire format of a known VTargetPack.
    /// Catches accidental field-order changes in `write_pack` (which
    /// round-trip tests miss because `read_pack` is order-agnostic) and
    /// catches `encoded_len` drift relative to `write_pack`.
    #[test]
    fn write_pack_canonical_byte_layout() {
        let pack = VTargetPack {
            target_id: 7,
            centroid_pixel: Some(0x1234), // Tag 1, V6 → 2 bytes [0x12, 0x34]
            priority: Some(2),            // Tag 4, U8
            confidence_level: Some(95),   // Tag 5, U8
            target_color: Some([0xAA, 0xBB, 0xCC]), // Tag 8, 3-byte RGB
            detection_status: Some(1),    // Tag 23, U8
            vmask: Some(vec![0xDE, 0xAD]), // Tag 101
            ..Default::default()
        };

        let mut bytes = Vec::new();
        let written = write_pack(&pack, &mut bytes).unwrap();

        // Expected wire form (BER-OID Target ID + ascending-tag TLVs):
        let expected: Vec<u8> = vec![
            0x07, // BER-OID Target ID = 7
            // Tag 1, len 2, value [0x12, 0x34] (centroid_pixel)
            0x01, 0x02, 0x12, 0x34, // Tag 4, len 1, value [0x02] (priority)
            0x04, 0x01, 0x02,
            // Tag 5, len 1, value [0x5F] (confidence_level = 95 = 0x5F)
            0x05, 0x01, 0x5F, // Tag 8, len 3, value [0xAA, 0xBB, 0xCC] (target_color)
            0x08, 0x03, 0xAA, 0xBB, 0xCC,
            // Tag 23, len 1, value [0x01] (detection_status)
            0x17, 0x01, 0x01, // Tag 101, len 2, value [0xDE, 0xAD] (vmask)
            0x65, 0x02, 0xDE, 0xAD,
        ];
        assert_eq!(
            bytes, expected,
            "write_pack produced unexpected byte layout — \
            either field-order changed or a TLV got bogus bytes"
        );

        assert_eq!(written, bytes.len());
        assert_eq!(
            written,
            encoded_len(&pack),
            "write_pack length disagrees with encoded_len — drift between the two functions"
        );
    }
}
