//! H.266 Profile/Tier/Level parser. Per H.266 V4 §7.3.3.

use crate::codec::ParseError;
use crate::codec::h265::bitreader::BitReader;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct H266ProfileTierLevel {
    /// `general_profile_idc` u(7) — H.266 V4 Annex A profile assignment.
    pub general_profile_idc: u8,
    /// `general_tier_flag` u(1) — 0 = Main tier, 1 = High tier.
    pub general_tier_flag: bool,
    /// `general_level_idc` u(8) — H.266 V4 Annex A.4 level table.
    pub general_level_idc: u8,
}

/// Parse an H.266 PTL syntax structure. Per H.266 V4 §7.3.3.
///
/// `profile_tier_present_flag` is the spec's outer guard (caller knows
/// from context — usually `true` for the first PTL in an SPS). When
/// `false` we skip the headline fields and return a default-zeroed PTL.
///
/// `max_num_sub_layers_minus1` controls how many sub-layer PTL records
/// follow; v0 reads but ignores them — only the headline fields matter
/// for metadata extraction.
pub fn parse_profile_tier_level(
    rbsp: &[u8],
    profile_tier_present_flag: bool,
    _max_num_sub_layers_minus1: u8,
) -> Result<H266ProfileTierLevel, ParseError> {
    let mut br = BitReader::new(rbsp);
    if !profile_tier_present_flag {
        return Ok(H266ProfileTierLevel::default());
    }
    let general_profile_idc = br.read_u(7)? as u8;
    let general_tier_flag = br.read_bool()?;
    let general_level_idc = br.read_u(8)? as u8;
    Ok(H266ProfileTierLevel {
        general_profile_idc,
        general_tier_flag,
        general_level_idc,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a constructed PTL with profile_idc=1 (Main 10), tier=0, level=63 (4.0).
    /// Layout: profile_idc(7) | tier(1) | level(8) | ptl_num_sub_profiles(8) | ...
    /// (The full PTL has many more fields; we only assert the headline three.)
    #[test]
    fn parse_ptl_main10_at_4_0() {
        // profile_idc=1 (0b0000_001), tier_flag=0 → byte0 = 0b0000_0010
        // level_idc=63 → byte1 = 0x3F
        // ptl_num_sub_profiles=0 → byte2 = 0x00
        let rbsp = vec![0x02, 0x3F, 0x00];
        let ptl = parse_profile_tier_level(&rbsp, /* profileTierPresentFlag */ true, /* MaxNumSubLayersMinus1 */ 0)
            .expect("PTL should parse");
        assert_eq!(ptl.general_profile_idc, 1);
        assert!(!ptl.general_tier_flag);
        assert_eq!(ptl.general_level_idc, 63);
    }
}
