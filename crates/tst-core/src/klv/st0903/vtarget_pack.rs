//! ST 0903.6 §6 Table 2 + §6.4 — VTargetPack typed layer.
//!
//! A VTargetPack is a BER-OID-prefixed ordered Pack: the first 1..=5
//! bytes are the Target ID encoded as BER-OID; the remaining bytes are
//! a Local Set–encoded body (BER-tag + BER-length + value). Each
//! VTargetPack is itself prefixed with a BER outer-length when
//! serialized inside a VTargetSeries (Tag 101) — that outer length is
//! consumed by the series walker, not by `read_pack` / `write_pack`.

use thiserror::Error;

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
