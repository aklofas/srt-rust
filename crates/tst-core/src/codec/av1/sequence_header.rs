//! AV1 Sequence Header OBU parser. Per AV1 spec §5.5.1 / §6.4.1.
//!
//! Surfaces the fields a TS / file consumer typically needs:
//! profile, level/tier of operating point 0, max frame dimensions,
//! bit depth, chroma format, monochrome flag, still-picture flags,
//! optional color description (mapped via H.273), and optional
//! frame rate (when timing info is present with `equal_picture_interval`).
//!
//! All other fields are walked-and-discarded — the bitstream cursor
//! must advance through every conditional path correctly so that
//! downstream parsers (e.g. frame_header) can rely on the OBU body
//! shape being well-formed even though we don't surface the data.

use crate::codec::av1::bitreader::Av1BitReader;
use crate::codec::{
    ChromaFormat, ColorInfo, ColourPrimaries, MatrixCoefficients, CodecParseError, Rational,
    TransferCharacteristics,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Av1SequenceHeader {
    pub profile: u8,
    /// `seq_level_idx[0]` — operating point 0 level index.
    pub level: u8,
    /// `seq_tier[0]` — operating point 0 tier (0 unless level > 7).
    pub tier: u8,
    /// `max_frame_width_minus_1 + 1`.
    pub max_frame_width: u32,
    /// `max_frame_height_minus_1 + 1`.
    pub max_frame_height: u32,
    /// 8, 10, or 12 per `BitDepth` derivation in §5.5.2.
    pub bit_depth: u8,
    pub monochrome: bool,
    pub chroma_format: ChromaFormat,
    pub still_picture: bool,
    pub reduced_still_picture_header: bool,
    pub color_info: Option<ColorInfo>,
    /// Frame rate derived from `time_scale / num_units_in_display_tick`,
    /// only populated when `timing_info_present_flag == 1` and
    /// `equal_picture_interval == 1`. Otherwise `None`.
    pub frame_rate: Option<Rational>,
    pub raw: Vec<u8>,
}

pub fn parse_sequence_header(payload: &[u8]) -> Result<Av1SequenceHeader, CodecParseError> {
    let mut br = Av1BitReader::new(payload);

    let profile = br.f(3)? as u8;
    let still_picture = br.f(1)? != 0;
    let reduced_still_picture_header = br.f(1)? != 0;

    let mut timing_info_present = false;
    let mut decoder_model_info_present = false;
    let mut initial_display_delay_present = false;
    let mut op_cnt_minus_1: usize = 0;
    let mut buffer_delay_length_minus_1: u8 = 0;
    let mut frame_rate: Option<Rational> = None;

    if reduced_still_picture_header {
        // Per spec: timing_info / decoder_model_info / initial_display_delay
        // all defaulted to 0, single op, seq_tier[0]=0. Only seq_level_idx[0]
        // is coded — read it here so the i==0 branch in the loop doesn't
        // misread.
    } else {
        timing_info_present = br.f(1)? != 0;
        if timing_info_present {
            // timing_info() — §5.5.3
            let num_units_in_display_tick = br.f(32)? as u32;
            let time_scale = br.f(32)? as u32;
            let equal_picture_interval = br.f(1)? != 0;
            if equal_picture_interval {
                let _num_ticks_per_picture_minus_1 = br.uvlc()?;
            }
            // Surface frame rate as time_scale / num_units_in_display_tick
            // when the encoder asserts a fixed picture interval. When
            // num_units_in_display_tick is 0 the rate is undefined per spec —
            // skip the surfacing rather than emit a div-by-zero rational.
            if equal_picture_interval && num_units_in_display_tick != 0 {
                frame_rate = Some(Rational {
                    num: time_scale,
                    den: num_units_in_display_tick,
                });
            }
            decoder_model_info_present = br.f(1)? != 0;
            if decoder_model_info_present {
                // decoder_model_info() — §5.5.4
                buffer_delay_length_minus_1 = br.f(5)? as u8;
                let _num_units_in_decoding_tick = br.f(32)?;
                let _buffer_removal_time_length_minus_1 = br.f(5)?;
                let _frame_presentation_time_length_minus_1 = br.f(5)?;
            }
        }
        initial_display_delay_present = br.f(1)? != 0;
        op_cnt_minus_1 = br.f(5)? as usize;
    }

    let mut level = 0u8;
    let mut tier = 0u8;

    if reduced_still_picture_header {
        // Single op, seq_level_idx[0] f(5), no seq_tier, no decoder_model
        // / initial_display_delay walks.
        level = br.f(5)? as u8;
        tier = 0;
    } else {
        for i in 0..=op_cnt_minus_1 {
            let _operating_point_idc = br.f(12)?;
            let seq_level_idx = br.f(5)? as u8;
            let seq_tier = if seq_level_idx > 7 { br.f(1)? as u8 } else { 0 };
            if i == 0 {
                level = seq_level_idx;
                tier = seq_tier;
            }
            if decoder_model_info_present {
                let dmpfto = br.f(1)? != 0;
                if dmpfto {
                    // operating_parameters_info(i) — §5.5.5:
                    //   decoder_buffer_delay[op] f(n)
                    //   encoder_buffer_delay[op] f(n)
                    //   low_delay_mode_flag[op]   f(1)
                    // where n = buffer_delay_length_minus_1 + 1.
                    let n = (buffer_delay_length_minus_1 as usize) + 1;
                    let _ = br.f(n)?;
                    let _ = br.f(n)?;
                    let _ = br.f(1)?;
                }
            }
            if initial_display_delay_present {
                let idd_present = br.f(1)? != 0;
                if idd_present {
                    let _initial_display_delay_minus_1 = br.f(4)?;
                }
            }
        }
    }

    let frame_width_bits_minus_1 = br.f(4)? as usize;
    let frame_height_bits_minus_1 = br.f(4)? as usize;
    let max_frame_width = (br.f(frame_width_bits_minus_1 + 1)? + 1) as u32;
    let max_frame_height = (br.f(frame_height_bits_minus_1 + 1)? + 1) as u32;

    let frame_id_numbers_present = if reduced_still_picture_header {
        false
    } else {
        br.f(1)? != 0
    };
    if frame_id_numbers_present {
        let _delta_frame_id_length_minus_2 = br.f(4)?;
        let _additional_frame_id_length_minus_1 = br.f(3)?;
    }

    let _use_128x128_superblock = br.f(1)? != 0;
    let _enable_filter_intra = br.f(1)? != 0;
    let _enable_intra_edge_filter = br.f(1)? != 0;
    if !reduced_still_picture_header {
        let _enable_interintra_compound = br.f(1)?;
        let _enable_masked_compound = br.f(1)?;
        let _enable_warped_motion = br.f(1)?;
        let _enable_dual_filter = br.f(1)?;
        let enable_order_hint = br.f(1)? != 0;
        if enable_order_hint {
            let _enable_jnt_comp = br.f(1)?;
            let _enable_ref_frame_mvs = br.f(1)?;
        }
        let seq_choose_screen_content_tools = br.f(1)? != 0;
        let seq_force_screen_content_tools = if seq_choose_screen_content_tools {
            // SELECT_SCREEN_CONTENT_TOOLS = 2 — implicit, not coded.
            2u8
        } else {
            br.f(1)? as u8
        };
        if seq_force_screen_content_tools > 0 {
            let seq_choose_integer_mv = br.f(1)? != 0;
            if !seq_choose_integer_mv {
                let _seq_force_integer_mv = br.f(1)?;
            }
            // else: SELECT_INTEGER_MV (implicit, not coded).
        }
        if enable_order_hint {
            let _order_hint_bits_minus_1 = br.f(3)?;
        }
    }

    let _enable_superres = br.f(1)? != 0;
    let _enable_cdef = br.f(1)? != 0;
    let _enable_restoration = br.f(1)? != 0;

    // color_config() — §5.5.2.
    let high_bitdepth = br.f(1)? != 0;
    let bit_depth = if profile == 2 && high_bitdepth {
        let twelve_bit = br.f(1)? != 0;
        if twelve_bit { 12 } else { 10 }
    } else if high_bitdepth {
        10
    } else {
        8
    };
    let monochrome = if profile == 1 { false } else { br.f(1)? != 0 };
    let color_description_present = br.f(1)? != 0;
    let (cp_byte, tc_byte, mc_byte) = if color_description_present {
        (br.f(8)? as u8, br.f(8)? as u8, br.f(8)? as u8)
    } else {
        // Defaults from §5.5.2: CP_UNSPECIFIED=2, TC_UNSPECIFIED=2, MC_UNSPECIFIED=2.
        (2u8, 2u8, 2u8)
    };

    // Stash ColorInfo only when color_description_present_flag=1 — implicit
    // (BT.709-derived) color values are noise without an explicit signal.
    let (full_range, subsampling_x, subsampling_y);
    if monochrome {
        full_range = br.f(1)? != 0;
        subsampling_x = true;
        subsampling_y = true;
        // For monochrome streams, chroma_sample_position and
        // separate_uv_delta_q are not coded in the bitstream — they're
        // inferred (CSP_UNKNOWN and 0 respectively) per AV1 §5.5.2. The
        // `if !monochrome` guards below skip those reads on this path.
    } else if cp_byte == 1 /* CP_BT_709 */
        && tc_byte == 13 /* TC_SRGB */
        && mc_byte == 0
    /* MC_IDENTITY */
    {
        // Implicit: color_range=1, subsampling_x=0, subsampling_y=0.
        full_range = true;
        subsampling_x = false;
        subsampling_y = false;
    } else {
        full_range = br.f(1)? != 0;
        if profile == 0 {
            subsampling_x = true;
            subsampling_y = true;
        } else if profile == 1 {
            subsampling_x = false;
            subsampling_y = false;
        } else {
            // profile == 2
            if bit_depth == 12 {
                let sx = br.f(1)? != 0;
                let sy = if sx { br.f(1)? != 0 } else { false };
                subsampling_x = sx;
                subsampling_y = sy;
            } else {
                subsampling_x = true;
                subsampling_y = false;
            }
        }
    }

    if !monochrome && subsampling_x && subsampling_y {
        let _chroma_sample_position = br.f(2)?;
    }
    if !monochrome {
        let _separate_uv_delta_q = br.f(1)?;
    }

    let chroma_format = if monochrome {
        ChromaFormat::Monochrome
    } else {
        match (subsampling_x, subsampling_y) {
            (false, false) => ChromaFormat::Yuv444,
            (true, false) => ChromaFormat::Yuv422,
            (true, true) => ChromaFormat::Yuv420,
            (false, true) => {
                // AV1 forbids 4:4:0 — spec doesn't define this combination;
                // surface as ReservedValue rather than a misleading enum.
                return Err(CodecParseError::ReservedValue {
                    field: "av1_subsampling",
                    value: 0b01,
                });
            }
        }
    };

    let color_info = if color_description_present {
        Some(ColorInfo {
            primaries: ColourPrimaries::from_h273(cp_byte),
            transfer: TransferCharacteristics::from_h273(tc_byte),
            matrix: MatrixCoefficients::from_h273(mc_byte),
            full_range,
            chroma_loc: None,
            sample_aspect_ratio: None,
        })
    } else {
        None
    };

    if !reduced_still_picture_header {
        let _film_grain_params_present = br.f(1)?;
    }

    // Suppress unused-warning paths the compiler can't see through.
    let _ = (timing_info_present, initial_display_delay_present);

    Ok(Av1SequenceHeader {
        profile,
        level,
        tier,
        max_frame_width,
        max_frame_height,
        bit_depth,
        monochrome,
        chroma_format,
        still_picture,
        reduced_still_picture_header,
        color_info,
        frame_rate,
        raw: payload.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::ChromaFormat;

    /// Append-only bit writer for hand-crafting AV1 OBU payloads.
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
        fn write(&mut self, value: u64, n: u32) {
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
    }

    /// Build a minimal Sequence Header OBU body.
    /// Main profile, non-still-picture, level 2.0 tier 0, 320x240,
    /// 8-bit 4:2:0, no color description, no timing info.
    /// Walks §5.5.1 in order — every `bw.write` is a single named
    /// syntax element from the spec.
    fn minimal_sequence_header() -> Vec<u8> {
        let mut bw = BitWriter::new();

        bw.write(0, 3); // seq_profile = 0 (Main)
        bw.write(0, 1); // still_picture = 0
        bw.write(0, 1); // reduced_still_picture_header = 0
        bw.write(0, 1); // timing_info_present_flag = 0
        // (timing_info(), decoder_model_info_present_flag, decoder_model_info()
        // all skipped because timing_info_present_flag == 0.)
        bw.write(0, 1); // initial_display_delay_present_flag = 0
        bw.write(0, 5); // operating_points_cnt_minus_1 = 0
        bw.write(0, 12); // operating_point_idc[0] = 0
        bw.write(0, 5); // seq_level_idx[0] = 0 (level 2.0; <=7 so no seq_tier)
        // (decoder_model_present_for_this_op + initial_display_delay_present_for_this_op
        // both skipped since their gate flags are 0.)

        bw.write(8, 4); // frame_width_bits_minus_1 = 8 (n=9 fits 320-1=319=0x13F)
        bw.write(7, 4); // frame_height_bits_minus_1 = 7 (n=8 fits 240-1=239=0xEF)
        bw.write(319, 9); // max_frame_width_minus_1 = 319
        bw.write(239, 8); // max_frame_height_minus_1 = 239

        bw.write(0, 1); // frame_id_numbers_present_flag = 0
        bw.write(0, 1); // use_128x128_superblock = 0
        bw.write(0, 1); // enable_filter_intra = 0
        bw.write(0, 1); // enable_intra_edge_filter = 0

        // !reduced_still_picture_header → tool flags are coded:
        bw.write(0, 1); // enable_interintra_compound = 0
        bw.write(0, 1); // enable_masked_compound = 0
        bw.write(0, 1); // enable_warped_motion = 0
        bw.write(0, 1); // enable_dual_filter = 0
        bw.write(0, 1); // enable_order_hint = 0
        // enable_order_hint=0 → enable_jnt_comp + enable_ref_frame_mvs not coded
        bw.write(0, 1); // seq_choose_screen_content_tools = 0
        bw.write(0, 1); // seq_force_screen_content_tools = 0
        // seq_force_screen_content_tools == 0 → seq_choose_integer_mv +
        // seq_force_integer_mv NOT coded (SELECT_INTEGER_MV implicit).
        // enable_order_hint=0 → order_hint_bits_minus_1 NOT coded.

        bw.write(0, 1); // enable_superres = 0
        bw.write(0, 1); // enable_cdef = 0
        bw.write(0, 1); // enable_restoration = 0

        // color_config():
        bw.write(0, 1); // high_bitdepth = 0 → BitDepth = 8
        // profile != 1 → mono_chrome IS coded:
        bw.write(0, 1); // mono_chrome = 0
        bw.write(0, 1); // color_description_present_flag = 0
        // !mono_chrome AND defaults are CP_UNSPECIFIED ≠ CP_BT_709 → else branch:
        bw.write(0, 1); // color_range = 0 (limited)
        // profile == 0 → subsampling_x=1, subsampling_y=1 (NOT coded)
        // subsampling_x && subsampling_y → chroma_sample_position f(2):
        bw.write(0, 2); // chroma_sample_position = 0 (CSP_UNKNOWN)
        bw.write(0, 1); // separate_uv_delta_q = 0

        bw.write(0, 1); // film_grain_params_present = 0

        bw.bytes
    }

    #[test]
    fn parse_sequence_header_main_320x240() {
        let payload = minimal_sequence_header();
        let seq = parse_sequence_header(&payload).expect("should parse");
        assert_eq!(seq.profile, 0);
        assert_eq!(seq.level, 0);
        assert_eq!(seq.tier, 0);
        assert_eq!(seq.max_frame_width, 320);
        assert_eq!(seq.max_frame_height, 240);
        assert_eq!(seq.bit_depth, 8);
        assert!(!seq.monochrome);
        assert_eq!(seq.chroma_format, ChromaFormat::Yuv420);
        assert!(!seq.still_picture);
        assert!(!seq.reduced_still_picture_header);
        assert_eq!(seq.color_info, None);
        assert_eq!(seq.frame_rate, None);
    }

    /// Reduced still-picture variant — single op, seq_level_idx[0] f(5)
    /// is coded directly (no operating_points_cnt_minus_1, no
    /// operating_point_idc[], no seq_tier).
    fn reduced_still_picture_header_payload() -> Vec<u8> {
        let mut bw = BitWriter::new();
        bw.write(0, 3); // seq_profile
        bw.write(1, 1); // still_picture = 1
        bw.write(1, 1); // reduced_still_picture_header = 1
        bw.write(0, 5); // seq_level_idx[0] = 0
        bw.write(8, 4); // frame_width_bits_minus_1 = 8
        bw.write(7, 4); // frame_height_bits_minus_1 = 7
        bw.write(319, 9); // max_frame_width_minus_1
        bw.write(239, 8); // max_frame_height_minus_1
        // reduced → frame_id_numbers_present NOT coded.
        bw.write(0, 1); // use_128x128_superblock
        bw.write(0, 1); // enable_filter_intra
        bw.write(0, 1); // enable_intra_edge_filter
        // reduced → tool flags + screen-content + integer-mv + order-hint NOT coded.
        bw.write(0, 1); // enable_superres
        bw.write(0, 1); // enable_cdef
        bw.write(0, 1); // enable_restoration
        bw.write(0, 1); // high_bitdepth = 0
        bw.write(0, 1); // mono_chrome = 0
        bw.write(0, 1); // color_description_present_flag
        bw.write(0, 1); // color_range
        bw.write(0, 2); // chroma_sample_position
        bw.write(0, 1); // separate_uv_delta_q
        // reduced → film_grain_params_present NOT coded.
        bw.bytes
    }

    #[test]
    fn parse_sequence_header_reduced_still_picture() {
        let payload = reduced_still_picture_header_payload();
        let seq = parse_sequence_header(&payload).expect("should parse");
        assert!(seq.still_picture);
        assert!(seq.reduced_still_picture_header);
        assert_eq!(seq.max_frame_width, 320);
        assert_eq!(seq.max_frame_height, 240);
        assert_eq!(seq.chroma_format, ChromaFormat::Yuv420);
    }

    /// Same as the minimal payload but with a color_description block
    /// asserting BT.2020 + PQ + BT.2020-NCL.
    fn payload_with_color_description() -> Vec<u8> {
        let mut bw = BitWriter::new();

        bw.write(0, 3); // seq_profile
        bw.write(0, 1); // still_picture
        bw.write(0, 1); // reduced_still_picture_header
        bw.write(0, 1); // timing_info_present_flag
        bw.write(0, 1); // initial_display_delay_present_flag
        bw.write(0, 5); // operating_points_cnt_minus_1
        bw.write(0, 12); // operating_point_idc[0]
        bw.write(0, 5); // seq_level_idx[0]

        bw.write(8, 4);
        bw.write(7, 4);
        bw.write(319, 9);
        bw.write(239, 8);

        bw.write(0, 1); // frame_id_numbers_present
        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);

        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1); // enable_order_hint
        bw.write(0, 1); // seq_choose_screen_content_tools
        bw.write(0, 1); // seq_force_screen_content_tools

        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);

        // color_config:
        bw.write(0, 1); // high_bitdepth = 0 → BitDepth=8
        bw.write(0, 1); // mono_chrome = 0
        bw.write(1, 1); // color_description_present_flag = 1
        bw.write(9, 8); // color_primaries = 9 (BT.2020)
        bw.write(16, 8); // transfer_characteristics = 16 (PQ)
        bw.write(9, 8); // matrix_coefficients = 9 (BT.2020 NCL)
        // !mono_chrome, AND not the special BT.709/sRGB/Identity case → else:
        bw.write(1, 1); // color_range = 1 (full)
        // profile == 0 → subsampling 1,1 (not coded)
        bw.write(0, 2); // chroma_sample_position
        bw.write(0, 1); // separate_uv_delta_q

        bw.write(0, 1); // film_grain_params_present
        bw.bytes
    }

    #[test]
    fn parse_sequence_header_color_description_bt2020_pq() {
        let payload = payload_with_color_description();
        let seq = parse_sequence_header(&payload).expect("should parse");
        let ci = seq.color_info.expect("color_info should be Some");
        assert_eq!(ci.primaries, ColourPrimaries::Bt2020);
        assert_eq!(ci.transfer, TransferCharacteristics::SmpteSt2084);
        assert_eq!(ci.matrix, MatrixCoefficients::Bt2020NonConstant);
        assert!(ci.full_range);
    }

    /// Timing info present with `equal_picture_interval=1` → frame_rate
    /// is surfaced as time_scale / num_units_in_display_tick.
    /// 30000 / 1001 = 29.97 fps.
    fn payload_with_timing_info() -> Vec<u8> {
        let mut bw = BitWriter::new();

        bw.write(0, 3); // seq_profile
        bw.write(0, 1); // still_picture
        bw.write(0, 1); // reduced_still_picture_header
        bw.write(1, 1); // timing_info_present_flag = 1
        bw.write(1001, 32); // num_units_in_display_tick
        bw.write(30000, 32); // time_scale
        bw.write(1, 1); // equal_picture_interval = 1
        bw.write(1, 1); // num_ticks_per_picture_minus_1 = 0 (uvlc encoding)
        bw.write(0, 1); // decoder_model_info_present_flag = 0
        bw.write(0, 1); // initial_display_delay_present_flag = 0
        bw.write(0, 5); // operating_points_cnt_minus_1
        bw.write(0, 12); // operating_point_idc[0]
        bw.write(0, 5); // seq_level_idx[0]

        bw.write(8, 4);
        bw.write(7, 4);
        bw.write(319, 9);
        bw.write(239, 8);

        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);

        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);

        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);

        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 1);
        bw.write(0, 2);
        bw.write(0, 1);

        bw.write(0, 1);
        bw.bytes
    }

    #[test]
    fn parse_sequence_header_timing_info_surfaces_frame_rate() {
        let payload = payload_with_timing_info();
        let seq = parse_sequence_header(&payload).expect("should parse");
        let fr = seq.frame_rate.expect("frame_rate should be Some");
        assert_eq!(fr.num, 30000);
        assert_eq!(fr.den, 1001);
    }

    #[test]
    fn truncated_payload_returns_err() {
        let payload = vec![0u8; 2]; // Way too short.
        let r = parse_sequence_header(&payload);
        assert!(r.is_err());
    }
}
