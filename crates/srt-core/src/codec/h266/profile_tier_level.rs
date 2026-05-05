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

/// Parse an H.266 PTL syntax structure standalone. Per H.266 V4 §7.3.3.
///
/// `profile_tier_present_flag` is the spec's outer guard (caller knows
/// from context — usually `true` for the first PTL in an SPS). When
/// `false` we skip the headline fields and return a default-zeroed PTL.
///
/// `max_num_sub_layers_minus1` controls how many sub-layer PTL records
/// follow; v0 reads but ignores them — only the headline fields matter
/// for metadata extraction.
///
/// This standalone variant is for callers that only have the PTL bytes
/// (e.g., tests, future external parameter-set walkers). Sub-parsers
/// embedded in SPS/VPS use [`parse_into`] instead, which shares the
/// caller's bit cursor so subsequent fields stay aligned.
pub fn parse_profile_tier_level(
    rbsp: &[u8],
    profile_tier_present_flag: bool,
    max_num_sub_layers_minus1: u8,
) -> Result<H266ProfileTierLevel, ParseError> {
    let mut br = BitReader::new(rbsp);
    let mut out = H266ProfileTierLevel::default();
    parse_into(
        &mut br,
        profile_tier_present_flag,
        max_num_sub_layers_minus1,
        &mut out,
    )?;
    Ok(out)
}

/// Walk the full PTL syntax via a caller-supplied bit cursor.
///
/// Per H.266 V4 §7.3.3.1. Surfaces only the headline three fields into
/// `out`; everything after (constraint info, sublayer level flags,
/// alignment padding, sub-profile loop) is walked-and-discarded so the
/// cursor lands at the next SPS/VPS field correctly.
///
/// Pre-condition: `out` is already default-initialized; this function
/// overwrites the headline fields when `profile_tier_present_flag` is
/// true and leaves them at their default-zero values otherwise.
pub(crate) fn parse_into(
    br: &mut BitReader<'_>,
    profile_tier_present_flag: bool,
    max_num_sub_layers_minus1: u8,
    out: &mut H266ProfileTierLevel,
) -> Result<(), ParseError> {
    if profile_tier_present_flag {
        out.general_profile_idc = br.read_u(7)? as u8;
        out.general_tier_flag = br.read_bool()?;
    }
    out.general_level_idc = br.read_u(8)? as u8;
    let _ptl_frame_only_constraint_flag = br.read_bool()?;
    let _ptl_multilayer_enabled_flag = br.read_bool()?;
    if profile_tier_present_flag {
        // §7.3.3.2 general_constraints_info() — when gci_present_flag=0,
        // the body is just the flag bit; otherwise a long fixed-shape
        // header (~71 bits) plus a variable additional-bits tail. We
        // walk both shapes so the cursor lands correctly on either.
        parse_general_constraints_info(br)?;
    }
    // Sublayer ptl_sublayer_level_present_flag[i] — i runs from
    // MaxNumSubLayersMinus1-1 down to 0, so for max=0 the loop is empty.
    let mut sublayer_present = [false; 7];
    if max_num_sub_layers_minus1 > 0 {
        for i in (0..max_num_sub_layers_minus1).rev() {
            sublayer_present[i as usize] = br.read_bool()?;
        }
    }
    // Byte-align via ptl_reserved_zero_bit u(1) padding.
    while br.position() % 8 != 0 {
        let _ = br.read_bool()?;
    }
    // sublayer_level_idc[i] u(8) for any present sublayer (loop also
    // empty when max_num_sub_layers_minus1 == 0).
    if max_num_sub_layers_minus1 > 0 {
        for i in (0..max_num_sub_layers_minus1).rev() {
            if sublayer_present[i as usize] {
                let _sublayer_level_idc = br.read_u(8)?;
            }
        }
    }
    if profile_tier_present_flag {
        let ptl_num_sub_profiles = br.read_u(8)?;
        for _ in 0..ptl_num_sub_profiles {
            let _general_sub_profile_idc = br.read_u(32)?;
        }
    }
    Ok(())
}

/// §7.3.3.2 general_constraints_info(). Walk-and-discard.
fn parse_general_constraints_info(br: &mut BitReader<'_>) -> Result<(), ParseError> {
    let gci_present_flag = br.read_bool()?;
    if gci_present_flag {
        // Fixed-shape body — count by section per H.266 V4 §7.3.3.2.
        // - general:        3 bits (intra_only, all_layers_independent, one_au_only)
        // - picture format: 6 bits (sixteen_minus_max_bitdepth u(4) + three_minus_max_chroma_format u(2))
        // - NAL related:    11 bits
        // - tile/slice:     6 bits
        // - CTU/block:      5 bits (three_minus_max_log2_ctu u(2) + three flags)
        // - intra:          6 bits
        // - inter:          14 bits
        // - transform/qp:   13 bits
        // - loop filter:    6 bits
        // = 70 fixed bits, then gci_num_additional_bits u(8) = 78 bits.
        br.skip(3)?;
        br.skip(4 + 2)?;
        br.skip(11)?;
        br.skip(6)?;
        br.skip(2 + 3)?;
        br.skip(6)?;
        br.skip(14)?;
        br.skip(13)?;
        br.skip(6)?;
        let gci_num_additional_bits = br.read_u(8)?;
        // First 6 of additional bits are named flags; remaining are
        // reserved bits — both walked the same.
        br.skip(gci_num_additional_bits)?;
    }
    // Byte-align via gci_alignment_zero_bit f(1) padding.
    while br.position() % 8 != 0 {
        let _ = br.read_bool()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a constructed PTL with profile_idc=1 (Main 10), tier=0, level=63 (4.0).
    /// MaxNumSubLayersMinus1=0, gci_present_flag=0, ptl_num_sub_profiles=0.
    /// Layout: profile_idc(7) | tier(1) | level(8) | frame_only(1) | multilayer(1) |
    ///         gci_present(1) | align(5) | num_sub_profiles(8) = 32 bits = 4 bytes.
    #[test]
    fn parse_ptl_main10_at_4_0() {
        // Byte 0: profile_idc=1 (0b0000001) | tier=0 → 0b00000010 = 0x02
        // Byte 1: level=63 → 0x3F
        // Byte 2: frame_only=0 | multilayer=0 | gci_present=0 | 5 align zeros → 0x00
        // Byte 3: num_sub_profiles=0 → 0x00
        let rbsp = vec![0x02, 0x3F, 0x00, 0x00];
        let ptl = parse_profile_tier_level(&rbsp, true, 0).expect("PTL should parse");
        assert_eq!(ptl.general_profile_idc, 1);
        assert!(!ptl.general_tier_flag);
        assert_eq!(ptl.general_level_idc, 63);
    }
}
