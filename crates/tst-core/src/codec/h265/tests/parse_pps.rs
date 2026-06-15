//! PPS parser tests.

use crate::codec::CodecParseError;
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

// rbsp [0x02,0x0C]: pps_pic = ue(64), pps_seq = ue(0).
#[test]
fn rejects_pps_pic_id_above_63() {
    let err = parse_pps(&[0x02, 0x0C]).unwrap_err();
    assert!(matches!(
        err,
        CodecParseError::ReservedValue {
            field: "pps_pic_parameter_set_id",
            value: 64
        }
    ));
}

// rbsp [0x84,0x40]: pps_pic = ue(0), pps_seq = ue(16).
#[test]
fn rejects_pps_seq_id_above_15() {
    let err = parse_pps(&[0x84, 0x40]).unwrap_err();
    assert!(matches!(
        err,
        CodecParseError::ReservedValue {
            field: "pps_seq_parameter_set_id",
            value: 16
        }
    ));
}

// rbsp [0xC0]: pps_pic = ue(0), pps_seq = ue(0).
#[test]
fn accepts_conformant_ids() {
    let pps = parse_pps(&[0xC0]).unwrap();
    assert_eq!(pps.pps_pic_parameter_set_id, 0);
    assert_eq!(pps.pps_seq_parameter_set_id, 0);
}
