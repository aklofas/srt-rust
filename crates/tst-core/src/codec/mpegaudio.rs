//! MPEG-1 / MPEG-2 / MPEG-2.5 audio frame iterator.
//!
//! Spec: ISO/IEC 11172-3 (MPEG-1 Audio) §2.4 + ISO/IEC 13818-3 (MPEG-2
//! Lower Sampling Frequencies, "LSF") + the de-facto MPEG-2.5 half-rate
//! extension. Covers Layer I, II, and III in one module per the spec
//! scope.
//!
//! See [`frames`] for the iterator entry point.

use crate::codec::CodecParseError;

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

    /// Promote this borrowed frame to a [`FrameOwned`] by copying the body.
    pub fn to_owned(&self) -> FrameOwned {
        FrameOwned {
            layer: self.layer,
            version: self.version,
            bitrate_kbps: self.bitrate_kbps,
            sample_rate_hz: self.sample_rate_hz,
            channel_mode: self.channel_mode,
            channels: self.channels,
            frame_length_bytes: self.frame_length_bytes,
            samples_per_frame: self.samples_per_frame,
            has_crc: self.has_crc,
            raw_header: self.raw_header,
            body: self.body.to_vec(),
        }
    }
}

/// Owned variant of [`Frame`].
///
/// `body: Vec<u8>` instead of `&'a [u8]` — usable across FFI / thread /
/// async boundaries where the borrowed source slice doesn't outlive the
/// consumer.
///
/// Round-trip with [`Frame::to_owned`] / [`Self::as_ref`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FrameOwned {
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
    pub body: Vec<u8>,
}

impl FrameOwned {
    /// Borrow this owned frame as a [`Frame`] — zero-copy.
    pub fn as_ref(&self) -> Frame<'_> {
        Frame {
            layer: self.layer,
            version: self.version,
            bitrate_kbps: self.bitrate_kbps,
            sample_rate_hz: self.sample_rate_hz,
            channel_mode: self.channel_mode,
            channels: self.channels,
            frame_length_bytes: self.frame_length_bytes,
            samples_per_frame: self.samples_per_frame,
            has_crc: self.has_crc,
            raw_header: self.raw_header,
            body: &self.body,
        }
    }
}

/// Iterator over MPEG audio frames in `bytes`. Use [`frames`] to construct.
pub struct Frames<'a> {
    buf: &'a [u8],
    cursor: usize,
    done: bool,
}

impl<'a> Iterator for Frames<'a> {
    type Item = Result<Frame<'a>, CodecParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if self.cursor >= self.buf.len() {
            self.done = true;
            return None;
        }
        let remaining = &self.buf[self.cursor..];
        let header = match parse_header(remaining) {
            Ok(h) => h,
            Err(e) => {
                self.done = true;
                return Some(Err(e));
            }
        };
        let len = header.frame_length_bytes as usize;
        if remaining.len() < len {
            self.done = true;
            return Some(Err(CodecParseError::Truncated {
                needed: header.frame_length_bytes,
                had: remaining.len() as u32,
            }));
        }
        let body = &remaining[..len];
        let frame = Frame {
            layer: header.layer,
            version: header.version,
            bitrate_kbps: header.bitrate_kbps,
            sample_rate_hz: header.sample_rate_hz,
            channel_mode: header.channel_mode,
            channels: header.channels,
            frame_length_bytes: header.frame_length_bytes,
            samples_per_frame: header.samples_per_frame,
            has_crc: header.has_crc,
            raw_header: header.raw,
            body,
        };
        self.cursor += len;
        Some(Ok(frame))
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
const BITRATE_TABLE: [[u32; 16]; 5] = [
    // index: 0   1   2   3   4    5    6    7    8    9    10   11   12   13   14   15
    [
        0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0,
    ], // V1L1
    [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0,
    ], // V1L2
    [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ], // V1L3
    [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0,
    ], // V2L1
    [
        0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
    ], // V2L2/L3
];

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
pub(crate) fn decode_bitrate(
    version: Version,
    layer: Layer,
    bitrate_index: u8,
) -> Result<u32, CodecParseError> {
    if bitrate_index == 0 {
        return Err(CodecParseError::ReservedValue {
            field: "bitrate_index",
            value: 0,
        });
    }
    if bitrate_index == 15 {
        return Err(CodecParseError::Forbidden {
            field: "bitrate_index",
        });
    }
    let col = bitrate_column(version, layer);
    Ok(BITRATE_TABLE[col][bitrate_index as usize])
}

// Sample rate table per ISO 11172-3 §2.4.2.3 Table 9 + ISO 13818-3 Table 6.
// Indexed by [version][sample_rate_index]. Index 3 = reserved.
const SAMPLE_RATE_TABLE: [[u32; 4]; 3] = [
    [44100, 48000, 32000, 0], // MPEG-1
    [22050, 24000, 16000, 0], // MPEG-2
    [11025, 12000, 8000, 0],  // MPEG-2.5
];

/// Decode sample rate (Hz) from `(version, sample_rate_index)`.
///
/// Errors: `ReservedValue { field: "sample_rate_index", value: 3 }`.
pub(crate) fn decode_sample_rate(
    version: Version,
    sample_rate_index: u8,
) -> Result<u32, CodecParseError> {
    if sample_rate_index == 3 {
        return Err(CodecParseError::ReservedValue {
            field: "sample_rate_index",
            value: 3,
        });
    }
    let row = match version {
        Version::Mpeg1 => 0,
        Version::Mpeg2 => 1,
        Version::Mpeg2_5 => 2,
    };
    Ok(SAMPLE_RATE_TABLE[row][sample_rate_index as usize])
}

/// Decode channel mode from the 2-bit header field (bits 25-26).
fn decode_channel_mode(bits: u8) -> ChannelMode {
    match bits & 0b11 {
        0b00 => ChannelMode::Stereo,
        0b01 => ChannelMode::JointStereo,
        0b10 => ChannelMode::DualChannel,
        0b11 => ChannelMode::Mono,
        _ => unreachable!(),
    }
}

/// Return the number of channels for a given channel mode.
fn channels_for_mode(mode: ChannelMode) -> u8 {
    match mode {
        ChannelMode::Mono => 1,
        ChannelMode::Stereo | ChannelMode::JointStereo | ChannelMode::DualChannel => 2,
    }
}

/// Return the number of samples per frame for a given (version, layer) pair.
/// Per ISO 11172-3 (MPEG-1) + ISO 13818-3 (MPEG-2/2.5).
fn samples_per_frame(version: Version, layer: Layer) -> u16 {
    match (version, layer) {
        (_, Layer::I) => 384,
        (_, Layer::II) => 1152,
        (Version::Mpeg1, Layer::III) => 1152,
        (Version::Mpeg2, Layer::III) | (Version::Mpeg2_5, Layer::III) => 576,
    }
}

/// Compute frame length in bytes per spec.
///
/// Formulas:
/// - Layer I: `(12 * bitrate * 1000 / sample_rate + padding) * 4`
/// - Layer II: `144 * bitrate * 1000 / sample_rate + padding`
/// - Layer III, MPEG-1: `144 * bitrate * 1000 / sample_rate + padding`
/// - Layer III, MPEG-2/2.5: `72 * bitrate * 1000 / sample_rate + padding`
///
/// `bitrate` is in kbps; `sample_rate` is in Hz.
fn frame_length(
    layer: Layer,
    version: Version,
    bitrate_kbps: u32,
    sample_rate_hz: u32,
    padding: bool,
) -> u32 {
    let pad = if padding { 1 } else { 0 };
    let bitrate_bps = bitrate_kbps * 1000;
    match layer {
        Layer::I => (12 * bitrate_bps / sample_rate_hz + pad) * 4,
        Layer::II => 144 * bitrate_bps / sample_rate_hz + pad,
        Layer::III => match version {
            Version::Mpeg1 => 144 * bitrate_bps / sample_rate_hz + pad,
            Version::Mpeg2 | Version::Mpeg2_5 => 72 * bitrate_bps / sample_rate_hz + pad,
        },
    }
}

/// Decoded view of the 4-byte header (no body slice yet).
#[derive(Debug)]
struct Header {
    version: Version,
    layer: Layer,
    bitrate_kbps: u32,
    sample_rate_hz: u32,
    channel_mode: ChannelMode,
    channels: u8,
    frame_length_bytes: u32,
    samples_per_frame: u16,
    has_crc: bool,
    raw: [u8; 4],
}

/// Decode the 4-byte MPEG audio header.
///
/// Field layout (MSB first):
///   syncword:           bits  0..11 (12 bits, must be 0xFFE or 0xFFF)
///   version_id:         bits 11..13 (2 bits)
///       0b00 = MPEG-2.5; 0b01 = reserved; 0b10 = MPEG-2; 0b11 = MPEG-1
///   layer:              bits 13..15 (2 bits)
///       0b00 = reserved; 0b01 = Layer III; 0b10 = Layer II; 0b11 = Layer I
///   protection_bit:     bit  15 (1 bit)
///       0 = CRC follows; 1 = no CRC
///   bitrate_index:      bits 16..20 (4 bits)
///   sample_rate_index:  bits 20..22 (2 bits)
///   padding_bit:        bit  22 (1 bit)
///   private_bit:        bit  23 (1 bit)
///   channel_mode:       bits 24..26 (2 bits)
fn parse_header(bytes: &[u8]) -> Result<Header, CodecParseError> {
    if bytes.len() < 4 {
        return Err(CodecParseError::Truncated {
            needed: 4,
            had: bytes.len() as u32,
        });
    }
    let raw = [bytes[0], bytes[1], bytes[2], bytes[3]];
    let h: u32 = ((raw[0] as u32) << 24)
        | ((raw[1] as u32) << 16)
        | ((raw[2] as u32) << 8)
        | (raw[3] as u32);

    // Sync word: top 11 bits must be 0x7FF (frame sync). We validate the
    // 12-bit form here; the 12th bit (next: version_id MSB) is allowed to
    // be 0 (MPEG-2.5) or 1.
    let sync = (h >> 21) & 0x7FF;
    if sync != 0x7FF {
        return Err(CodecParseError::BadSyncWord {
            expected: 0x7FF,
            found: sync as u16,
        });
    }

    let version_id = ((h >> 19) & 0b11) as u8;
    let version = match version_id {
        0b00 => Version::Mpeg2_5,
        0b01 => {
            return Err(CodecParseError::ReservedValue {
                field: "version_id",
                value: 0b01,
            });
        }
        0b10 => Version::Mpeg2,
        0b11 => Version::Mpeg1,
        _ => unreachable!(),
    };

    let layer_id = ((h >> 17) & 0b11) as u8;
    let layer = match layer_id {
        0b00 => {
            return Err(CodecParseError::ReservedValue {
                field: "layer",
                value: 0,
            });
        }
        0b01 => Layer::III,
        0b10 => Layer::II,
        0b11 => Layer::I,
        _ => unreachable!(),
    };

    let protection_bit = ((h >> 16) & 1) as u8;
    let has_crc = protection_bit == 0;

    let bitrate_index = ((h >> 12) & 0b1111) as u8;
    let bitrate_kbps = decode_bitrate(version, layer, bitrate_index)?;

    let sample_rate_index = ((h >> 10) & 0b11) as u8;
    let sample_rate_hz = decode_sample_rate(version, sample_rate_index)?;

    let padding = ((h >> 9) & 1) != 0;

    let channel_mode_bits = ((h >> 6) & 0b11) as u8;
    let channel_mode = decode_channel_mode(channel_mode_bits);
    let channels = channels_for_mode(channel_mode);

    let frame_length_bytes = frame_length(layer, version, bitrate_kbps, sample_rate_hz, padding);
    let samples_per_frame = samples_per_frame(version, layer);

    Ok(Header {
        version,
        layer,
        bitrate_kbps,
        sample_rate_hz,
        channel_mode,
        channels,
        frame_length_bytes,
        samples_per_frame,
        has_crc,
        raw,
    })
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
        assert!(
            matches!(err, CodecParseError::ReservedValue { field, value: 0 } if field == "bitrate_index")
        );
    }
    #[test]
    fn bitrate_index15_is_forbidden() {
        let err = decode_bitrate(Version::Mpeg1, Layer::I, 15).unwrap_err();
        assert!(matches!(err, CodecParseError::Forbidden { field } if field == "bitrate_index"));
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
        assert!(
            matches!(err, CodecParseError::ReservedValue { field, value: 3 } if field == "sample_rate_index")
        );
    }
    #[test]
    fn channel_mode_decoded() {
        assert_eq!(decode_channel_mode(0b00), ChannelMode::Stereo);
        assert_eq!(decode_channel_mode(0b01), ChannelMode::JointStereo);
        assert_eq!(decode_channel_mode(0b10), ChannelMode::DualChannel);
        assert_eq!(decode_channel_mode(0b11), ChannelMode::Mono);
    }
    #[test]
    fn channels_count_per_mode() {
        assert_eq!(channels_for_mode(ChannelMode::Stereo), 2);
        assert_eq!(channels_for_mode(ChannelMode::JointStereo), 2);
        assert_eq!(channels_for_mode(ChannelMode::DualChannel), 2);
        assert_eq!(channels_for_mode(ChannelMode::Mono), 1);
    }
    #[test]
    fn samples_per_frame_layer1() {
        assert_eq!(samples_per_frame(Version::Mpeg1, Layer::I), 384);
        assert_eq!(samples_per_frame(Version::Mpeg2, Layer::I), 384);
    }
    #[test]
    fn samples_per_frame_layer2() {
        assert_eq!(samples_per_frame(Version::Mpeg1, Layer::II), 1152);
        assert_eq!(samples_per_frame(Version::Mpeg2, Layer::II), 1152);
    }
    #[test]
    fn samples_per_frame_layer3_v1_is_1152_v2_is_576() {
        assert_eq!(samples_per_frame(Version::Mpeg1, Layer::III), 1152);
        assert_eq!(samples_per_frame(Version::Mpeg2, Layer::III), 576);
        assert_eq!(samples_per_frame(Version::Mpeg2_5, Layer::III), 576);
    }

    #[test]
    fn frame_length_v1l1_128kbps_44100_no_padding_is_136() {
        // Layer I frame length: (12 * bitrate / sample_rate + padding) * 4
        // (12 * 128000 / 44100 + 0) * 4 = 34*4 = 136 bytes.
        assert_eq!(
            frame_length(Layer::I, Version::Mpeg1, 128, 44100, false),
            136
        );
    }
    #[test]
    fn frame_length_v1l1_padding_adds_4_bytes() {
        assert_eq!(
            frame_length(Layer::I, Version::Mpeg1, 128, 44100, true),
            140
        );
    }
    #[test]
    fn frame_length_v1l3_128kbps_44100_no_padding_is_417() {
        // Layer III frame length: 144 * bitrate / sample_rate + padding
        // 144 * 128000 / 44100 = 417 (truncated)
        assert_eq!(
            frame_length(Layer::III, Version::Mpeg1, 128, 44100, false),
            417
        );
    }
    #[test]
    fn frame_length_v1l3_padding_adds_1_byte() {
        assert_eq!(
            frame_length(Layer::III, Version::Mpeg1, 128, 44100, true),
            418
        );
    }
    #[test]
    fn frame_length_v2l3_64kbps_22050_is_208() {
        // V2 Layer III: 72 * 64000 / 22050 = 208 (truncated)
        assert_eq!(
            frame_length(Layer::III, Version::Mpeg2, 64, 22050, false),
            208
        );
    }

    /// V1 Layer III, 128 kbps, 44.1 kHz, joint stereo, no padding, no CRC.
    /// Header bits:
    ///   syncword:        0xFFF
    ///   version_id:      0b11 (MPEG-1)
    ///   layer_desc:      0b01 (Layer III; bits inverted: 01 = III)
    ///   protection_bit:  1 (no CRC)
    ///   bitrate_index:   0b1001 (9 = 128 kbps)
    ///   sample_rate_idx: 0b00 (44100 Hz)
    ///   padding:         0
    ///   private:         0
    ///   channel_mode:    0b01 (joint stereo) — byte 3 bits 7-6
    ///   mode_ext:        0b00
    ///   copyright:       0
    ///   original:        0
    ///   emphasis:        0b00
    /// Byte layout: FF  FB  90  40
    ///              [0] [1] [2] [3]
    const V1L3_128K_44100_JS: [u8; 4] = [0xFF, 0xFB, 0x90, 0x40];

    #[test]
    fn parse_header_v1l3_128k_44100_js() {
        let h = parse_header(&V1L3_128K_44100_JS).unwrap();
        assert_eq!(h.version, Version::Mpeg1);
        assert_eq!(h.layer, Layer::III);
        assert_eq!(h.bitrate_kbps, 128);
        assert_eq!(h.sample_rate_hz, 44100);
        assert_eq!(h.channel_mode, ChannelMode::JointStereo);
        assert_eq!(h.channels, 2);
        assert!(!h.has_crc);
        assert_eq!(h.frame_length_bytes, 417);
        assert_eq!(h.samples_per_frame, 1152);
    }

    #[test]
    fn parse_header_too_short_yields_truncated() {
        let err = parse_header(&[0xFF, 0xFB]).unwrap_err();
        assert!(matches!(
            err,
            CodecParseError::Truncated { needed: 4, had: 2 }
        ));
    }

    #[test]
    fn parse_header_bad_sync_yields_bad_sync_word() {
        let err = parse_header(&[0xAB, 0xCD, 0x00, 0x00]).unwrap_err();
        assert!(matches!(err, CodecParseError::BadSyncWord { .. }));
    }

    #[test]
    fn frames_empty_yields_none() {
        let mut it = frames(&[]);
        assert!(it.next().is_none());
    }

    #[test]
    fn frames_short_yields_truncated_then_none() {
        let mut it = frames(&[0xFF]);
        match it.next() {
            Some(Err(CodecParseError::Truncated { .. })) => {}
            other => panic!("expected Err(Truncated), got {:?}", other),
        }
        assert!(it.next().is_none(), "iterator should end after first error");
    }

    #[test]
    fn frames_two_back_to_back() {
        // Build two V1L3 128k 44.1k frames (417 bytes each, no padding).
        let header: [u8; 4] = [0xFF, 0xFB, 0x90, 0x40];
        let mut buf = Vec::with_capacity(2 * 417);
        for _ in 0..2 {
            buf.extend_from_slice(&header);
            buf.extend(std::iter::repeat(0u8).take(417 - 4));
        }
        let mut it = frames(&buf);
        let f1 = it.next().unwrap().unwrap();
        assert_eq!(f1.frame_length_bytes, 417);
        assert_eq!(f1.bytes().len(), 417);
        let f2 = it.next().unwrap().unwrap();
        assert_eq!(f2.frame_length_bytes, 417);
        assert!(it.next().is_none());
    }

    #[test]
    fn frames_truncated_body_yields_truncated() {
        // 4-byte header says 417 but we only give 100 bytes total.
        let header: [u8; 4] = [0xFF, 0xFB, 0x90, 0x40];
        let mut buf = Vec::from(header);
        buf.extend(std::iter::repeat(0u8).take(96));
        let mut it = frames(&buf);
        match it.next() {
            Some(Err(CodecParseError::Truncated { .. })) => {}
            other => panic!("expected Err(Truncated), got {:?}", other),
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn mpegaudio_frame_owned_roundtrip() {
        let payload = vec![0xFF, 0xFB, 0x90, 0x40, 0x01, 0x02, 0x03];
        let borrowed = Frame {
            layer: Layer::III,
            version: Version::Mpeg1,
            bitrate_kbps: 128,
            sample_rate_hz: 44100,
            channel_mode: ChannelMode::JointStereo,
            channels: 2,
            frame_length_bytes: 417,
            samples_per_frame: 1152,
            has_crc: false,
            raw_header: [0xFF, 0xFB, 0x90, 0x40],
            body: &payload,
        };
        let owned = borrowed.to_owned();
        assert_eq!(borrowed, owned.as_ref());
        // Verify the body was actually copied into owned storage.
        assert_eq!(owned.body, payload);
    }
}
