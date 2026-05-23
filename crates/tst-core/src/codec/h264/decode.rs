//! H.264 / AVC parameter-set decoders (private; re-exported through [`super`]).

use super::model::{EntropyCodingMode, H264ParameterSets, H264Pps, H264Sps};
use crate::codec::{
    ChromaFormat, CodecParseError, ColorInfo, ColourPrimaries, MatrixCoefficients, Rational,
    TransferCharacteristics,
};
use crate::mpegts::demux::event::NalUnit;

use h264_reader::nal::sps::SeqParameterSet;
use h264_reader::rbsp::{BitRead, BitReader, ByteReader};

/// Parse all SPS/PPS NAL units from a slice of [`NalUnit`]s.
///
/// Behavior is partial-success-tolerant: bad parameter set NALs emit a
/// `tracing::warn!` and are skipped. If every parameter set NAL in the
/// input failed, the function returns `Err`. Inputs that contain no
/// parameter set NALs (e.g., non-IDR access units, H.265-only slices)
/// return `Ok(H264ParameterSets::default())`.
///
/// After NAL collection, each PPS is cross-validated against the parsed
/// SPS map: per H.264 V15 §7.4.2.2, a PPS's `seq_parameter_set_id` must
/// refer to an SPS active in the stream. PPSes whose referenced SPS is
/// absent from the input emit a `tracing::warn!` and are dropped from
/// the output. (Strict cross-validation enforcement is not currently
/// surfaced as a typed error — see [`CodecParseError::DanglingSpsReference`]
/// for the variant reserved for future strict-mode use.)
pub fn parse_parameter_sets(nals: &[NalUnit]) -> Result<H264ParameterSets, CodecParseError> {
    let mut out = H264ParameterSets::default();
    let mut had_param_set = false;
    let mut all_failed = true;

    for nal in nals {
        let NalUnit::H264 {
            nal_type, payload, ..
        } = nal
        else {
            continue;
        };
        match *nal_type {
            7 => {
                had_param_set = true;
                match parse_sps(payload) {
                    Ok(sps) => {
                        out.sps_by_id.insert(sps.seq_parameter_set_id, sps);
                        all_failed = false;
                    }
                    Err(e) => {
                        tracing::warn!(target: "tst_core::codec::h264",
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
                        tracing::warn!(target: "tst_core::codec::h264",
                            error = ?e, "skipping malformed PPS");
                    }
                }
            }
            _ => {}
        }
    }

    if had_param_set && all_failed {
        return Err(CodecParseError::EngineError(
            "every parameter set NAL in the input failed to parse".into(),
        ));
    }

    // Cross-validate PPS→SPS references per H.264 V15 §7.4.2.2: each
    // PPS's `seq_parameter_set_id` must refer to an SPS present in the
    // stream. Drop dangling PPSes (matches the partial-success policy
    // used above for malformed SPS/PPS NALs).
    let sps_ids = &out.sps_by_id;
    out.pps_by_id.retain(|pps_id, pps| {
        if sps_ids.contains_key(&pps.seq_parameter_set_id) {
            true
        } else {
            tracing::warn!(
                target: "tst_core::codec::h264",
                pps_id = *pps_id,
                sps_id = pps.seq_parameter_set_id,
                "dropping PPS that references SPS id not in input"
            );
            false
        }
    });

    Ok(out)
}

/// Parse a single PPS RBSP. Same input contract as `parse_sps`. Strict
/// (returns Err on first failure). Range-checks both IDs per H.264 V15
/// §7.4.2.2: `pic_parameter_set_id` ∈ [0, 255], `seq_parameter_set_id`
/// ∈ [0, 31].
///
/// This standalone variant does NOT cross-validate that
/// `seq_parameter_set_id` references an SPS actually present elsewhere
/// in the stream — that cross-check is performed by
/// [`parse_parameter_sets`], which has visibility into both maps.
pub fn parse_pps(rbsp: &[u8]) -> Result<H264Pps, CodecParseError> {
    if rbsp.is_empty() {
        return Err(CodecParseError::TruncatedRbsp {
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
        .map_err(|e| CodecParseError::EngineError(format!("{e:?}")))?;
    let sps_id = bit_reader
        .read_ue("seq_parameter_set_id")
        .map_err(|e| CodecParseError::EngineError(format!("{e:?}")))?;
    let cabac = bit_reader
        .read_bool("entropy_coding_mode_flag")
        .map_err(|e| CodecParseError::EngineError(format!("{e:?}")))?;
    // H.264 V15 §7.4.2.2 (PDF p. 109):
    //   pic_parameter_set_id ∈ [0, 255]
    //   seq_parameter_set_id ∈ [0, 31] (NOT the same range as pic id)
    // The PPS ID storage type (u8) enforces the first bound implicitly via
    // u8::try_from; the SPS ID bound is tighter than u8 and must be
    // checked explicitly. Accepting sps_id ∈ [32, 255] would silently
    // create a PPS that can never match any spec-conformant SPS map.
    let pic_parameter_set_id =
        u8::try_from(pps_id).map_err(|_| CodecParseError::ReservedValue {
            field: "pic_parameter_set_id",
            value: pps_id,
        })?;
    if sps_id > 31 {
        return Err(CodecParseError::ReservedValue {
            field: "seq_parameter_set_id",
            value: sps_id,
        });
    }
    let seq_parameter_set_id = sps_id as u8;
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
pub fn parse_sps(rbsp: &[u8]) -> Result<H264Sps, CodecParseError> {
    if rbsp.is_empty() {
        return Err(CodecParseError::TruncatedRbsp {
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
        .map_err(|e| CodecParseError::EngineError(format!("{e:?}")))?;
    convert_sps(&parsed, rbsp)
}

fn convert_sps(p: &SeqParameterSet, rbsp: &[u8]) -> Result<H264Sps, CodecParseError> {
    // pixel_dimensions() implements the spec crop math correctly.
    // Propagate errors (FieldValueTooLarge, CroppingError) rather than
    // silently returning (0, 0) on malformed streams.
    let (width, height) = p
        .pixel_dimensions()
        .map_err(|e| CodecParseError::EngineError(format!("{e:?}")))?;

    let chroma_format = match p.chroma_info.chroma_format {
        h264_reader::nal::sps::ChromaFormat::Monochrome => ChromaFormat::Monochrome,
        h264_reader::nal::sps::ChromaFormat::YUV420 => ChromaFormat::Yuv420,
        h264_reader::nal::sps::ChromaFormat::YUV422 => ChromaFormat::Yuv422,
        h264_reader::nal::sps::ChromaFormat::YUV444 => ChromaFormat::Yuv444,
        // H.264 V15 §7.4.2.1.1: chroma_format_idc shall be in 0..=3.
        // h264-reader surfaces 4..=255 as Invalid(u32). Match the posture
        // of `validate_bit_depth_minus8` (mod.rs:282) and reject — the
        // downstream cropping math (lines 248-259) and chroma bit-depth
        // (line 281) both assume a spec-valid chroma_format_idc.
        h264_reader::nal::sps::ChromaFormat::Invalid(v) => {
            return Err(CodecParseError::ReservedValue {
                field: "chroma_format_idc",
                value: v,
            });
        }
    };

    let frame_mbs_only = matches!(
        p.frame_mbs_flags,
        h264_reader::nal::sps::FrameMbsFlags::Frames
    );

    // Convert h264-reader's chroma-unit crop offsets into luma samples per
    // H.264 §6.4. step_x = SubWidthC, step_y = SubHeightC * (2 -
    // frame_mbs_only_flag) — matches the math in
    // `SeqParameterSet::pixel_dimensions` so our (coded - cropped) reverses
    // exactly what the post-crop dimensions discarded.
    let mul: u32 = match p.frame_mbs_flags {
        h264_reader::nal::sps::FrameMbsFlags::Fields { .. } => 2,
        h264_reader::nal::sps::FrameMbsFlags::Frames => 1,
    };
    let vsub: u32 = if p.chroma_info.chroma_format == h264_reader::nal::sps::ChromaFormat::YUV420 {
        1
    } else {
        0
    };
    let hsub: u32 = if p.chroma_info.chroma_format == h264_reader::nal::sps::ChromaFormat::YUV420
        || p.chroma_info.chroma_format == h264_reader::nal::sps::ChromaFormat::YUV422
    {
        1
    } else {
        0
    };
    let step_x: u32 = 1 << hsub;
    let step_y: u32 = mul << vsub;
    let (crop_left, crop_right, crop_top, crop_bottom) = match p.frame_cropping.as_ref() {
        Some(c) => (
            c.left_offset.saturating_mul(step_x),
            c.right_offset.saturating_mul(step_x),
            c.top_offset.saturating_mul(step_y),
            c.bottom_offset.saturating_mul(step_y),
        ),
        None => (0, 0, 0, 0),
    };

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
        crop_left,
        crop_right,
        crop_top,
        crop_bottom,
        // Surfaced for slice_header_light: frame_num bit width = log2_max_frame_num_minus4 + 4.
        log2_max_frame_num_minus4: p.log2_max_frame_num_minus4,
        raw_rbsp: rbsp.to_vec(),
    })
}

fn extract_frame_rate(p: &SeqParameterSet) -> Option<Rational> {
    let vui = p.vui_parameters.as_ref()?;
    let timing = vui.timing_info.as_ref()?;
    // H.264 §E.2.1: frame_rate = time_scale / (2 * num_units_in_tick)
    // when fixed_frame_rate_flag is set. We report the ratio regardless
    // so callers can decide how to interpret it.
    //
    // num_units_in_tick is a u32, so `2 * num_units_in_tick` can overflow
    // in debug builds (panic) for streams claiming num_units_in_tick > u32::MAX/2.
    // CodecParseError rustdoc promises non-panicking parse — use saturating_mul
    // and treat saturation as "unknowable" by returning None rather than emitting
    // a nonsense ratio like `time_scale / u32::MAX`.
    let den = timing.num_units_in_tick.saturating_mul(2);
    if den == u32::MAX {
        return None;
    }
    Some(Rational {
        num: timing.time_scale,
        den,
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
    if profile == 66 {
        return false;
    }
    // H.264 §A.2: profile_idc=100 (High) admits two B-frameless subsets
    // distinguished by constraint flags:
    //   - Constrained High: constraint_set1_flag = 1
    //   - Constrained-Baseline-lifted-to-High:
    //       constraint_set4_flag = 1 AND constraint_set5_flag = 1
    // h264-reader's ConstraintFlags maps flag1/flag4/flag5 to
    // constraint_set1/4/5 respectively. Narrowed to profile_idc == 100
    // because the same constraint bits carry different semantics on other
    // profile_idc values (e.g., constraint_set1_flag on Baseline (66)
    // signals Main compatibility, not "no B-frames").
    if profile == 100 {
        let cf = p.constraint_flags;
        if cf.flag1() || (cf.flag4() && cf.flag5()) {
            return false;
        }
    }
    true
}

fn extract_color(p: &SeqParameterSet) -> Option<ColorInfo> {
    let vui = p.vui_parameters.as_ref()?;

    let (full_range, primaries, transfer, matrix) = if let Some(vs) = vui.video_signal_type.as_ref()
    {
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
