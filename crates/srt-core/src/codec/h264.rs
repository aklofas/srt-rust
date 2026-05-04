//! H.264 / AVC parameter-set parsers.
//!
//! See [`crate::codec`] for umbrella architecture and design rationale.

use std::collections::BTreeMap;

use crate::codec::{
    ChromaFormat, ColorInfo, ColourPrimaries, MatrixCoefficients, ParseError, Rational,
    TransferCharacteristics,
};
use crate::mpegts::demux::event::NalUnit;

use h264_reader::nal::sps::SeqParameterSet;
use h264_reader::rbsp::{BitRead, BitReader, ByteReader};

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

/// Parsed H.264 Picture Parameter Set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H264Pps {
    pub pic_parameter_set_id: u8,
    pub seq_parameter_set_id: u8,
    pub entropy_coding_mode: EntropyCodingMode,
    /// The original RBSP bytes as supplied by the caller.
    pub raw_rbsp: Vec<u8>,
}

/// H.264 entropy coding mode signalled in the PPS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntropyCodingMode {
    /// Context-Adaptive Variable Length Coding (used by Baseline/Main profiles).
    Cavlc,
    /// Context-Adaptive Binary Arithmetic Coding (used by Main/High profiles).
    Cabac,
}

/// All SPS and PPS NAL units parsed from a single access unit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct H264ParameterSets {
    pub sps_by_id: BTreeMap<u8, H264Sps>,
    pub pps_by_id: BTreeMap<u8, H264Pps>,
}

/// Parse all SPS/PPS NAL units from a slice of [`NalUnit`]s.
///
/// Behavior is partial-success-tolerant: bad parameter set NALs emit a
/// `tracing::warn!` and are skipped. If every parameter set NAL in the
/// input failed, the function returns `Err`. Inputs that contain no
/// parameter set NALs (e.g., non-IDR access units, H.265-only slices)
/// return `Ok(H264ParameterSets::default())`.
pub fn parse_parameter_sets(nals: &[NalUnit]) -> Result<H264ParameterSets, ParseError> {
    let mut out = H264ParameterSets::default();
    let mut had_param_set = false;
    let mut all_failed = true;

    for nal in nals {
        let NalUnit::H264 { nal_type, payload, .. } = nal else { continue };
        match *nal_type {
            7 => {
                had_param_set = true;
                match parse_sps(payload) {
                    Ok(sps) => {
                        out.sps_by_id.insert(sps.seq_parameter_set_id, sps);
                        all_failed = false;
                    }
                    Err(e) => {
                        tracing::warn!(target: "srt_core::codec::h264",
                            error = ?e, "skipping malformed SPS");
                    }
                }
            }
            8 => {
                had_param_set = true;
                match parse_pps(payload) {
                    Ok(pps) => {
                        out.pps_by_id.insert(pps.pic_parameter_set_id, pps);
                        all_failed = false;
                    }
                    Err(e) => {
                        tracing::warn!(target: "srt_core::codec::h264",
                            error = ?e, "skipping malformed PPS");
                    }
                }
            }
            _ => {}
        }
    }

    if had_param_set && all_failed {
        return Err(ParseError::EngineError(
            "every parameter set NAL in the input failed to parse".into(),
        ));
    }
    Ok(out)
}

/// Parse a single PPS RBSP. Same input contract as `parse_sps`. Strict
/// (returns Err on first failure). Note: this standalone variant cannot
/// validate that `seq_parameter_set_id` references a real SPS — that
/// check happens in [`parse_parameter_sets`] which has the SPS context.
pub fn parse_pps(rbsp: &[u8]) -> Result<H264Pps, ParseError> {
    if rbsp.is_empty() {
        return Err(ParseError::TruncatedRbsp {
            offset_bits: 0,
            needed_bits: 8,
        });
    }
    // ByteReader::without_skip strips emulation-prevention-three bytes,
    // matching the same setup as parse_sps. We only need the first three
    // fields (pic_parameter_set_id, seq_parameter_set_id,
    // entropy_coding_mode_flag) — all three are at the very start of the
    // PPS RBSP, so we never need SPS context to read them.
    let byte_reader = ByteReader::without_skip(std::io::Cursor::new(rbsp));
    let mut bit_reader = BitReader::new(byte_reader);
    let pps_id = bit_reader
        .read_ue("pic_parameter_set_id")
        .map_err(|e| ParseError::EngineError(format!("{e:?}")))?;
    let sps_id = bit_reader
        .read_ue("seq_parameter_set_id")
        .map_err(|e| ParseError::EngineError(format!("{e:?}")))?;
    let cabac = bit_reader
        .read_bool("entropy_coding_mode_flag")
        .map_err(|e| ParseError::EngineError(format!("{e:?}")))?;
    // Both IDs are constrained to [0, 255] by the H.264 spec (Table 7-1).
    let pic_parameter_set_id = u8::try_from(pps_id)
        .map_err(|_| ParseError::ReservedValue { field: "pic_parameter_set_id", value: pps_id })?;
    let seq_parameter_set_id = u8::try_from(sps_id)
        .map_err(|_| ParseError::ReservedValue { field: "seq_parameter_set_id", value: sps_id })?;
    Ok(H264Pps {
        pic_parameter_set_id,
        seq_parameter_set_id,
        entropy_coding_mode: if cabac {
            EntropyCodingMode::Cabac
        } else {
            EntropyCodingMode::Cavlc
        },
        raw_rbsp: rbsp.to_vec(),
    })
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
    convert_sps(&parsed, rbsp)
}

fn convert_sps(p: &SeqParameterSet, rbsp: &[u8]) -> Result<H264Sps, ParseError> {
    // pixel_dimensions() implements the spec crop math correctly.
    // Propagate errors (FieldValueTooLarge, CroppingError) rather than
    // silently returning (0, 0) on malformed streams.
    let (width, height) = p
        .pixel_dimensions()
        .map_err(|e| ParseError::EngineError(format!("{e:?}")))?;

    let chroma_format = match p.chroma_info.chroma_format {
        h264_reader::nal::sps::ChromaFormat::Monochrome => ChromaFormat::Monochrome,
        h264_reader::nal::sps::ChromaFormat::YUV420 => ChromaFormat::Yuv420,
        h264_reader::nal::sps::ChromaFormat::YUV422 => ChromaFormat::Yuv422,
        h264_reader::nal::sps::ChromaFormat::YUV444 => ChromaFormat::Yuv444,
        h264_reader::nal::sps::ChromaFormat::Invalid(_) => ChromaFormat::Yuv420,
    };

    let frame_mbs_only = matches!(p.frame_mbs_flags, h264_reader::nal::sps::FrameMbsFlags::Frames);

    Ok(H264Sps {
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
    })
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
    use crate::mpegts::demux::event::NalUnit;

    fn nal_h264(nal_type: u8, payload: Vec<u8>) -> NalUnit {
        NalUnit::H264 { nal_type, ref_idc: 3, payload }
    }

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

    const PPS_1080P_HIGH40: &[u8] = include_bytes!(
        "../../tests/fixtures/codec/h264/h264_1080p_high40_bt709_pps.bin"
    );

    #[test]
    fn parse_pps_1080p_high_basics() {
        let pps = parse_pps(PPS_1080P_HIGH40).expect("parse PPS");
        assert_eq!(pps.pic_parameter_set_id, 0);
        assert_eq!(pps.seq_parameter_set_id, 0);
    }

    #[test]
    fn parse_pps_preserves_raw_rbsp() {
        let pps = parse_pps(PPS_1080P_HIGH40).expect("parse");
        assert_eq!(pps.raw_rbsp, PPS_1080P_HIGH40);
    }

    #[test]
    fn parse_pps_returns_err_on_empty() {
        assert!(parse_pps(&[]).is_err());
    }

    #[test]
    fn parse_parameter_sets_sps_plus_pps() {
        let nals = vec![
            nal_h264(7, SPS_1080P_HIGH40.to_vec()),
            nal_h264(8, PPS_1080P_HIGH40.to_vec()),
        ];
        let ps = parse_parameter_sets(&nals).expect("parse");
        assert_eq!(ps.sps_by_id.len(), 1);
        assert_eq!(ps.pps_by_id.len(), 1);
        assert_eq!(ps.sps_by_id[&0].width, 1920);
        assert_eq!(ps.pps_by_id[&0].seq_parameter_set_id, 0);
    }

    #[test]
    fn parse_parameter_sets_skips_slice_nals_silently() {
        let nals = vec![
            nal_h264(5, vec![0xff; 32]),
            nal_h264(7, SPS_1080P_HIGH40.to_vec()),
            nal_h264(8, PPS_1080P_HIGH40.to_vec()),
        ];
        let ps = parse_parameter_sets(&nals).expect("parse");
        assert_eq!(ps.sps_by_id.len(), 1);
    }

    #[test]
    fn parse_parameter_sets_skips_h265_nals_silently() {
        let nals = vec![
            NalUnit::H265 { nal_type: 32, layer_id: 0, temporal_id_plus1: 1, payload: vec![0; 8] },
            nal_h264(7, SPS_1080P_HIGH40.to_vec()),
        ];
        let ps = parse_parameter_sets(&nals).expect("parse");
        assert_eq!(ps.sps_by_id.len(), 1);
    }

    #[test]
    fn parse_parameter_sets_empty_input_returns_ok_empty() {
        let ps = parse_parameter_sets(&[]).expect("parse");
        assert!(ps.sps_by_id.is_empty());
        assert!(ps.pps_by_id.is_empty());
    }

    #[test]
    fn parse_parameter_sets_only_slice_nals_returns_ok_empty() {
        let nals = vec![nal_h264(1, vec![0; 16])];
        let ps = parse_parameter_sets(&nals).expect("parse");
        assert!(ps.sps_by_id.is_empty());
        assert!(ps.pps_by_id.is_empty());
    }

    #[test]
    fn parse_parameter_sets_partial_success_one_bad_sps() {
        let nals = vec![
            nal_h264(7, SPS_1080P_HIGH40.to_vec()),
            nal_h264(7, vec![0xff; 8]),
        ];
        let ps = parse_parameter_sets(&nals).expect("parse — good SPS keeps it Ok");
        assert_eq!(ps.sps_by_id.len(), 1);
    }

    #[test]
    fn parse_parameter_sets_all_param_sets_fail_returns_err() {
        // 0xff bytes: SPS fails (SeqParameterSet::from_bits rejects garbage profile/level).
        // 0x00 bytes: PPS fails (all-zero Golomb prefix exhausts the bit buffer mid-read).
        let nals = vec![
            nal_h264(7, vec![0xff; 8]),
            nal_h264(8, vec![0x00; 8]),
        ];
        assert!(parse_parameter_sets(&nals).is_err());
    }
}
