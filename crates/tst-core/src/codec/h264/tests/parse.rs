//! H.264 parameter-set parser tests.

use crate::codec::h264::{parse_parameter_sets, parse_pps, parse_sps};
use crate::codec::{
    ChromaFormat, CodecParseError, ColourPrimaries, MatrixCoefficients, TransferCharacteristics,
};
use crate::mpegts::demux::event::NalUnit;

fn nal_h264(nal_type: u8, payload: Vec<u8>) -> NalUnit {
    NalUnit::H264 {
        nal_type,
        ref_idc: 3,
        payload,
    }
}

const SPS_1080P_HIGH40: &[u8] =
    include_bytes!("../../../../tests/fixtures/codec/h264/h264_1080p_high40_bt709_sps.bin");
const SPS_720P_MAIN31: &[u8] =
    include_bytes!("../../../../tests/fixtures/codec/h264/h264_720p_main31_sps.bin");

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
fn parse_sps_surfaces_frame_crop_offsets_invariant() {
    // Invariant: post-crop dims + crop offsets reconstruct the coded
    // dimensions exactly. Holds whether or not the fixture has
    // frame_cropping_flag set (uncropped → all four offsets are zero).
    for bytes in [SPS_1080P_HIGH40, SPS_720P_MAIN31] {
        let sps = parse_sps(bytes).expect("parse");
        assert_eq!(
            sps.coded_width(),
            sps.width + sps.crop_left + sps.crop_right,
            "coded_width helper must agree with field arithmetic"
        );
        assert_eq!(
            sps.coded_height(),
            sps.height + sps.crop_top + sps.crop_bottom,
            "coded_height helper must agree with field arithmetic"
        );
        // Coded dims are always macroblock-aligned (16x16) — H.264
        // pic_width_in_mbs × pic_height_in_map_units.
        assert_eq!(sps.coded_width() % 16, 0, "coded_width must be MB-aligned");
        assert_eq!(
            sps.coded_height() % 16,
            0,
            "coded_height must be MB-aligned"
        );
    }
}

#[test]
fn parse_sps_1080p_has_bottom_crop() {
    // The 1080p x264 fixture is coded as 1920×1088 (68 MB rows) and
    // signals frame_cropping_flag with bottom_offset that lops off the
    // 8 extra luma samples. crop_bottom is in chroma units; 4:2:0
    // SubHeightC=2 with frame_mbs_only=1 gives step_y=2 → 4×2=8 luma.
    let sps = parse_sps(SPS_1080P_HIGH40).expect("parse");
    assert_eq!(sps.width, 1920);
    assert_eq!(sps.height, 1080);
    assert_eq!(sps.coded_width(), 1920);
    assert_eq!(sps.coded_height(), 1088);
    assert_eq!(sps.crop_left, 0);
    assert_eq!(sps.crop_right, 0);
    assert_eq!(sps.crop_top, 0);
    assert_eq!(sps.crop_bottom, 8);
}

#[test]
fn parse_sps_720p_has_no_crop() {
    // 1280×720 = 80×45 macroblocks — no crop needed; SPS likely omits
    // frame_cropping_flag entirely. Confirm all four offsets are zero
    // and coded == post-crop.
    let sps = parse_sps(SPS_720P_MAIN31).expect("parse");
    assert_eq!(sps.crop_left, 0);
    assert_eq!(sps.crop_right, 0);
    assert_eq!(sps.crop_top, 0);
    assert_eq!(sps.crop_bottom, 0);
    assert_eq!(sps.coded_width(), sps.width);
    assert_eq!(sps.coded_height(), sps.height);
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

const PPS_1080P_HIGH40: &[u8] =
    include_bytes!("../../../../tests/fixtures/codec/h264/h264_1080p_high40_bt709_pps.bin");

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
        NalUnit::H265 {
            nal_type: 32,
            layer_id: 0,
            temporal_id_plus1: 1,
            payload: vec![0; 8],
        },
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
    let nals = vec![nal_h264(7, vec![0xff; 8]), nal_h264(8, vec![0x00; 8])];
    assert!(parse_parameter_sets(&nals).is_err());
}

// --- H264-01 regression test (chroma_format_idc Invalid arm) ---

/// Hand-crafted minimal High-profile SPS for exercising the
/// `chroma_format_idc` validation path. Returns the raw RBSP body (no
/// NAL header byte — `parse_sps` expects the caller to strip it).
///
/// Per H.264 V15 §7.3.2.1.1:
/// - profile_idc = 100 (High) → chroma_format_idc is signaled.
fn craft_high_sps_with_chroma_format_idc(idc: u32) -> Vec<u8> {
    struct Bw {
        bytes: Vec<u8>,
        pos: u32,
    }
    impl Bw {
        fn new() -> Self {
            Self {
                bytes: vec![],
                pos: 0,
            }
        }
        fn u(&mut self, value: u32, n: u32) {
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
        fn ue(&mut self, value: u32) {
            let v = value + 1;
            let leading_zeros = 31 - v.leading_zeros();
            for _ in 0..leading_zeros {
                self.u(0, 1);
            }
            self.u(v, leading_zeros + 1);
        }
        fn trailing(&mut self) {
            self.u(1, 1);
            while self.pos % 8 != 0 {
                self.u(0, 1);
            }
        }
    }
    let mut bw = Bw::new();
    // §7.3.2.1.1 SPS body (NAL header already stripped by caller).
    bw.u(100, 8); // profile_idc = 100 (High)
    bw.u(0, 8); // constraint_set_flags + reserved_zero_2bits = 0
    bw.u(40, 8); // level_idc = 40 (Level 4.0)
    bw.ue(0); // seq_parameter_set_id = 0
    // profile_idc == 100 → chroma_format_idc is present.
    bw.ue(idc); // chroma_format_idc (CALLER-SUPPLIED — possibly out-of-spec)
    if idc == 3 {
        bw.u(0, 1); // separate_colour_plane_flag (only when idc=3)
    }
    bw.ue(0); // bit_depth_luma_minus8 = 0
    bw.ue(0); // bit_depth_chroma_minus8 = 0
    bw.u(0, 1); // qpprime_y_zero_transform_bypass_flag = 0
    bw.u(0, 1); // seq_scaling_matrix_present_flag = 0
    bw.ue(0); // log2_max_frame_num_minus4 = 0
    bw.ue(0); // pic_order_cnt_type = 0
    bw.ue(0); // log2_max_pic_order_cnt_lsb_minus4 = 0
    bw.ue(1); // num_ref_frames = 1
    bw.u(0, 1); // gaps_in_frame_num_value_allowed_flag = 0
    bw.ue(19); // pic_width_in_mbs_minus1 = 19 (320px / 16)
    bw.ue(14); // pic_height_in_map_units_minus1 = 14 (240px / 16)
    bw.u(1, 1); // frame_mbs_only_flag = 1
    bw.u(0, 1); // direct_8x8_inference_flag = 0
    bw.u(0, 1); // frame_cropping_flag = 0
    bw.u(0, 1); // vui_parameters_present_flag = 0
    bw.trailing();
    bw.bytes
}

/// Per H.264 V15 §7.4.2.1.1, `chroma_format_idc` shall be in 0..=3.
/// `h264-reader 0.8` surfaces out-of-range values as
/// `ChromaFormat::Invalid(u32)`. The current `convert_sps` arm silently
/// coerces to `Yuv420`, producing a `H264Sps` with cropping math that
/// disagrees with the (also wrong) chroma_format. This test pins the
/// spec-correct behavior: reject with `CodecParseError::ReservedValue`.
#[test]
fn chroma_format_idc_5_rejected_with_reserved_value() {
    let rbsp = craft_high_sps_with_chroma_format_idc(5);
    match parse_sps(&rbsp) {
        Err(CodecParseError::ReservedValue { field, value }) => {
            assert_eq!(field, "chroma_format_idc");
            assert_eq!(value, 5);
        }
        other => {
            panic!("expected ReservedValue {{ field: chroma_format_idc, value: 5 }}, got {other:?}")
        }
    }
}

// --- A8 regression tests (seq_parameter_set_id range + cross-validation) ---

/// Hand-craft a PPS RBSP with the given `pic_parameter_set_id` and
/// `seq_parameter_set_id`, no NAL header. Per H.264 V15 §7.3.2.2 the PPS
/// body starts with two ue(v) fields and an entropy_coding_mode_flag bit,
/// then more fields we don't need to terminate before — the standalone
/// `parse_pps` decoder reads only those first three values, so we can stop
/// after the entropy_coding_mode_flag (no rbsp_trailing_bits needed; the
/// parser tolerates a payload that ends mid-byte).
fn craft_pps(pic_parameter_set_id: u32, seq_parameter_set_id: u32) -> Vec<u8> {
    struct Bw {
        bytes: Vec<u8>,
        pos: u32,
    }
    impl Bw {
        fn new() -> Self {
            Self {
                bytes: vec![],
                pos: 0,
            }
        }
        fn u(&mut self, value: u32, n: u32) {
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
        fn ue(&mut self, value: u32) {
            let v = value + 1;
            let leading_zeros = 31 - v.leading_zeros();
            for _ in 0..leading_zeros {
                self.u(0, 1);
            }
            self.u(v, leading_zeros + 1);
        }
        fn trailing(&mut self) {
            self.u(1, 1);
            while self.pos % 8 != 0 {
                self.u(0, 1);
            }
        }
    }
    let mut bw = Bw::new();
    bw.ue(pic_parameter_set_id);
    bw.ue(seq_parameter_set_id);
    bw.u(0, 1); // entropy_coding_mode_flag = 0 (CAVLC)
    bw.trailing();
    bw.bytes
}

/// Per H.264 V15 §7.4.2.2 (PDF p. 109): `seq_parameter_set_id` in a PPS
/// "shall be in the range of 0 to 31, inclusive". 31 is the boundary OK.
#[test]
fn parse_pps_with_sps_id_31_succeeds() {
    let rbsp = craft_pps(0, 31);
    let pps = parse_pps(&rbsp).expect("PPS with sps_id=31 must succeed");
    assert_eq!(pps.pic_parameter_set_id, 0);
    assert_eq!(pps.seq_parameter_set_id, 31);
}

/// Per H.264 V15 §7.4.2.2, 32 is just past the spec bound and must be
/// rejected with a typed error. Was silently accepted before A8 closed.
#[test]
fn parse_pps_with_sps_id_32_returns_typed_range_error() {
    let rbsp = craft_pps(0, 32);
    match parse_pps(&rbsp) {
        Err(CodecParseError::ReservedValue { field, value }) => {
            assert_eq!(field, "seq_parameter_set_id");
            assert_eq!(value, 32);
        }
        other => panic!(
            "expected ReservedValue {{ field: seq_parameter_set_id, value: 32 }}, got {other:?}"
        ),
    }
}

/// 255 was the upper bound of the old buggy [0, 255] range — must now be
/// rejected. Per H.264 V15 §7.4.2.2, `seq_parameter_set_id` ∈ [0, 31].
#[test]
fn parse_pps_with_sps_id_255_returns_typed_range_error() {
    let rbsp = craft_pps(0, 255);
    match parse_pps(&rbsp) {
        Err(CodecParseError::ReservedValue { field, value }) => {
            assert_eq!(field, "seq_parameter_set_id");
            assert_eq!(value, 255);
        }
        other => panic!(
            "expected ReservedValue {{ field: seq_parameter_set_id, value: 255 }}, got {other:?}"
        ),
    }
}

/// `parse_parameter_sets` cross-validates that each parsed PPS references
/// an SPS that is present in the input. On miss, it emits a warning
/// (matching the partial-success policy at lines 41-43 / 53-55 in
/// decode.rs) and drops the dangling PPS from the output. This test
/// builds a fixture with an SPS at id=0 and a PPS that points at id=7 —
/// the SPS is retained, the PPS is dropped.
#[test]
fn parse_parameter_sets_with_pps_referencing_missing_sps_drops_pps() {
    // sps_id=7 is in spec range (≤ 31) but no SPS with id=7 is in the input.
    let bad_pps = craft_pps(0, 7);
    let nals = vec![nal_h264(7, SPS_1080P_HIGH40.to_vec()), nal_h264(8, bad_pps)];
    let ps = parse_parameter_sets(&nals).expect("parse — SPS keeps it Ok");
    assert_eq!(ps.sps_by_id.len(), 1);
    // Dangling PPS dropped.
    assert!(
        ps.pps_by_id.is_empty(),
        "PPS referencing missing SPS must be dropped"
    );
}
