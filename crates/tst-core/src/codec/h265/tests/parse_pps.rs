//! PPS parser tests.

use crate::codec::h265::parse_pps;

const PPS_1080P_MAIN40: &[u8] =
    include_bytes!("../../../../tests/fixtures/codec/h265/h265_1080p_main40_pps.bin");

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
