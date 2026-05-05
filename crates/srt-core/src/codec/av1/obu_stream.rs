//! AV1 OBU stream collector. Per AV1 spec §5.2.
//!
//! Implementation lands in Task 25.

use crate::codec::ParseError;
use crate::codec::av1::frame_header::Av1FrameHeaderLight;
use crate::codec::av1::sequence_header::Av1SequenceHeader;
use crate::mpegts::demux::event::Obu;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Av1ObuStream {
    pub sequence_headers: Vec<Av1SequenceHeader>,
    pub frame_headers: Vec<Av1FrameHeaderLight>,
    pub unparseable: Vec<(u8, ParseError)>,
}

pub fn parse_obu_stream(_obus: &[Obu]) -> Av1ObuStream {
    Av1ObuStream {
        sequence_headers: Vec::new(),
        frame_headers: Vec::new(),
        unparseable: Vec::new(),
    }
}
