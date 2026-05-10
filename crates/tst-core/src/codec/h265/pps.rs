//! PPS parser per H.265 §7.3.2.3. Only `pps_pic_parameter_set_id` and
//! `pps_seq_parameter_set_id` are exposed; everything else is
//! decoder-internal.

use crate::codec::CodecParseError;
use crate::codec::bitreader::BitReader;

#[derive(Debug, Clone, PartialEq, Eq)]
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
    let pps_pic_parameter_set_id = br.read_ue()? as u8;
    let pps_seq_parameter_set_id = br.read_ue()? as u8;
    Ok(H265Pps {
        pps_pic_parameter_set_id,
        pps_seq_parameter_set_id,
        raw_rbsp: rbsp.to_vec(),
    })
}
