//! MPEG-1 / MPEG-2 / MPEG-2.5 audio frame iterator.
//!
//! Spec: ISO/IEC 11172-3 (MPEG-1 Audio) §2.4 + ISO/IEC 13818-3 (MPEG-2
//! Lower Sampling Frequencies, "LSF") + the de-facto MPEG-2.5 half-rate
//! extension. Covers Layer I, II, and III in one module per the spec
//! scope.
//!
//! See [`frames`] for the iterator entry point.

use crate::codec::ParseError;

/// MPEG audio layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    I,
    II,
    III,
}

/// MPEG audio version. MPEG-2.5 is the de-facto half-rate extension
/// (8 / 11.025 / 12 kHz Layer III); not part of any ratified ISO spec
/// but ubiquitous in consumer MP3 streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    Mpeg1,
    Mpeg2,
    Mpeg2_5,
}

/// MPEG audio channel mode (header bits 25-26).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMode {
    Stereo,
    JointStereo,
    DualChannel,
    Mono,
}

/// Decoded MPEG audio frame. Borrows from the source buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame<'a> {
    pub layer: Layer,
    pub version: Version,
    pub bitrate_kbps: u32,
    pub sample_rate_hz: u32,
    pub channel_mode: ChannelMode,
    pub channels: u8,
    pub frame_length_bytes: u32,
    pub samples_per_frame: u16,
    pub has_crc: bool,
    pub raw_header: [u8; 4],
    body: &'a [u8],
}

impl<'a> Frame<'a> {
    /// Full-frame slice (header + body, including CRC bytes when
    /// `has_crc`). The `raw_header` field is the first 4 bytes of this
    /// slice copied for ownership convenience.
    pub fn bytes(&self) -> &'a [u8] {
        self.body
    }
}

/// Iterator over MPEG audio frames in `bytes`. Use [`frames`] to construct.
#[allow(dead_code)] // buf + cursor read once Frames::next is implemented (Task 8)
pub struct Frames<'a> {
    buf: &'a [u8],
    cursor: usize,
    done: bool,
}

impl<'a> Iterator for Frames<'a> {
    type Item = Result<Frame<'a>, ParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        // Implementation lands in Task 8.
        todo!("mpegaudio::Frames::next not yet implemented")
    }
}

/// Construct a frame iterator over an MPEG audio elementary stream
/// (PES payload bytes).
pub fn frames(bytes: &[u8]) -> Frames<'_> {
    Frames {
        buf: bytes,
        cursor: 0,
        done: false,
    }
}
