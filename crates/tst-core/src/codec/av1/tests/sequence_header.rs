//! AV1 Sequence Header parser tests.

use crate::codec::av1::parse_sequence_header;
use crate::codec::{ChromaFormat, ColourPrimaries, MatrixCoefficients, TransferCharacteristics};

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
