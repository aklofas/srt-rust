//! SPS parser per H.265 §7.3.2.2 + §E.2.1 (VUI).

use super::bitreader::BitReader;
use super::{profile_tier_level, vui};
use crate::codec::{ChromaFormat, ColorInfo, ParseError, Rational, validate_bit_depth_minus8};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H265Sps {
    pub sps_seq_parameter_set_id: u8,
    pub sps_video_parameter_set_id: u8,
    pub width: u32,
    pub height: u32,
    pub general_profile_idc: u8,
    pub general_tier_flag: bool,
    pub general_level_idc: u8,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    pub chroma_format: ChromaFormat,
    pub max_sub_layers_minus1: u8,
    pub frame_rate: Option<Rational>,
    pub color: Option<ColorInfo>,
    pub raw_rbsp: Vec<u8>,
}

pub fn parse_sps(rbsp: &[u8]) -> Result<H265Sps, ParseError> {
    if rbsp.is_empty() {
        return Err(ParseError::TruncatedRbsp {
            offset_bits: 0,
            needed_bits: 8,
        });
    }
    let mut br = BitReader::new(rbsp);
    let sps_video_parameter_set_id = br.read_u(4)? as u8;
    let max_sub_layers_minus1 = br.read_u(3)? as u8;
    let _temporal_id_nesting_flag = br.read_bool()?;

    let ptl = profile_tier_level::parse(&mut br, max_sub_layers_minus1)?;

    let sps_seq_parameter_set_id = br.read_ue()? as u8;
    let chroma_format_idc = br.read_ue()?;
    let separate_colour_plane_flag = if chroma_format_idc == 3 {
        br.read_bool()?
    } else {
        false
    };

    let pic_width_in_luma_samples = br.read_ue()?;
    let pic_height_in_luma_samples = br.read_ue()?;

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

    let bit_depth_luma_minus8 = br.read_ue()?;
    let bit_depth_luma = validate_bit_depth_minus8("bit_depth_luma_minus8", bit_depth_luma_minus8)?;
    let bit_depth_chroma_minus8 = br.read_ue()?;
    let bit_depth_chroma =
        validate_bit_depth_minus8("bit_depth_chroma_minus8", bit_depth_chroma_minus8)?;

    let log2_max_pic_order_cnt_lsb_minus4 = br.read_ue()?;

    let sub_layer_ordering_info_present_flag = br.read_bool()?;
    let layers_to_read = if sub_layer_ordering_info_present_flag {
        max_sub_layers_minus1 as usize + 1
    } else {
        1
    };
    for _ in 0..layers_to_read {
        let _ = br.read_ue()?;
        let _ = br.read_ue()?;
        let _ = br.read_ue()?;
    }

    let _ = br.read_ue()?;
    let _ = br.read_ue()?;
    let _ = br.read_ue()?;
    let _ = br.read_ue()?;
    let _ = br.read_ue()?;
    let _ = br.read_ue()?;

    let scaling_list_enabled_flag = br.read_bool()?;
    if scaling_list_enabled_flag {
        let sps_scaling_list_data_present_flag = br.read_bool()?;
        if sps_scaling_list_data_present_flag {
            return Err(ParseError::UnsupportedProfile {
                profile_idc: ptl.general_profile_idc,
            });
        }
    }

    let _amp_enabled_flag = br.read_bool()?;
    let _sample_adaptive_offset_enabled_flag = br.read_bool()?;
    let pcm_enabled_flag = br.read_bool()?;
    if pcm_enabled_flag {
        br.skip(4)?;
        br.skip(4)?;
        let _ = br.read_ue()?;
        let _ = br.read_ue()?;
        br.skip(1)?;
    }

    let num_short_term_ref_pic_sets = br.read_ue()?;
    if num_short_term_ref_pic_sets > 0 {
        return Err(ParseError::UnsupportedProfile {
            profile_idc: ptl.general_profile_idc,
        });
    }

    let long_term_ref_pics_present_flag = br.read_bool()?;
    if long_term_ref_pics_present_flag {
        let num_long_term_ref_pics_sps = br.read_ue()?;
        for _ in 0..num_long_term_ref_pics_sps {
            let _ = br.read_u(log2_max_pic_order_cnt_lsb_minus4 + 4)?;
            let _ = br.read_bool()?;
        }
    }

    let _sps_temporal_mvp_enabled_flag = br.read_bool()?;
    let _strong_intra_smoothing_enabled_flag = br.read_bool()?;

    let vui_parameters_present_flag = br.read_bool()?;
    let vui_out = if vui_parameters_present_flag {
        vui::parse(&mut br, max_sub_layers_minus1)?
    } else {
        vui::VuiOut {
            frame_rate: None,
            color: None,
        }
    };

    let chroma_format = match chroma_format_idc {
        0 => ChromaFormat::Monochrome,
        1 => ChromaFormat::Yuv420,
        2 => ChromaFormat::Yuv422,
        3 if !separate_colour_plane_flag => ChromaFormat::Yuv444,
        3 => ChromaFormat::Yuv444,
        other => {
            return Err(ParseError::ReservedValue {
                field: "chroma_format_idc",
                value: other,
            });
        }
    };

    let raw_w = pic_width_in_luma_samples;
    let raw_h = pic_height_in_luma_samples;
    let width = raw_w.saturating_sub(crop_x_left + crop_x_right);
    let height = raw_h.saturating_sub(crop_y_top + crop_y_bottom);

    Ok(H265Sps {
        sps_seq_parameter_set_id,
        sps_video_parameter_set_id,
        width,
        height,
        general_profile_idc: ptl.general_profile_idc,
        general_tier_flag: ptl.general_tier_flag,
        general_level_idc: ptl.general_level_idc,
        bit_depth_luma,
        bit_depth_chroma,
        chroma_format,
        max_sub_layers_minus1,
        frame_rate: vui_out.frame_rate,
        color: vui_out.color,
        raw_rbsp: rbsp.to_vec(),
    })
}
