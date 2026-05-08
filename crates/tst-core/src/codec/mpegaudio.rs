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

// Bitrate table per ISO 11172-3 §2.4.2.3 Table 8 + ISO 13818-3 Table 5.
// Indexed by [column][bitrate_index]. Column selection is by
// (version, layer); see `bitrate_column` below. Index 0 = free format
// (rejected); index 15 = forbidden.
//
// Columns:
//   0 = MPEG-1 Layer I
//   1 = MPEG-1 Layer II
//   2 = MPEG-1 Layer III
//   3 = MPEG-2/2.5 Layer I
//   4 = MPEG-2/2.5 Layer II/III (shared column per ISO 13818-3 Table 5)
#[allow(dead_code)]
const BITRATE_TABLE: [[u32; 16]; 5] = [
    // index: 0   1   2   3   4    5    6    7    8    9    10   11   12   13   14   15
    [        0,  32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0], // V1L1
    [        0,  32, 48, 56, 64,  80,  96,  112, 128, 160, 192, 224, 256, 320, 384, 0], // V1L2
    [        0,  32, 40, 48, 56,  64,  80,  96,  112, 128, 160, 192, 224, 256, 320, 0], // V1L3
    [        0,  32, 48, 56, 64,  80,  96,  112, 128, 144, 160, 176, 192, 224, 256, 0], // V2L1
    [        0,  8,  16, 24, 32,  40,  48,  56,  64,  80,  96,  112, 128, 144, 160, 0], // V2L2/L3
];

#[allow(dead_code)]
fn bitrate_column(version: Version, layer: Layer) -> usize {
    match (version, layer) {
        (Version::Mpeg1, Layer::I) => 0,
        (Version::Mpeg1, Layer::II) => 1,
        (Version::Mpeg1, Layer::III) => 2,
        (_, Layer::I) => 3,
        (_, _) => 4, // V2/V2.5 Layer II + Layer III share column 4
    }
}

/// Decode bitrate (kbps) from `(version, layer, bitrate_index)` per
/// ISO 11172-3 §2.4.2.3 Table 8 + ISO 13818-3 Table 5.
///
/// Errors:
/// - `ReservedValue { field: "bitrate_index", value: 0 }` for free-format
/// - `Forbidden { field: "bitrate_index" }` for index 15
#[allow(dead_code)]
pub(crate) fn decode_bitrate(version: Version, layer: Layer, bitrate_index: u8) -> Result<u32, ParseError> {
    if bitrate_index == 0 {
        return Err(ParseError::ReservedValue { field: "bitrate_index", value: 0 });
    }
    if bitrate_index == 15 {
        return Err(ParseError::Forbidden { field: "bitrate_index" });
    }
    let col = bitrate_column(version, layer);
    Ok(BITRATE_TABLE[col][bitrate_index as usize])
}

// Sample rate table per ISO 11172-3 §2.4.2.3 Table 9 + ISO 13818-3 Table 6.
// Indexed by [version][sample_rate_index]. Index 3 = reserved.
#[allow(dead_code)]
const SAMPLE_RATE_TABLE: [[u32; 4]; 3] = [
    [44100, 48000, 32000, 0], // MPEG-1
    [22050, 24000, 16000, 0], // MPEG-2
    [11025, 12000, 8000,  0], // MPEG-2.5
];

/// Decode sample rate (Hz) from `(version, sample_rate_index)`.
///
/// Errors: `ReservedValue { field: "sample_rate_index", value: 3 }`.
#[allow(dead_code)]
pub(crate) fn decode_sample_rate(version: Version, sample_rate_index: u8) -> Result<u32, ParseError> {
    if sample_rate_index == 3 {
        return Err(ParseError::ReservedValue { field: "sample_rate_index", value: 3 });
    }
    let row = match version {
        Version::Mpeg1 => 0,
        Version::Mpeg2 => 1,
        Version::Mpeg2_5 => 2,
    };
    Ok(SAMPLE_RATE_TABLE[row][sample_rate_index as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitrate_v1l1_index1_is_32() {
        assert_eq!(decode_bitrate(Version::Mpeg1, Layer::I, 1).unwrap(), 32);
    }
    #[test]
    fn bitrate_v1l3_index9_is_128() {
        assert_eq!(decode_bitrate(Version::Mpeg1, Layer::III, 9).unwrap(), 128);
    }
    #[test]
    fn bitrate_v2l3_index12_is_128() {
        // V2/V2.5 Layer II/III shares a column.
        assert_eq!(decode_bitrate(Version::Mpeg2, Layer::III, 12).unwrap(), 128);
    }
    #[test]
    fn bitrate_index0_is_free_format_rejected() {
        let err = decode_bitrate(Version::Mpeg1, Layer::I, 0).unwrap_err();
        assert!(matches!(err, ParseError::ReservedValue { field, value: 0 } if field == "bitrate_index"));
    }
    #[test]
    fn bitrate_index15_is_forbidden() {
        let err = decode_bitrate(Version::Mpeg1, Layer::I, 15).unwrap_err();
        assert!(matches!(err, ParseError::Forbidden { field } if field == "bitrate_index"));
    }
    #[test]
    fn sample_rate_v1_index0_is_44100() {
        assert_eq!(decode_sample_rate(Version::Mpeg1, 0).unwrap(), 44100);
    }
    #[test]
    fn sample_rate_v2_index0_is_22050() {
        assert_eq!(decode_sample_rate(Version::Mpeg2, 0).unwrap(), 22050);
    }
    #[test]
    fn sample_rate_v25_index0_is_11025() {
        assert_eq!(decode_sample_rate(Version::Mpeg2_5, 0).unwrap(), 11025);
    }
    #[test]
    fn sample_rate_index3_is_reserved() {
        let err = decode_sample_rate(Version::Mpeg1, 3).unwrap_err();
        assert!(matches!(err, ParseError::ReservedValue { field, value: 3 } if field == "sample_rate_index"));
    }
}
