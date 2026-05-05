//! H.266 PPS parser. Per H.266 V4 §7.3.2.5.

use crate::codec::ParseError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H266Pps {
    pub pps_id: u8,
    pub sps_id: u8,
    pub raw_rbsp: Vec<u8>,
}

pub fn parse_pps(_rbsp: &[u8]) -> Result<H266Pps, ParseError> {
    Err(ParseError::EngineError(
        "parse_pps not yet implemented".into(),
    ))
}
