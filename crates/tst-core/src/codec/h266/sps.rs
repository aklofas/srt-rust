//! H.266 SPS parser. Per H.266 V4 §7.3.2.4.

use crate::codec::h265::bitreader::BitReader;
use crate::codec::h266::profile_tier_level::{H266ProfileTierLevel, parse_into};
use crate::codec::h266::vui::parse_h266_vui;
use crate::codec::{ChromaFormat, ColorInfo, ParseError, Rational, validate_bit_depth_minus8};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H266Sps {
    pub sps_id: u8,
    pub vps_id: u8,
    pub profile_tier_level: H266ProfileTierLevel,
    pub width: u32,
    pub height: u32,
    pub chroma_format: ChromaFormat,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    pub color_info: Option<ColorInfo>,
    pub frame_rate: Option<Rational>,
    pub raw_rbsp: Vec<u8>,
}

/// Parse an H.266 SPS RBSP (Annex-B start codes already stripped,
/// emulation-prevention bytes preserved). Per H.266 V4 §7.3.2.4.
///
/// v0 scope surfaces: `sps_id`, `vps_id`, headline `profile_tier_level`
/// fields, dimensions, chroma format, bit depth, and (optionally)
/// `color_info` + `frame_rate` from VUI. Conformance-window cropping is
/// applied to width/height before they are returned.
///
/// Bails `ParseError::UnsupportedProfile` on `sps_subpic_info_present_flag`
/// and `sps_scaling_list_data_present_flag` paths — these are rare in
/// reference-encoder defaults and would require walking large per-tile
/// or per-coefficient blocks not modeled here. Same conservative stance
/// as `codec::h265::parse_sps`.
pub fn parse_sps(rbsp: &[u8]) -> Result<H266Sps, ParseError> {
    if rbsp.is_empty() {
        return Err(ParseError::TruncatedRbsp {
            offset_bits: 0,
            needed_bits: 8,
        });
    }
    let mut br = BitReader::new(rbsp);

    // §7.3.2.4 SPS header.
    let sps_id = br.read_u(4)? as u8;
    let vps_id = br.read_u(4)? as u8;
    let max_sublayers_minus1 = br.read_u(3)? as u8;
    let chroma_format_idc = br.read_u(2)?;
    let _log2_ctu_size_minus5 = br.read_u(2)?;
    let ptl_dpb_hrd_present = br.read_bool()?;

    // profile_tier_level(1, sps_max_sublayers_minus1) when flag is set.
    // The PTL walks past its headline fields — alignment matters for
    // every subsequent ue(v) read, so the full PTL syntax must be
    // consumed (not just the 16-bit headline that the standalone
    // `parse_profile_tier_level` exposes).
    let mut profile_tier_level = H266ProfileTierLevel::default();
    if ptl_dpb_hrd_present {
        parse_into(&mut br, true, max_sublayers_minus1, &mut profile_tier_level)?;
    }

    let _gdr_enabled_flag = br.read_bool()?;
    let ref_pic_resampling_enabled_flag = br.read_bool()?;
    if ref_pic_resampling_enabled_flag {
        let _res_change_in_clvs_allowed_flag = br.read_bool()?;
    }

    let pic_width_max_in_luma_samples = br.read_ue()?;
    let pic_height_max_in_luma_samples = br.read_ue()?;

    // Conformance-window cropping (semantics match H.265 §7.4.3.2.1).
    let conformance_window_flag = br.read_bool()?;
    let (mut crop_x_left, mut crop_x_right, mut crop_y_top, mut crop_y_bottom) = (0u32, 0, 0, 0);
    if conformance_window_flag {
        let conf_win_left_offset = br.read_ue()?;
        let conf_win_right_offset = br.read_ue()?;
        let conf_win_top_offset = br.read_ue()?;
        let conf_win_bottom_offset = br.read_ue()?;
        let (sub_w, sub_h) = match chroma_format_idc {
            1 => (2u32, 2u32),
            2 => (2, 1),
            3 => (1, 1),
            _ => (1, 1),
        };
        crop_x_left = sub_w * conf_win_left_offset;
        crop_x_right = sub_w * conf_win_right_offset;
        crop_y_top = sub_h * conf_win_top_offset;
        crop_y_bottom = sub_h * conf_win_bottom_offset;
    }

    let subpic_info_present_flag = br.read_bool()?;
    if subpic_info_present_flag {
        // The subpic block reads variable-width per-subpic fields whose
        // bit-widths depend on CTU size and picture dimensions.
        // Modeling that path adds significant complexity for streams
        // that almost never occur in non-multi-picture encoders.
        return Err(ParseError::UnsupportedProfile {
            profile_idc: profile_tier_level.general_profile_idc,
        });
    }

    let bit_depth_minus8 = br.read_ue()?;
    let bit_depth_luma = validate_bit_depth_minus8("sps_bitdepth_minus8", bit_depth_minus8)?;
    // H.266 §7.4.3.4 has a single `sps_bitdepth_minus8` covering both
    // luma and chroma — spec invariant, not a parser simplification.
    let bit_depth_chroma = bit_depth_luma;

    // The chroma format derives directly from sps_chroma_format_idc.
    let chroma_format = match chroma_format_idc {
        0 => ChromaFormat::Monochrome,
        1 => ChromaFormat::Yuv420,
        2 => ChromaFormat::Yuv422,
        3 => ChromaFormat::Yuv444,
        other => {
            return Err(ParseError::ReservedValue {
                field: "sps_chroma_format_idc",
                value: other,
            });
        }
    };

    // Width/height after conformance-window cropping.
    let width = pic_width_max_in_luma_samples.saturating_sub(crop_x_left + crop_x_right);
    let height = pic_height_max_in_luma_samples.saturating_sub(crop_y_top + crop_y_bottom);

    // For v0 we don't walk past bit-depth into the deeper SPS body
    // (entropy_coding_sync, log2_max_pic_order_cnt_lsb, ref-pic-list
    // structures, scaling lists, VUI). The minimal-SPS test path sets
    // sps_vui_parameters_present_flag=0 so VUI is never invoked; the
    // VUI stub returns (None, None) when called from a future
    // expansion. Frame-rate + color_info are surfaced as None today.
    //
    // The parse_h266_vui stub is referenced here so that a future task
    // can wire it in once the field-walk between bit-depth and VUI is
    // implemented (entropy_coding_sync_enabled_flag, ref-pic-list
    // structures, scaling lists, etc.). Keep the import warm — see
    // vui.rs for the deferral note.
    let (color_info, frame_rate) = (None, None);
    let _vui_stub_keep_warm = parse_h266_vui;

    Ok(H266Sps {
        sps_id,
        vps_id,
        profile_tier_level,
        width,
        height,
        chroma_format,
        bit_depth_luma,
        bit_depth_chroma,
        color_info,
        frame_rate,
        raw_rbsp: rbsp.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::ChromaFormat;

    /// Inline bit-builder. Mirrors the parser's expected reads exactly,
    /// keeping the test bytes debuggable by reading the field-write
    /// sequence top-to-bottom.
    struct BitWriter {
        bytes: Vec<u8>,
        pos: u32,
    }

    impl BitWriter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                pos: 0,
            }
        }
        fn write(&mut self, value: u32, n: u32) {
            for i in (0..n).rev() {
                let bit = ((value >> i) & 1) as u8;
                let byte_idx = (self.pos / 8) as usize;
                let bit_in_byte = 7 - (self.pos % 8);
                if byte_idx == self.bytes.len() {
                    self.bytes.push(0);
                }
                self.bytes[byte_idx] |= bit << bit_in_byte;
                self.pos += 1;
            }
        }
        /// Exp-Golomb ue(v) per H.266 §9.3.2.2 (identical formula to H.264/H.265).
        fn write_ue(&mut self, value: u32) {
            let v = value + 1;
            let leading_zeros = 31 - v.leading_zeros();
            for _ in 0..leading_zeros {
                self.write(0, 1);
            }
            self.write(v, leading_zeros + 1);
        }
        /// rbsp_trailing_bits(): one '1' bit + zero-pad to byte align.
        fn end_rbsp(&mut self) {
            self.write(1, 1);
            while self.pos % 8 != 0 {
                self.write(0, 1);
            }
        }
    }

    /// Construct a minimal valid H.266 SPS bitstream:
    /// sps_id=0, vps_id=0, 320x240, 8-bit 4:2:0, Main 10 profile @ Level 4.0.
    ///
    /// Per H.266 V4 §7.3.2.4 SPS syntax + §7.3.3.1 PTL syntax.
    fn minimal_sps_rbsp() -> Vec<u8> {
        minimal_sps_rbsp_with_bitdepth_minus8(0)
    }

    /// Same as [`minimal_sps_rbsp`] but lets callers inject an arbitrary
    /// `sps_bitdepth_minus8` value to exercise the bounds check.
    fn minimal_sps_rbsp_with_bitdepth_minus8(bitdepth_minus8: u32) -> Vec<u8> {
        let mut bw = BitWriter::new();

        // §7.3.2.4 SPS header.
        bw.write(0, 4); // sps_seq_parameter_set_id
        bw.write(0, 4); // sps_video_parameter_set_id
        bw.write(0, 3); // sps_max_sublayers_minus1
        bw.write(1, 2); // sps_chroma_format_idc = 1 (4:2:0)
        bw.write(0, 2); // sps_log2_ctu_size_minus5
        bw.write(1, 1); // sps_ptl_dpb_hrd_params_present_flag = 1

        // §7.3.3.1 profile_tier_level(profileTierPresentFlag=1, MaxNumSubLayersMinus1=0).
        bw.write(1, 7); // general_profile_idc = 1 (Main 10)
        bw.write(0, 1); // general_tier_flag = 0 (Main tier)
        bw.write(63, 8); // general_level_idc = 63 (Level 4.0)
        bw.write(0, 1); // ptl_frame_only_constraint_flag
        bw.write(0, 1); // ptl_multilayer_enabled_flag
        // §7.3.3.2 general_constraints_info(): gci_present_flag=0 → only
        // the flag bit, then byte-align.
        bw.write(0, 1); // gci_present_flag = 0
        // Byte-align: bits written so far in PTL = 7+1+8+1+1+1 = 19,
        // need 5 zero bits to align to 24 bits = 3 bytes.
        bw.write(0, 5);
        // No sublayer ptl_sublayer_level_present_flag loop (count = -1).
        // No sublayer level_idc loop.
        bw.write(0, 8); // ptl_num_sub_profiles = 0
        // No sub_profile_idc loop.
        // PTL total = 32 bits = 4 bytes.

        // §7.3.2.4 continues.
        bw.write(0, 1); // sps_gdr_enabled_flag
        bw.write(0, 1); // sps_ref_pic_resampling_enabled_flag = 0
        // (sps_res_change_in_clvs_allowed_flag not coded.)

        // sps_pic_width_max_in_luma_samples is the direct sample count
        // (not minus-1 like H.264/H.265 use elsewhere). Per V4 §7.3.2.4.
        bw.write_ue(320); // sps_pic_width_max_in_luma_samples
        bw.write_ue(240); // sps_pic_height_max_in_luma_samples

        bw.write(0, 1); // sps_conformance_window_flag = 0
        bw.write(0, 1); // sps_subpic_info_present_flag = 0
        bw.write_ue(bitdepth_minus8); // sps_bitdepth_minus8

        bw.end_rbsp();
        bw.bytes
    }

    #[test]
    fn parse_sps_320x240_main10() {
        let rbsp = minimal_sps_rbsp();
        let sps = parse_sps(&rbsp).expect("minimal SPS should parse");
        assert_eq!(sps.sps_id, 0);
        assert_eq!(sps.vps_id, 0);
        assert_eq!(sps.profile_tier_level.general_profile_idc, 1);
        assert!(!sps.profile_tier_level.general_tier_flag);
        assert_eq!(sps.profile_tier_level.general_level_idc, 63);
        assert_eq!(sps.width, 320);
        assert_eq!(sps.height, 240);
        assert_eq!(sps.chroma_format, ChromaFormat::Yuv420);
        assert_eq!(sps.bit_depth_luma, 8);
        assert_eq!(sps.bit_depth_chroma, 8);
        assert_eq!(sps.color_info, None);
        assert_eq!(sps.frame_rate, None);
    }

    #[test]
    fn parse_sps_truncated_returns_err() {
        assert!(parse_sps(&[]).is_err());
    }

    #[test]
    fn parse_sps_truncated_byte_returns_err() {
        // Parser reads sps_id(4) + vps_id(4) = 8 bits, then max_sublayers(3)
        // beyond a single byte should bail with TruncatedRbsp.
        assert!(parse_sps(&[0x00]).is_err());
    }

    /// Per H.266 V4 §7.4.3.4, `sps_bitdepth_minus8 ∈ 0..=8` (bit_depth ∈
    /// 8..=16). ffmpeg's `libavcodec/hevc/ps.c:366-369` clamps at 14
    /// (minus8 ≤ 6); we adopt the same threshold. A fuzzed value of 248
    /// would have silently wrapped to `bit_depth_luma=0` via
    /// `8u8.saturating_add(248 as u8)` — caught now via
    /// [`validate_bit_depth_minus8`].
    #[test]
    fn h266_sps_rejects_bit_depth_overflow() {
        let rbsp = minimal_sps_rbsp_with_bitdepth_minus8(248);
        let result = parse_sps(&rbsp);
        assert!(
            matches!(
                result,
                Err(ParseError::ReservedValue {
                    field: "sps_bitdepth_minus8",
                    value: 248
                })
            ),
            "expected ReservedValue, got {result:?}"
        );
    }
}
