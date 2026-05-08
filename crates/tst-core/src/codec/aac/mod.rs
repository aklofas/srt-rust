//! AAC frame iterator (ADTS framing today; LATM is a follow-up plan).
//!
//! Spec: ISO/IEC 13818-7 §1.A (ADTS) over MPEG-2 / MPEG-4 AAC.
//! Surfaces what the ADTS header says — does not decode audio.
//!
//! See [`frames`] for the iterator entry point.

mod adts;
mod tables;

use crate::codec::ParseError;

/// AAC profile per ADTS §1.A (legacy MPEG-2 AAC profile names; most
/// real-world ADTS encodes AAC-LC regardless of which MPEG-4 audio
/// object type the encoder used).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AacProfile {
    Main,
    Lc,
    Ssr,
    LongTermPrediction,
}

/// ADTS MPEG version bit (one bit; 0 = MPEG-4, 1 = MPEG-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MpegVersion {
    Mpeg2,
    Mpeg4,
}

/// Decoded ADTS frame. Borrows from the source buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdtsFrame<'a> {
    pub profile: AacProfile,
    pub sample_rate_hz: u32,
    pub channel_configuration: u8,
    pub channels: u8,
    pub frame_length_bytes: u32,
    pub samples_per_frame: u16,
    pub num_raw_data_blocks: u8,
    pub has_crc: bool,
    pub mpeg_version: MpegVersion,
    pub raw_header: Vec<u8>,
    body: &'a [u8],
}

impl<'a> AdtsFrame<'a> {
    /// Full-frame slice (header + body). `raw_header` is the first 7
    /// (no CRC) or 9 (with CRC) bytes of this slice copied for
    /// ownership convenience.
    pub fn bytes(&self) -> &'a [u8] {
        self.body
    }
}

/// Iterator over ADTS frames in `bytes`. Use [`frames`] to construct.
pub struct AdtsFrames<'a> {
    buf: &'a [u8],
    cursor: usize,
    done: bool,
}

impl<'a> Iterator for AdtsFrames<'a> {
    type Item = Result<AdtsFrame<'a>, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        // Implementation lands in Task 13.
        todo!("aac::AdtsFrames::next not yet implemented")
    }
}

/// Construct an ADTS frame iterator over an AAC elementary stream
/// (PES payload bytes).
pub fn frames(bytes: &[u8]) -> AdtsFrames<'_> {
    AdtsFrames {
        buf: bytes,
        cursor: 0,
        done: false,
    }
}
