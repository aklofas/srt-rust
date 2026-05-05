//! H.266 VPS parser. Per H.266 V4 §7.3.2.3.

use crate::codec::ParseError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H266Vps {
    pub vps_id: u8,
    pub max_layers: u8,
    pub max_sub_layers: u8,
    pub raw_rbsp: Vec<u8>,
}

pub fn parse_vps(_rbsp: &[u8]) -> Result<H266Vps, ParseError> {
    Err(ParseError::EngineError(
        "parse_vps not yet implemented".into(),
    ))
}
