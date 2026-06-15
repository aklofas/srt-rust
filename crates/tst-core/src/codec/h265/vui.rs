//! `vui_parameters()` parser per H.265 §E.2.1. Only the fields surfaced
//! on [`crate::codec::ColorInfo`] and frame_rate are decoded; the rest
//! are skipped.

use crate::codec::CodecParseError;
use crate::codec::bitreader::BitReader;
use crate::codec::{
    ColorInfo, ColourPrimaries, MatrixCoefficients, Rational, TransferCharacteristics,
};

pub(crate) struct VuiOut {
    pub frame_rate: Option<Rational>,
    pub color: Option<ColorInfo>,
}

pub(crate) fn parse(
    br: &mut BitReader<'_>,
    _max_sub_layers_minus1: u8,
) -> Result<VuiOut, CodecParseError> {
    let aspect_ratio_info_present_flag = br.read_bool()?;
    let mut sample_aspect_ratio = None;
    if aspect_ratio_info_present_flag {
        let aspect_ratio_idc = br.read_u(8)? as u8;
        sample_aspect_ratio = aspect_ratio_idc_to_sar(aspect_ratio_idc);
        if aspect_ratio_idc == 255 {
            let w = br.read_u(16)?;
            let h = br.read_u(16)?;
            sample_aspect_ratio = Some(Rational { num: w, den: h });
        }
    }

    let overscan_info_present_flag = br.read_bool()?;
    if overscan_info_present_flag {
        br.skip(1)?;
    }

    let video_signal_type_present_flag = br.read_bool()?;
    let mut full_range = false;
    let mut primaries = ColourPrimaries::Unspecified;
    let mut transfer = TransferCharacteristics::Unspecified;
    let mut matrix = MatrixCoefficients::Unspecified;
    if video_signal_type_present_flag {
        let _video_format = br.read_u(3)?;
        full_range = br.read_bool()?;
        let colour_description_present_flag = br.read_bool()?;
        if colour_description_present_flag {
            primaries = ColourPrimaries::from_h273(br.read_u(8)? as u8);
            transfer = TransferCharacteristics::from_h273(br.read_u(8)? as u8);
            matrix = MatrixCoefficients::from_h273(br.read_u(8)? as u8);
        }
    }

    let chroma_loc_info_present_flag = br.read_bool()?;
    let mut chroma_loc = None;
    if chroma_loc_info_present_flag {
        let top = super::read_ue_max(br, "chroma_sample_loc_type_top_field", 5)? as u8;
        let _bottom = br.read_ue()?;
        chroma_loc = Some(top);
    }

    let _neutral_chroma_indication_flag = br.read_bool()?;
    let _field_seq_flag = br.read_bool()?;
    let _frame_field_info_present_flag = br.read_bool()?;

    let default_display_window_flag = br.read_bool()?;
    if default_display_window_flag {
        let _ = br.read_ue()?;
        let _ = br.read_ue()?;
        let _ = br.read_ue()?;
        let _ = br.read_ue()?;
    }

    let vui_timing_info_present_flag = br.read_bool()?;
    let mut frame_rate = None;
    if vui_timing_info_present_flag {
        let num_units_in_tick = br.read_u(32)?;
        let time_scale = br.read_u(32)?;
        if num_units_in_tick > 0 {
            frame_rate = Some(Rational {
                num: time_scale,
                den: num_units_in_tick,
            });
        }
    }

    let color = if video_signal_type_present_flag
        || chroma_loc_info_present_flag
        || aspect_ratio_info_present_flag
    {
        Some(ColorInfo {
            primaries,
            transfer,
            matrix,
            full_range,
            chroma_loc,
            sample_aspect_ratio,
        })
    } else {
        None
    };

    Ok(VuiOut { frame_rate, color })
}

fn aspect_ratio_idc_to_sar(idc: u8) -> Option<Rational> {
    Some(match idc {
        1 => Rational { num: 1, den: 1 },
        2 => Rational { num: 12, den: 11 },
        3 => Rational { num: 10, den: 11 },
        4 => Rational { num: 16, den: 11 },
        5 => Rational { num: 40, den: 33 },
        6 => Rational { num: 24, den: 11 },
        7 => Rational { num: 20, den: 11 },
        8 => Rational { num: 32, den: 11 },
        9 => Rational { num: 80, den: 33 },
        10 => Rational { num: 18, den: 11 },
        11 => Rational { num: 15, den: 11 },
        12 => Rational { num: 64, den: 33 },
        13 => Rational { num: 160, den: 99 },
        14 => Rational { num: 4, den: 3 },
        15 => Rational { num: 3, den: 2 },
        16 => Rational { num: 2, den: 1 },
        _ => return None,
    })
}
