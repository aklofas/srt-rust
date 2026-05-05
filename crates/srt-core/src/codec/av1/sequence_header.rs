//! AV1 Sequence Header OBU parser. Per AV1 spec §5.5.1.
//!
//! Implementation lands in Task 23.

use crate::codec::ParseError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Av1SequenceHeader {
    pub profile: u8,
    // Stub-fill expanded in Task 23.
}

pub fn parse_sequence_header(_bytes: &[u8]) -> Result<Av1SequenceHeader, ParseError> {
    Err(ParseError::EngineError(
        "parse_sequence_header not yet implemented".into(),
    ))
}
