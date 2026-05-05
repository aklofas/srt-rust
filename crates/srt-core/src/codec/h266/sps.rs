//! H.266 SPS parser. Per H.266 V4 §7.3.2.4.

use crate::codec::h266::profile_tier_level::H266ProfileTierLevel;
use crate::codec::{ChromaFormat, ColorInfo, ParseError, Rational};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct H266Sps {
    pub sps_id: u8,
    pub vps_id: u8,
    pub profile_tier_level: H266ProfileTierLevel,
    pub width: u32,
    pub height: u32,
    pub chroma_format: ChromaFormat,
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    pub color_info: Option<ColorInfo>,
    pub frame_rate: Option<Rational>,
    pub raw_rbsp: Vec<u8>,
}

pub fn parse_sps(_rbsp: &[u8]) -> Result<H266Sps, ParseError> {
    Err(ParseError::EngineError(
        "parse_sps not yet implemented".into(),
    ))
}
