//! PPS parser per H.265 §7.3.2.3. Only `pps_pic_parameter_set_id` and
//! `pps_seq_parameter_set_id` are exposed; everything else is
//! decoder-internal.

use crate::codec::CodecParseError;
use crate::codec::bitreader::BitReader;
use alloc::vec::Vec;

#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct H265Pps {
    pub pps_pic_parameter_set_id: u8,
    pub pps_seq_parameter_set_id: u8,
    pub raw_rbsp: Vec<u8>,
}

pub fn parse_pps(rbsp: &[u8]) -> Result<H265Pps, CodecParseError> {
    if rbsp.is_empty() {
        return Err(CodecParseError::TruncatedRbsp {
            offset_bits: 0,
            needed_bits: 8,
        });
    }
    let mut br = BitReader::new(rbsp);
    let pps_pic_parameter_set_id =
        super::read_ue_max(&mut br, "pps_pic_parameter_set_id", 63)? as u8;
    let pps_seq_parameter_set_id =
        super::read_ue_max(&mut br, "pps_seq_parameter_set_id", 15)? as u8;
    Ok(H265Pps {
        pps_pic_parameter_set_id,
        pps_seq_parameter_set_id,
        raw_rbsp: rbsp.to_vec(),
    })
}

#[cfg(test)]
mod id_bounds_tests {
    use super::parse_pps;
    use crate::codec::CodecParseError;

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
}
