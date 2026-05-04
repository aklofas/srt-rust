//! H.265 / HEVC parameter-set parsers.
//!
//! See [`crate::codec`] for umbrella architecture and design rationale.
//!
//! H.265 parsing is hand-rolled (the `hevc-parser` crate's struct fields
//! are crate-private, and `h265-parser` does not exist); reference
//! sections are noted on each module.

mod bitreader;
mod profile_tier_level;
mod vps;
mod sps;
mod vui;
mod pps;

pub use vps::{H265Vps, parse_vps};
pub use sps::{H265Sps, parse_sps};
pub use pps::{H265Pps, parse_pps};

use std::collections::BTreeMap;
use crate::codec::ParseError;
use crate::mpegts::demux::event::NalUnit;

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
        let NalUnit::H265 { nal_type, payload, .. } = nal else { continue };
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
            "every parameter set NAL in the input failed to parse".into()
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VPS_1080P_MAIN40: &[u8] = include_bytes!(
        "../../../tests/fixtures/codec/h265/h265_1080p_main40_vps.bin"
    );
    const VPS_1080P_MAIN10_50: &[u8] = include_bytes!(
        "../../../tests/fixtures/codec/h265/h265_1080p_main10_50_pq_vps.bin"
    );

    #[test]
    fn parse_vps_1080p_main40_basics() {
        let vps = parse_vps(VPS_1080P_MAIN40).expect("parse VPS");
        assert_eq!(vps.vps_video_parameter_set_id, 0);
        assert_eq!(vps.general_level_idc, 120);  // Level 4.0
        assert!(vps.general_tier_flag);
    }

    #[test]
    fn parse_vps_1080p_main10_50() {
        let vps = parse_vps(VPS_1080P_MAIN10_50).expect("parse VPS");
        assert_eq!(vps.general_level_idc, 150);  // Level 5.0
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
    use crate::codec::{ChromaFormat, ColourPrimaries, MatrixCoefficients,
        TransferCharacteristics};

    const SPS_1080P_MAIN40: &[u8] = include_bytes!(
        "../../../tests/fixtures/codec/h265/h265_1080p_main40_sps.bin"
    );
    const SPS_1080P_MAIN10_50: &[u8] = include_bytes!(
        "../../../tests/fixtures/codec/h265/h265_1080p_main10_50_pq_sps.bin"
    );

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
    fn parse_sps_returns_err_on_garbage() {
        assert!(parse_sps(&[0xff; 16]).is_err());
    }
}

#[cfg(test)]
mod pps_tests {
    use super::*;

    const PPS_1080P_MAIN40: &[u8] = include_bytes!(
        "../../../tests/fixtures/codec/h265/h265_1080p_main40_pps.bin"
    );

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
        NalUnit::H265 { nal_type: nt, layer_id: 0, temporal_id_plus1: 1, payload }
    }

    const VPS: &[u8] = include_bytes!(
        "../../../tests/fixtures/codec/h265/h265_1080p_main40_vps.bin"
    );
    const SPS: &[u8] = include_bytes!(
        "../../../tests/fixtures/codec/h265/h265_1080p_main40_sps.bin"
    );
    const PPS: &[u8] = include_bytes!(
        "../../../tests/fixtures/codec/h265/h265_1080p_main40_pps.bin"
    );

    #[test]
    fn parse_parameter_sets_collects_all_three() {
        let nals = vec![nal(32, VPS.into()), nal(33, SPS.into()), nal(34, PPS.into())];
        let ps = parse_parameter_sets(&nals).expect("parse");
        assert_eq!(ps.vps_by_id.len(), 1);
        assert_eq!(ps.sps_by_id.len(), 1);
        assert_eq!(ps.pps_by_id.len(), 1);
        assert_eq!(ps.sps_by_id[&0].width, 1920);
    }

    #[test]
    fn parse_parameter_sets_skips_h264_nals_silently() {
        let nals = vec![
            NalUnit::H264 { nal_type: 7, ref_idc: 3, payload: vec![0; 8] },
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
        let nals = vec![nal(32, VPS.into()), nal(32, vec![0xff; 8]), nal(33, SPS.into())];
        let ps = parse_parameter_sets(&nals).expect("parse");
        assert_eq!(ps.vps_by_id.len(), 1);
        assert_eq!(ps.sps_by_id.len(), 1);
    }
}
