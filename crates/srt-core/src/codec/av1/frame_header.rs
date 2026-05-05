//! AV1 Frame Header parser (light scope). Per AV1 spec §5.9.
//!
//! Implementation lands in Task 24.

use crate::codec::ParseError;
use crate::codec::av1::sequence_header::Av1SequenceHeader;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Av1FrameHeaderLight {
    pub frame_type: u8,
    pub show_frame: bool,
    pub show_existing_frame: bool,
    pub frame_size: Option<(u32, u32)>,
    pub raw: Vec<u8>,
}

pub fn parse_frame_header_light(
    _payload: &[u8],
    _seq: &Av1SequenceHeader,
) -> Result<Av1FrameHeaderLight, ParseError> {
    Err(ParseError::EngineError(
        "parse_frame_header_light not yet implemented".into(),
    ))
}
