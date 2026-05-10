//! `vui_parameters()` parser per H.266 V4 §7.3.2.5 / §E.2.1.
//!
//! H.266 VUI differs from H.265 in two key ways:
//!   - Four source flags precede aspect_ratio_info (H.265 has none).
//!   - Timing (num_units_in_tick / time_scale) is in §7.3.5.1
//!     general_timing_hrd_parameters(), NOT in VUI. So this function
//!     returns only `Option<ColorInfo>`.

use crate::codec::CodecParseError;
use crate::codec::bitreader::BitReader;
use crate::codec::{
    ColorInfo, ColourPrimaries, MatrixCoefficients, Rational, TransferCharacteristics,
};

/// Parse H.266 VUI per §7.3.2.5, returning `ColorInfo` when at least one
/// color/SAR field was present. Frame rate is NOT recovered here — it lives
/// in `general_timing_hrd_parameters()` (§7.3.5.1).
///
/// `payload_size_bytes` is `vui_payload_size_minus1 + 1` from the SPS.
pub(super) fn parse_h266_vui(
    br: &mut BitReader<'_>,
    _payload_size_bytes: usize,
) -> Result<Option<ColorInfo>, CodecParseError> {
    // §7.3.2.5 — four source flags (H.266-specific; precede aspect_ratio).
    let vui_progressive_source_flag = br.read_bool()?; // u(1)
    let vui_interlaced_source_flag = br.read_bool()?; // u(1)
    let _vui_non_packed_constraint_flag = br.read_bool()?; // u(1)
    let _vui_non_projected_constraint_flag = br.read_bool()?; // u(1)

    // §7.3.2.5 — aspect ratio.
    let vui_aspect_ratio_info_present_flag = br.read_bool()?; // u(1)
    let mut sample_aspect_ratio = None;
    if vui_aspect_ratio_info_present_flag {
        // H.266 adds vui_aspect_ratio_constant_flag before aspect_ratio_idc.
        let _vui_aspect_ratio_constant_flag = br.read_bool()?; // u(1)
        let aspect_ratio_idc = br.read_u(8)? as u8; // u(8)
        sample_aspect_ratio = aspect_ratio_idc_to_sar(aspect_ratio_idc);
        if aspect_ratio_idc == 255 {
            // EXTENDED_SAR: explicit numerator / denominator.
            let w = br.read_u(16)?; // u(16) sar_width
            let h = br.read_u(16)?; // u(16) sar_height
            sample_aspect_ratio = Some(Rational { num: w, den: h });
        }
    }

    // §7.3.2.5 — overscan info.
    let vui_overscan_info_present_flag = br.read_bool()?; // u(1)
    if vui_overscan_info_present_flag {
        let _vui_overscan_appropriate_flag = br.read_bool()?; // u(1)
    }

    // §7.3.2.5 — colour description per H.273.
    let vui_colour_description_present_flag = br.read_bool()?; // u(1)
    let mut full_range = false;
    let mut primaries = ColourPrimaries::Unspecified;
    let mut transfer = TransferCharacteristics::Unspecified;
    let mut matrix = MatrixCoefficients::Unspecified;
    if vui_colour_description_present_flag {
        primaries = ColourPrimaries::from_h273(br.read_u(8)? as u8); // u(8)
        transfer = TransferCharacteristics::from_h273(br.read_u(8)? as u8); // u(8)
        matrix = MatrixCoefficients::from_h273(br.read_u(8)? as u8); // u(8)
        full_range = br.read_bool()?; // u(1) vui_full_range_flag
    }

    // §7.3.2.5 — chroma sample location.
    let vui_chroma_loc_info_present_flag = br.read_bool()?; // u(1)
    let mut chroma_loc = None;
    if vui_chroma_loc_info_present_flag {
        if vui_progressive_source_flag && !vui_interlaced_source_flag {
            // Progressive: single chroma_sample_loc_type_frame ue(v).
            chroma_loc = Some(br.read_ue()? as u8); // ue(v)
        } else {
            // Interlaced (or unspecified): top_field + bottom_field.
            let top = br.read_ue()? as u8; // ue(v)
            let _bottom = br.read_ue()?; // ue(v)
            chroma_loc = Some(top);
        }
    }

    // Build ColorInfo when any color-related field was present.
    let color = if vui_colour_description_present_flag
        || vui_chroma_loc_info_present_flag
        || vui_aspect_ratio_info_present_flag
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

    Ok(color)
}

/// Map H.265/H.266 `aspect_ratio_idc` (Table E-1) to a `Rational` SAR.
/// Returns `None` for unspecified (0) and extended-SAR (255, handled by caller).
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
