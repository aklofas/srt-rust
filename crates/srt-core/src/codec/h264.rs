//! H.264 / AVC parameter-set parsers.
//!
//! See [`crate::codec`] for umbrella architecture and design rationale.

use crate::codec::{
    ChromaFormat, ColorInfo, ColourPrimaries, MatrixCoefficients, ParseError, Rational,
    TransferCharacteristics,
};

use h264_reader::nal::sps::SeqParameterSet;
use h264_reader::rbsp::{BitReader, ByteReader};

/// Parsed H.264 Sequence Parameter Set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H264Sps {
    pub seq_parameter_set_id: u8,
    pub width: u32,
    pub height: u32,
    pub profile_idc: u8,
    pub level_idc: u8,
    pub constraint_set_flags: u8,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    pub chroma_format: ChromaFormat,
    pub frame_mbs_only: bool,
    pub frame_rate: Option<Rational>,
    pub fixed_frame_rate: bool,
    pub has_b_frames: bool,
    pub color: Option<ColorInfo>,
    /// The original RBSP bytes as supplied by the caller.
    pub raw_rbsp: Vec<u8>,
}

/// Parse a single SPS RBSP. Input contract: RBSP body only — Annex B
/// start code stripped, NAL header byte stripped, emulation prevention
/// bytes preserved (matches `NalUnit::H264 { payload }`).
pub fn parse_sps(rbsp: &[u8]) -> Result<H264Sps, ParseError> {
    if rbsp.is_empty() {
        return Err(ParseError::TruncatedRbsp {
            offset_bits: 0,
            needed_bits: 8,
        });
    }
    // ByteReader::without_skip strips emulation-prevention-three bytes
    // (the 0x03 escaping in the stream), producing clean RBSP for the
    // bit-level parser. No header bytes remain since callers strip the
    // 1-byte NAL header before passing the payload here.
    let byte_reader = ByteReader::without_skip(std::io::Cursor::new(rbsp));
    let bit_reader = BitReader::new(byte_reader);
    let parsed = SeqParameterSet::from_bits(bit_reader)
        .map_err(|e| ParseError::EngineError(format!("{e:?}")))?;
    Ok(convert_sps(&parsed, rbsp))
}

fn convert_sps(p: &SeqParameterSet, rbsp: &[u8]) -> H264Sps {
    // pixel_dimensions() implements the spec crop math correctly.
    let (width, height) = p.pixel_dimensions().unwrap_or((0, 0));

    let chroma_format = match p.chroma_info.chroma_format {
        h264_reader::nal::sps::ChromaFormat::Monochrome => ChromaFormat::Monochrome,
        h264_reader::nal::sps::ChromaFormat::YUV420 => ChromaFormat::Yuv420,
        h264_reader::nal::sps::ChromaFormat::YUV422 => ChromaFormat::Yuv422,
        h264_reader::nal::sps::ChromaFormat::YUV444 => ChromaFormat::Yuv444,
        h264_reader::nal::sps::ChromaFormat::Invalid(_) => ChromaFormat::Yuv420,
    };

    let frame_mbs_only = matches!(p.frame_mbs_flags, h264_reader::nal::sps::FrameMbsFlags::Frames);

    H264Sps {
        seq_parameter_set_id: p.seq_parameter_set_id.id(),
        width,
        height,
        profile_idc: p.profile_idc.into(),
        level_idc: p.level_idc,
        // ConstraintFlags is a newtype over u8 that implements Into<u8>.
        constraint_set_flags: p.constraint_flags.into(),
        bit_depth_luma: 8 + p.chroma_info.bit_depth_luma_minus8,
        bit_depth_chroma: 8 + p.chroma_info.bit_depth_chroma_minus8,
        chroma_format,
        frame_mbs_only,
        frame_rate: extract_frame_rate(p),
        fixed_frame_rate: extract_fixed_frame_rate(p),
        has_b_frames: extract_has_b_frames(p),
        color: extract_color(p),
        raw_rbsp: rbsp.to_vec(),
    }
}

fn extract_frame_rate(p: &SeqParameterSet) -> Option<Rational> {
    let vui = p.vui_parameters.as_ref()?;
    let timing = vui.timing_info.as_ref()?;
    // H.264 §E.2.1: frame_rate = time_scale / (2 * num_units_in_tick)
    // when fixed_frame_rate_flag is set. We report the ratio regardless
    // so callers can decide how to interpret it.
    Some(Rational {
        num: timing.time_scale,
        den: 2 * timing.num_units_in_tick,
    })
}

fn extract_fixed_frame_rate(p: &SeqParameterSet) -> bool {
    p.vui_parameters
        .as_ref()
        .and_then(|v| v.timing_info.as_ref())
        .map(|t| t.fixed_frame_rate_flag)
        .unwrap_or(false)
}

fn extract_has_b_frames(p: &SeqParameterSet) -> bool {
    // Prefer the explicit VUI bitstream_restrictions field when present.
    if let Some(vui) = p.vui_parameters.as_ref() {
        if let Some(restr) = vui.bitstream_restrictions.as_ref() {
            return restr.max_num_reorder_frames > 0;
        }
    }
    // Fallback: Baseline (66) never uses B-frames by definition.
    let profile: u8 = p.profile_idc.into();
    profile != 66
}

fn extract_color(p: &SeqParameterSet) -> Option<ColorInfo> {
    let vui = p.vui_parameters.as_ref()?;

    let (full_range, primaries, transfer, matrix) =
        if let Some(vs) = vui.video_signal_type.as_ref() {
            let full_range = vs.video_full_range_flag;
            let (prim, trc, mat) = match &vs.colour_description {
                Some(cd) => (
                    ColourPrimaries::from_h273(cd.colour_primaries),
                    TransferCharacteristics::from_h273(cd.transfer_characteristics),
                    MatrixCoefficients::from_h273(cd.matrix_coefficients),
                ),
                None => (
                    ColourPrimaries::Unspecified,
                    TransferCharacteristics::Unspecified,
                    MatrixCoefficients::Unspecified,
                ),
            };
            (full_range, prim, trc, mat)
        } else {
            (
                false,
                ColourPrimaries::Unspecified,
                TransferCharacteristics::Unspecified,
                MatrixCoefficients::Unspecified,
            )
        };

    let chroma_loc = vui
        .chroma_loc_info
        .as_ref()
        .map(|c| c.chroma_sample_loc_type_top_field as u8);

    // AspectRatioInfo::get() returns None for Unspecified and invalid
    // Extended(0,*)/(*,0) cases — preserves spec semantics cleanly.
    let sample_aspect_ratio =
        vui.aspect_ratio_info
            .as_ref()
            .and_then(|ar| ar.get())
            .map(|(w, h)| Rational {
                num: w as u32,
                den: h as u32,
            });

    Some(ColorInfo {
        primaries,
        transfer,
        matrix,
        full_range,
        chroma_loc,
        sample_aspect_ratio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPS_1080P_HIGH40: &[u8] = include_bytes!(
        "../../tests/fixtures/codec/h264/h264_1080p_high40_bt709_sps.bin"
    );
    const SPS_720P_MAIN31: &[u8] = include_bytes!(
        "../../tests/fixtures/codec/h264/h264_720p_main31_sps.bin"
    );

    #[test]
    fn parse_sps_1080p_high_dimensions() {
        let sps = parse_sps(SPS_1080P_HIGH40).expect("parse 1080p SPS");
        assert_eq!(sps.width, 1920);
        assert_eq!(sps.height, 1080);
        assert_eq!(sps.profile_idc, 100);
        assert_eq!(sps.level_idc, 40);
        assert_eq!(sps.bit_depth_luma, 8);
        assert_eq!(sps.bit_depth_chroma, 8);
        assert_eq!(sps.chroma_format, ChromaFormat::Yuv420);
        assert!(sps.frame_mbs_only);
        assert_eq!(sps.seq_parameter_set_id, 0);
    }

    #[test]
    fn parse_sps_720p_main_dimensions() {
        let sps = parse_sps(SPS_720P_MAIN31).expect("parse 720p SPS");
        assert_eq!(sps.width, 1280);
        assert_eq!(sps.height, 720);
        assert_eq!(sps.profile_idc, 77);
        assert_eq!(sps.level_idc, 31);
        assert_eq!(sps.chroma_format, ChromaFormat::Yuv420);
    }

    #[test]
    fn parse_sps_1080p_high_color_bt709() {
        let sps = parse_sps(SPS_1080P_HIGH40).expect("parse 1080p SPS");
        let color = sps.color.expect("VUI present in 1080p fixture");
        assert_eq!(color.primaries, ColourPrimaries::Bt709);
        assert_eq!(color.transfer, TransferCharacteristics::Bt709);
        assert_eq!(color.matrix, MatrixCoefficients::Bt709);
    }

    #[test]
    fn parse_sps_preserves_raw_rbsp() {
        let sps = parse_sps(SPS_1080P_HIGH40).expect("parse");
        assert_eq!(sps.raw_rbsp, SPS_1080P_HIGH40);
    }

    #[test]
    fn parse_sps_returns_err_on_garbage() {
        let bytes = [0xff_u8; 8];
        assert!(parse_sps(&bytes).is_err());
    }

    #[test]
    fn parse_sps_returns_err_on_empty() {
        assert!(parse_sps(&[]).is_err());
    }
}
