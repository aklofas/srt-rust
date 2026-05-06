//! H.265 / HEVC parameter-set parsers.
//!
//! See [`crate::codec`] for umbrella architecture and design rationale.
//!
//! H.265 parsing is hand-rolled (the `hevc-parser` crate's struct fields
//! are crate-private, and `h265-parser` does not exist); reference
//! sections are noted on each module.

pub(crate) mod bitreader;
mod pps;
mod profile_tier_level;
mod sps;
mod vps;
mod vui;

pub use pps::{H265Pps, parse_pps};
pub use sps::{H265Sps, parse_sps};
pub use vps::{H265Vps, parse_vps};

use crate::codec::ParseError;
use crate::mpegts::demux::event::NalUnit;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct H265ParameterSets {
    pub vps_by_id: BTreeMap<u8, H265Vps>,
    pub sps_by_id: BTreeMap<u8, H265Sps>,
    pub pps_by_id: BTreeMap<u8, H265Pps>,
}

/// Parse all VPS/SPS/PPS NAL units from a slice. See [`crate::codec`]
/// crate root for the partial-success-tolerant behavior. Returns Ok with
/// empty maps when no parameter set NALs are present.
pub fn parse_parameter_sets(nals: &[NalUnit]) -> Result<H265ParameterSets, ParseError> {
    let mut out = H265ParameterSets::default();
    let mut had_param_set = false;
    let mut all_failed = true;

    for nal in nals {
        let NalUnit::H265 {
            nal_type, payload, ..
        } = nal
        else {
            continue;
        };
        match *nal_type {
            32 => {
                had_param_set = true;
                match parse_vps(payload) {
                    Ok(vps) => {
                        out.vps_by_id.insert(vps.vps_video_parameter_set_id, vps);
                        all_failed = false;
                    }
                    Err(e) => tracing::warn!(target: "srt_core::codec::h265",
                        error = ?e, "skipping malformed VPS"),
                }
            }
            33 => {
                had_param_set = true;
                match parse_sps(payload) {
                    Ok(sps) => {
                        out.sps_by_id.insert(sps.sps_seq_parameter_set_id, sps);
                        all_failed = false;
                    }
                    Err(e) => tracing::warn!(target: "srt_core::codec::h265",
                        error = ?e, "skipping malformed SPS"),
                }
            }
            34 => {
                had_param_set = true;
                match parse_pps(payload) {
                    Ok(pps) => {
                        out.pps_by_id.insert(pps.pps_pic_parameter_set_id, pps);
                        all_failed = false;
                    }
                    Err(e) => tracing::warn!(target: "srt_core::codec::h265",
                        error = ?e, "skipping malformed PPS"),
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

#[cfg(test)]
mod tests {
    use super::*;

    const VPS_1080P_MAIN40: &[u8] =
        include_bytes!("../../../tests/fixtures/codec/h265/h265_1080p_main40_vps.bin");
    const VPS_1080P_MAIN10_50: &[u8] =
        include_bytes!("../../../tests/fixtures/codec/h265/h265_1080p_main10_50_pq_vps.bin");

    #[test]
    fn parse_vps_1080p_main40_basics() {
        let vps = parse_vps(VPS_1080P_MAIN40).expect("parse VPS");
        assert_eq!(vps.vps_video_parameter_set_id, 0);
        assert_eq!(vps.general_level_idc, 120); // Level 4.0
        assert!(vps.general_tier_flag);
    }

    #[test]
    fn parse_vps_1080p_main10_50() {
        let vps = parse_vps(VPS_1080P_MAIN10_50).expect("parse VPS");
        assert_eq!(vps.general_level_idc, 150); // Level 5.0
        assert!(vps.general_tier_flag);
    }

    #[test]
    fn parse_vps_returns_err_on_garbage() {
        assert!(parse_vps(&[0xff; 8]).is_err());
    }
}

#[cfg(test)]
mod sps_tests {
    use super::*;
    use crate::codec::{
        ChromaFormat, ColourPrimaries, MatrixCoefficients, TransferCharacteristics,
    };

    const SPS_1080P_MAIN40: &[u8] =
        include_bytes!("../../../tests/fixtures/codec/h265/h265_1080p_main40_sps.bin");
    const SPS_1080P_MAIN10_50: &[u8] =
        include_bytes!("../../../tests/fixtures/codec/h265/h265_1080p_main10_50_pq_sps.bin");

    #[test]
    fn parse_sps_1080p_main40_dimensions() {
        let sps = parse_sps(SPS_1080P_MAIN40).expect("parse SPS");
        assert_eq!(sps.width, 1920);
        assert_eq!(sps.height, 1080);
        assert_eq!(sps.bit_depth_luma, 8);
        assert_eq!(sps.bit_depth_chroma, 8);
        assert_eq!(sps.chroma_format, ChromaFormat::Yuv420);
        assert_eq!(sps.sps_seq_parameter_set_id, 0);
        assert_eq!(sps.sps_video_parameter_set_id, 0);
        assert_eq!(sps.general_level_idc, 120);
    }

    #[test]
    fn parse_sps_1080p_main10_50_pq_color() {
        let sps = parse_sps(SPS_1080P_MAIN10_50).expect("parse SPS");
        assert_eq!(sps.bit_depth_luma, 10);
        assert_eq!(sps.bit_depth_chroma, 10);
        assert_eq!(sps.general_level_idc, 150);
        let color = sps.color.expect("VUI present");
        assert_eq!(color.primaries, ColourPrimaries::Bt2020);
        assert_eq!(color.transfer, TransferCharacteristics::SmpteSt2084);
        assert_eq!(color.matrix, MatrixCoefficients::Bt2020NonConstant);
    }

    #[test]
    fn parse_sps_preserves_raw_rbsp() {
        let sps = parse_sps(SPS_1080P_MAIN40).expect("parse");
        assert_eq!(sps.raw_rbsp, SPS_1080P_MAIN40);
    }

    #[test]
    fn parse_sps_surfaces_conformance_window_offsets_invariant() {
        // Invariant: post-crop dims + crop offsets reconstruct the coded
        // dimensions exactly. Holds whether or not the fixture has
        // `conformance_window_flag` set (uncropped → all four offsets are
        // zero). Coded dims are also CTB-aligned (the encoder pads pic
        // width/height up to a multiple of MinCbSizeY = 8, so the crop
        // adjusts at most 7 luma samples in each direction — the 1080p
        // Main fixture is coded as 1920×1088 and crops 8 off the bottom).
        for bytes in [SPS_1080P_MAIN40, SPS_1080P_MAIN10_50] {
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
            // MinCbSizeY = 8 for Main / Main10 — coded dims are 8-aligned.
            assert_eq!(sps.coded_width() % 8, 0, "coded_width must be CB-aligned");
            assert_eq!(sps.coded_height() % 8, 0, "coded_height must be CB-aligned");
        }
    }

    #[test]
    fn parse_sps_1080p_has_bottom_crop() {
        // The 1080p HEVC Main fixture is coded as 1920×1088 and signals
        // `conformance_window_flag` with `conf_win_bottom_offset = 2`
        // chroma units. 4:2:0 → SubHeightC = 2 → crop_bottom = 4 luma
        // samples (the parser computes `sub_h * conf_win_bottom_offset`).
        // After crop: 1080. Other three offsets are zero.
        let sps = parse_sps(SPS_1080P_MAIN40).expect("parse");
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
    fn parse_sps_returns_err_on_garbage() {
        assert!(parse_sps(&[0xff; 16]).is_err());
    }

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
        /// Exp-Golomb ue(v) per H.265 §9.2.2.
        fn write_ue(&mut self, value: u32) {
            let v = value + 1;
            let leading_zeros = 31 - v.leading_zeros();
            for _ in 0..leading_zeros {
                self.write(0, 1);
            }
            self.write(v, leading_zeros + 1);
        }
    }

    /// Construct a synthetic H.265 SPS prefix that walks correctly up
    /// through `bit_depth_luma_minus8`, then writes the caller-supplied
    /// value at that field. `parse_sps` validates eagerly right after
    /// the read, so the bytes after that field don't need to be valid.
    ///
    /// Per H.265 §7.3.2.2 SPS syntax + §7.3.3 PTL syntax with
    /// `max_sub_layers_minus1 = 0` (no sublayer fields).
    fn h265_sps_with_bit_depth_luma_minus8(bit_depth_luma_minus8: u32) -> Vec<u8> {
        let mut bw = BitWriter::new();

        // §7.3.2.2 SPS header.
        bw.write(0, 4); // sps_video_parameter_set_id
        bw.write(0, 3); // sps_max_sub_layers_minus1 = 0
        bw.write(0, 1); // sps_temporal_id_nesting_flag

        // §7.3.3 profile_tier_level(max_sub_layers_minus1 = 0): 96 bits.
        bw.write(0, 2); // general_profile_space
        bw.write(0, 1); // general_tier_flag
        bw.write(1, 5); // general_profile_idc = 1 (Main)
        bw.write(0, 32); // general_profile_compatibility_flags
        bw.write(0, 32); // 32 of the 48 constraint/reserved bits
        bw.write(0, 16); // remaining 16 of the 48 constraint/reserved bits
        bw.write(120, 8); // general_level_idc = 120 (Level 4.0)

        // §7.3.2.2 continues.
        bw.write_ue(0); // sps_seq_parameter_set_id
        bw.write_ue(1); // chroma_format_idc = 1 (4:2:0)
        // separate_colour_plane_flag not coded (chroma_format_idc != 3).
        bw.write_ue(320); // pic_width_in_luma_samples
        bw.write_ue(240); // pic_height_in_luma_samples
        bw.write(0, 1); // conformance_window_flag = 0
        bw.write_ue(bit_depth_luma_minus8); // bit_depth_luma_minus8

        bw.bytes
    }

    /// Per H.265 §7.4.3.2.1, `bit_depth_luma_minus8 ∈ 0..=8` (bit_depth
    /// ∈ 8..=16). ffmpeg's `libavcodec/hevc/ps.c:366-369` clamps at 14
    /// (minus8 ≤ 6); we adopt the same threshold. A fuzzed value of 248
    /// would have silently wrapped to `bit_depth_luma = 8` via
    /// `8 + (248 as u8)` — caught now via `validate_bit_depth_minus8`.
    #[test]
    fn h265_sps_rejects_bit_depth_overflow() {
        let rbsp = h265_sps_with_bit_depth_luma_minus8(248);
        let result = parse_sps(&rbsp);
        assert!(
            matches!(
                result,
                Err(ParseError::ReservedValue {
                    field: "bit_depth_luma_minus8",
                    value: 248
                })
            ),
            "expected ReservedValue, got {result:?}"
        );
    }
}

#[cfg(test)]
mod pps_tests {
    use super::*;

    const PPS_1080P_MAIN40: &[u8] =
        include_bytes!("../../../tests/fixtures/codec/h265/h265_1080p_main40_pps.bin");

    #[test]
    fn parse_pps_basics() {
        let pps = parse_pps(PPS_1080P_MAIN40).expect("parse PPS");
        assert_eq!(pps.pps_pic_parameter_set_id, 0);
        assert_eq!(pps.pps_seq_parameter_set_id, 0);
    }

    #[test]
    fn parse_pps_returns_err_on_empty() {
        assert!(parse_pps(&[]).is_err());
    }
}

#[cfg(test)]
mod combined_tests {
    use super::*;
    use crate::mpegts::demux::event::NalUnit;

    fn nal(nt: u8, payload: Vec<u8>) -> NalUnit {
        NalUnit::H265 {
            nal_type: nt,
            layer_id: 0,
            temporal_id_plus1: 1,
            payload,
        }
    }

    const VPS: &[u8] =
        include_bytes!("../../../tests/fixtures/codec/h265/h265_1080p_main40_vps.bin");
    const SPS: &[u8] =
        include_bytes!("../../../tests/fixtures/codec/h265/h265_1080p_main40_sps.bin");
    const PPS: &[u8] =
        include_bytes!("../../../tests/fixtures/codec/h265/h265_1080p_main40_pps.bin");

    #[test]
    fn parse_parameter_sets_collects_all_three() {
        let nals = vec![
            nal(32, VPS.into()),
            nal(33, SPS.into()),
            nal(34, PPS.into()),
        ];
        let ps = parse_parameter_sets(&nals).expect("parse");
        assert_eq!(ps.vps_by_id.len(), 1);
        assert_eq!(ps.sps_by_id.len(), 1);
        assert_eq!(ps.pps_by_id.len(), 1);
        assert_eq!(ps.sps_by_id[&0].width, 1920);
    }

    #[test]
    fn parse_parameter_sets_skips_h264_nals_silently() {
        let nals = vec![
            NalUnit::H264 {
                nal_type: 7,
                ref_idc: 3,
                payload: vec![0; 8],
            },
            nal(33, SPS.into()),
        ];
        let ps = parse_parameter_sets(&nals).expect("parse");
        assert_eq!(ps.sps_by_id.len(), 1);
    }

    #[test]
    fn parse_parameter_sets_empty_returns_ok_empty() {
        let ps = parse_parameter_sets(&[]).expect("parse");
        assert!(ps.vps_by_id.is_empty());
    }

    #[test]
    fn parse_parameter_sets_only_slice_nals_returns_ok_empty() {
        let nals = vec![nal(0, vec![0; 16])];
        let ps = parse_parameter_sets(&nals).expect("parse");
        assert!(ps.sps_by_id.is_empty());
    }

    #[test]
    fn parse_parameter_sets_partial_success_one_bad_vps() {
        let nals = vec![
            nal(32, VPS.into()),
            nal(32, vec![0xff; 8]),
            nal(33, SPS.into()),
        ];
        let ps = parse_parameter_sets(&nals).expect("parse");
        assert_eq!(ps.vps_by_id.len(), 1);
        assert_eq!(ps.sps_by_id.len(), 1);
    }
}
