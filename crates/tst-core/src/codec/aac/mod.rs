//! AAC frame iterator (ADTS framing today; LATM is a follow-up plan).
//!
//! Spec: ISO/IEC 13818-7 §1.A (ADTS) over MPEG-2 / MPEG-4 AAC.
//! Surfaces what the ADTS header says — does not decode audio.
//!
//! See [`frames`] for the iterator entry point.

mod adts;
mod tables;

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
pub struct AdtsFrames<'a> {
    buf: &'a [u8],
    cursor: usize,
    done: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build full ADTS frame (7-byte header + zero-fill body).
    /// Mirrors the build_header helper in adts.rs but pads to total_len bytes.
    /// Defaults: MPEG-2 ID, no CRC, AAC-LC profile, num_blocks_wire=0 (1 block).
    fn build_frame(sample_rate_index: u8, channel_config: u8, total_len: u32) -> Vec<u8> {
        let mut h = vec![0u8; 7];
        h[0] = 0xFF;
        h[1] = 0b1111_0000 | (1 << 3) | 1; // ID=MPEG-2, layer=0, no CRC
        h[2] = (1 << 6) | ((sample_rate_index & 0xF) << 2) | ((channel_config >> 2) & 1); // LC profile
        h[3] = ((channel_config & 0b11) << 6) | (((total_len >> 11) & 0b11) as u8);
        h[4] = ((total_len >> 3) & 0xFF) as u8;
        h[5] = (((total_len & 0b111) as u8) << 5) | 0b1_1111;
        h[6] = 0b11_1111 << 2;
        let pad = total_len as usize - 7;
        let mut out = h;
        out.extend(std::iter::repeat(0u8).take(pad));
        out
    }

    #[test]
    fn frames_empty_yields_none() {
        assert!(frames(&[]).next().is_none());
    }

    #[test]
    fn frames_two_back_to_back() {
        let mut buf = build_frame(4, 2, 200);
        buf.extend(build_frame(4, 2, 200));
        let mut it = frames(&buf);
        let f1 = it.next().unwrap().unwrap();
        assert_eq!(f1.frame_length_bytes, 200);
        assert_eq!(f1.bytes().len(), 200);
        assert_eq!(f1.raw_header.len(), 7);
        let f2 = it.next().unwrap().unwrap();
        assert_eq!(f2.frame_length_bytes, 200);
        assert!(it.next().is_none());
    }

    #[test]
    fn frames_truncated_body_yields_truncated() {
        let mut buf = build_frame(4, 2, 200);
        buf.truncate(50); // header decodes but body too short
        let mut it = frames(&buf);
        match it.next() {
            Some(Err(CodecParseError::Truncated { .. })) => {}
            other => panic!("expected Err(Truncated), got {:?}", other),
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn frames_short_header_yields_truncated() {
        let mut it = frames(&[0xFF, 0xFF]);
        match it.next() {
            Some(Err(CodecParseError::Truncated { needed: 7, had: 2 })) => {}
            other => panic!("expected Truncated 7,2, got {:?}", other),
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn frames_bad_sync_yields_bad_sync_word() {
        let bad = [0xAB; 7];
        let mut it = frames(&bad);
        match it.next() {
            Some(Err(CodecParseError::BadSyncWord { .. })) => {}
            other => panic!("expected BadSyncWord, got {:?}", other),
        }
        assert!(it.next().is_none());
    }

    #[test]
    fn adts_frame_owned_roundtrip() {
        let body = vec![0xAA, 0xBB, 0xCC];
        let raw_header = vec![0x01, 0x02];
        let borrowed = AdtsFrame {
            profile: AacProfile::Lc,
            sample_rate_hz: 44100,
            channel_configuration: 2,
            channels: 2,
            frame_length_bytes: 3,
            samples_per_frame: 1024,
            num_raw_data_blocks: 1,
            has_crc: false,
            mpeg_version: MpegVersion::Mpeg4,
            raw_header: raw_header.clone(),
            body: &body,
        };
        let owned = borrowed.to_owned();
        let reborrowed = owned.as_ref();
        assert_eq!(borrowed, reborrowed);
        assert_eq!(owned.body, vec![0xAA, 0xBB, 0xCC]);
    }
}
