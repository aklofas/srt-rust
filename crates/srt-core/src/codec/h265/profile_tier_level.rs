// Dead-code lint suppressed: this is a private substrate module; callers
// arrive in subsequent tasks (VPS/SPS parsers).
#![allow(dead_code)]

//! `profile_tier_level()` parser per H.265 §7.3.3.
//!
//! Decoded fields used by VPS/SPS callers:
//! - `general_profile_space` (2 bits)
//! - `general_tier_flag` (1 bit)
//! - `general_profile_idc` (5 bits)
//! - `general_profile_compatibility_flags` (32 bits)
//! - `general_level_idc` (8 bits)
//!
//! Per-sub-layer fields (when `maxNumSubLayersMinus1 > 0`) are skipped.

use super::bitreader::BitReader;
use crate::codec::ParseError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ProfileTierLevel {
    pub general_profile_space: u8,
    pub general_tier_flag: bool,
    pub general_profile_idc: u8,
    pub general_profile_compatibility_flags: u32,
    pub general_level_idc: u8,
}

pub(crate) fn parse(
    br: &mut BitReader<'_>,
    max_num_sub_layers_minus1: u8,
) -> Result<ProfileTierLevel, ParseError> {
    let general_profile_space = br.read_u(2)? as u8;
    let general_tier_flag = br.read_bool()?;
    let general_profile_idc = br.read_u(5)? as u8;
    let general_profile_compatibility_flags = br.read_u(32)?;
    // 6 general_*_constraint_flag bits + 1 general_inbld_flag + 41 reserved = 48 bits.
    // Actually per the spec it's broken up; we only need the level so we skip 48 bits.
    br.skip(48)?;
    let general_level_idc = br.read_u(8)? as u8;

    // Per sub-layer: read sub_layer_profile_present_flag and sub_layer_level_present_flag
    // for each layer i in [0..max_num_sub_layers_minus1).
    let mut sub_layer_profile_present = [false; 8];
    let mut sub_layer_level_present = [false; 8];
    for i in 0..max_num_sub_layers_minus1 as usize {
        sub_layer_profile_present[i] = br.read_bool()?;
        sub_layer_level_present[i] = br.read_bool()?;
    }
    if max_num_sub_layers_minus1 > 0 {
        // Reserved zero bits to byte-align (2 bits per missing sub-layer
        // up to 8 sub-layers total).
        for _ in max_num_sub_layers_minus1..8 {
            br.skip(2)?;
        }
    }
    // Skip per-sub-layer profile/level info we don't expose.
    for i in 0..max_num_sub_layers_minus1 as usize {
        if sub_layer_profile_present[i] {
            br.skip(2 + 1 + 5 + 32 + 48)?;
        }
        if sub_layer_level_present[i] {
            br.skip(8)?;
        }
    }

    Ok(ProfileTierLevel {
        general_profile_space,
        general_tier_flag,
        general_profile_idc,
        general_profile_compatibility_flags,
        general_level_idc,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_no_sub_layers() {
        // Byte 0: profile_space=0b00, tier_flag=1, profile_idc=0b00001 → 0b0010_0001
        let buf: [u8; 12] = [
            0b0010_0001,
            0x60, 0x00, 0x00, 0x00,
            0, 0, 0, 0, 0, 0,
            120,
        ];
        let mut br = BitReader::new(&buf);
        let ptl = parse(&mut br, 0).unwrap();
        assert!(ptl.general_tier_flag);
        assert_eq!(ptl.general_profile_idc, 1);
        assert_eq!(ptl.general_profile_compatibility_flags, 0x60000000);
        assert_eq!(ptl.general_level_idc, 120);
    }
}
