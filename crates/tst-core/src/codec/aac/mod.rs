//! AAC frame iterator (ADTS framing today; LATM is a follow-up plan).
//!
//! See [`crate::codec`] for umbrella architecture and design rationale.
//!
//! ## Spec coverage
//!
//! Parsed per ISO/IEC 13818-7 §1.A (ADTS) over MPEG-2 / MPEG-4 AAC:
//! - ADTS sync word, MPEG version, layer validation.
//! - Per-frame: profile, sample_rate, channel_configuration, channels,
//!   frame_length, samples_per_frame, num_raw_data_blocks, has_crc.
//!
//! ## Not parsed (deferred)
//!
//! - LATM/LOAS framing (deferred — separate plan).
//! - AudioSpecificConfig / SBR / PS extension headers inside raw data blocks.
//! - MPEG-4 audio object types beyond the 4 legacy ADTS profiles.

mod adts;
mod tables;
#[cfg(test)]
mod tests;

use crate::codec::CodecParseError;

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
#[non_exhaustive]
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
    pub(super) body: &'a [u8],
}

impl<'a> AdtsFrame<'a> {
    /// Full-frame slice (header + body). `raw_header` is the first 7
    /// (no CRC) or 9 (with CRC) bytes of this slice copied for
    /// ownership convenience.
    pub fn bytes(&self) -> &'a [u8] {
        self.body
    }

    /// Promote this borrowed frame to an [`AdtsFrameOwned`] by copying `body`.
    pub fn to_owned(&self) -> AdtsFrameOwned {
        AdtsFrameOwned {
            profile: self.profile,
            sample_rate_hz: self.sample_rate_hz,
            channel_configuration: self.channel_configuration,
            channels: self.channels,
            frame_length_bytes: self.frame_length_bytes,
            samples_per_frame: self.samples_per_frame,
            num_raw_data_blocks: self.num_raw_data_blocks,
            has_crc: self.has_crc,
            mpeg_version: self.mpeg_version,
            raw_header: self.raw_header.clone(),
            body: self.body.to_vec(),
        }
    }
}

/// Owned variant of [`AdtsFrame`].
///
/// `body: Vec<u8>` instead of `&'a [u8]` — usable across FFI / thread / async
/// boundaries where the borrowed source slice doesn't outlive the consumer.
///
/// Round-trip with [`AdtsFrame::to_owned`] / [`Self::as_ref`].
///
/// # Example — collect owned frames from a borrowed iterator
/// ```
/// use tst_core::codec::aac::{frames, AdtsFrameOwned};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let payload: &[u8] = &[/* ADTS-framed AAC bytes */];
/// # let payload: &[u8] = &[];
/// let owned: Vec<AdtsFrameOwned> = frames(payload)
///     .filter_map(Result::ok)
///     .map(|f| f.to_owned())
///     .collect();
/// // `owned` outlives `payload` — safe to ship over FFI.
/// let _ = owned;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdtsFrameOwned {
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
    pub body: Vec<u8>,
}

impl AdtsFrameOwned {
    /// Borrow this owned frame as an [`AdtsFrame`] — zero-copy.
    pub fn as_ref(&self) -> AdtsFrame<'_> {
        AdtsFrame {
            profile: self.profile,
            sample_rate_hz: self.sample_rate_hz,
            channel_configuration: self.channel_configuration,
            channels: self.channels,
            frame_length_bytes: self.frame_length_bytes,
            samples_per_frame: self.samples_per_frame,
            num_raw_data_blocks: self.num_raw_data_blocks,
            has_crc: self.has_crc,
            mpeg_version: self.mpeg_version,
            raw_header: self.raw_header.clone(),
            body: &self.body,
        }
    }
}

/// Iterator over ADTS frames in `bytes`. Use [`frames`] to construct.
#[must_use]
pub struct AdtsFrames<'a> {
    pub(super) buf: &'a [u8],
    pub(super) cursor: usize,
    pub(super) done: bool,
}

impl<'a> Iterator for AdtsFrames<'a> {
    type Item = Result<AdtsFrame<'a>, CodecParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if self.cursor >= self.buf.len() {
            self.done = true;
            return None;
        }
        let remaining = &self.buf[self.cursor..];
        let header = match adts::parse_header(remaining) {
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
        let raw_header = body[..header.raw_header_len].to_vec();
        let frame = AdtsFrame {
            profile: header.profile,
            sample_rate_hz: header.sample_rate_hz,
            channel_configuration: header.channel_configuration,
            channels: header.channels,
            frame_length_bytes: header.frame_length_bytes,
            samples_per_frame: header.samples_per_frame,
            num_raw_data_blocks: header.num_raw_data_blocks,
            has_crc: header.has_crc,
            mpeg_version: header.mpeg_version,
            raw_header,
            body,
        };
        self.cursor += len;
        Some(Ok(frame))
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
