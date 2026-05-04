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
