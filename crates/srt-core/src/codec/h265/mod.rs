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

pub use vps::{H265Vps, parse_vps};

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
