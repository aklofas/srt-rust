//! VPS parser per H.265 §7.3.2.1. Only the fields exposed on
//! [`H265Vps`] are decoded; everything past `general_level_idc` is
//! skipped.

use super::profile_tier_level;
use crate::codec::CodecParseError;
use crate::codec::bitreader::BitReader;

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H265Vps {
    pub vps_video_parameter_set_id: u8,
    pub max_layers_minus1: u8,
    pub max_sub_layers_minus1: u8,
    pub temporal_id_nesting_flag: bool,
    pub general_profile_idc: u8,
    pub general_tier_flag: bool,
    pub general_level_idc: u8,
    /// 32-bit `general_profile_compatibility_flags` per H.265 §7.3.3 — bit
    /// `i` set means the stream conforms to profile `i`. See [`H265Sps`]
    /// for the disambiguation rationale (Main vs Main10 vs Main10-Intra).
    ///
    /// [`H265Sps`]: crate::codec::h265::H265Sps
    pub general_profile_compatibility_flags: u32,
    /// `general_progressive_source_flag` (§7.4.4).
    pub general_progressive_source_flag: bool,
    /// `general_interlaced_source_flag` (§7.4.4).
    pub general_interlaced_source_flag: bool,
    /// `general_non_packed_constraint_flag` (§7.4.4).
    pub general_non_packed_constraint_flag: bool,
    /// `general_frame_only_constraint_flag` (§7.4.4).
    pub general_frame_only_constraint_flag: bool,
    pub raw_rbsp: Vec<u8>,
}

/// Parse a single VPS RBSP. Input contract: RBSP body only (no NAL
/// header bytes, no Annex B start code, emulation prevention preserved).
pub fn parse_vps(rbsp: &[u8]) -> Result<H265Vps, CodecParseError> {
    if rbsp.is_empty() {
        return Err(CodecParseError::TruncatedRbsp {
            offset_bits: 0,
            needed_bits: 8,
        });
    }
    let mut br = BitReader::new(rbsp);
    let vps_video_parameter_set_id = br.read_u(4)? as u8;
    let _vps_base_layer_internal_flag = br.read_bool()?;
    let _vps_base_layer_available_flag = br.read_bool()?;
    let max_layers_minus1 = br.read_u(6)? as u8;
    let max_sub_layers_minus1 = br.read_u(3)? as u8;
    let temporal_id_nesting_flag = br.read_bool()?;
    let _vps_reserved_0xffff_16bits = br.read_u(16)?;

    let ptl = profile_tier_level::parse(&mut br, max_sub_layers_minus1)?;

    Ok(H265Vps {
        vps_video_parameter_set_id,
        max_layers_minus1,
        max_sub_layers_minus1,
        temporal_id_nesting_flag,
        general_profile_idc: ptl.general_profile_idc,
        general_tier_flag: ptl.general_tier_flag,
        general_level_idc: ptl.general_level_idc,
        general_profile_compatibility_flags: ptl.general_profile_compatibility_flags,
        general_progressive_source_flag: ptl.general_progressive_source_flag,
        general_interlaced_source_flag: ptl.general_interlaced_source_flag,
        general_non_packed_constraint_flag: ptl.general_non_packed_constraint_flag,
        general_frame_only_constraint_flag: ptl.general_frame_only_constraint_flag,
        raw_rbsp: rbsp.to_vec(),
    })
}
