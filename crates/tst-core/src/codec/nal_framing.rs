//! Annex B ↔ length-prefixed NAL conversion.
//!
//! **Stability: Provisional** — see the
//! [API stability reference](https://github.com/aklofas/ts-transformer/blob/main/docs/reference/api-stability.md).
//!
//! MPEG-TS elementary streams (and this crate's own [`crate::mpegts::demux`]
//! output) carry H.264/H.265/H.266 NAL units Annex-B-framed: each NAL is
//! delimited by a `00 00 01` or `00 00 00 01` start code. Some consumers —
//! notably Apple's VideoToolbox, and the ISO/IEC 14496-15 AVCC/HVCC sample
//! formats it expects — instead frame each NAL with a fixed-width
//! big-endian length prefix and no start code at all. This module converts
//! between the two framings without touching NAL contents: header bytes
//! and emulation-prevention bytes are passed through verbatim.
//!
//! Both directions are pure byte-plumbing — no bitstream parsing, no
//! codec-specific knowledge. `length_size` must be 1, 2, or 4 bytes,
//! matching the widths ISO/IEC 14496-15's `NALUnitLength` field allows.

use crate::codec::CodecParseError;
use alloc::vec::Vec;

/// Offsets of one Annex-B start-code occurrence: where the prefix starts
/// (the run of `00`s plus the trailing `01`) and where the NAL data begins
/// (immediately after). Mirrors the same-shaped helper in
/// `mpegts::demux::payload`, kept as a private local copy here rather than
/// exported from there — this module has no other reason to depend on
/// demux internals.
#[derive(Debug, Clone, Copy)]
struct StartCode {
    prefix_start: usize,
    data_start: usize,
}

/// Locate every Annex-B start code (`00 00 01` or `00 00 00 01`) in `buf`.
fn find_start_codes(buf: &[u8]) -> Vec<StartCode> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 <= buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 {
            if buf[i + 2] == 1 {
                out.push(StartCode {
                    prefix_start: i,
                    data_start: i + 3,
                });
                i += 3;
                continue;
            }
            if i + 4 <= buf.len() && buf[i + 2] == 0 && buf[i + 3] == 1 {
                out.push(StartCode {
                    prefix_start: i,
                    data_start: i + 4,
                });
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Validate a `length_size` argument, returning the maximum NAL byte
/// length it can encode.
fn max_encodable_len(length_size: u8) -> Result<u32, CodecParseError> {
    match length_size {
        1 => Ok(u8::MAX as u32),
        2 => Ok(u16::MAX as u32),
        4 => Ok(u32::MAX),
        other => Err(CodecParseError::InvalidLengthSize { got: other }),
    }
}

/// Convert an Annex-B-framed buffer (start-code-delimited NALs) into
/// length-prefixed framing: each NAL becomes a `length_size`-byte
/// big-endian length followed by the NAL bytes (header byte included).
///
/// Accepts both 3-byte (`00 00 01`) and 4-byte (`00 00 00 01`) start
/// codes; either may appear anywhere in `annexb`, including mixed within
/// the same buffer. Emulation-prevention bytes inside each NAL are left
/// untouched — they're part of the NAL payload on the wire, not part of
/// the framing this function rewrites.
///
/// `length_size` must be 1, 2, or 4 — [`CodecParseError::InvalidLengthSize`]
/// otherwise. If any single NAL's byte length exceeds what `length_size`
/// bytes can encode, returns [`CodecParseError::NalLengthOverflow`].
///
/// Bytes in `annexb` before the first start code (if any) are not part of
/// any NAL and are dropped, matching how [`crate::mpegts::demux`] itself
/// scans Annex B. An `annexb` with no start code at all yields an empty
/// `Vec`.
pub fn annexb_to_length_prefixed(
    annexb: &[u8],
    length_size: u8,
) -> Result<Vec<u8>, CodecParseError> {
    let max_len = max_encodable_len(length_size)?;
    let starts = find_start_codes(annexb);
    let mut out = Vec::new();
    for win in starts.windows(2) {
        let nal = &annexb[win[0].data_start..win[1].prefix_start];
        write_length_prefixed_nal(&mut out, nal, length_size, max_len)?;
    }
    if let Some(&last) = starts.last() {
        let nal = &annexb[last.data_start..annexb.len()];
        write_length_prefixed_nal(&mut out, nal, length_size, max_len)?;
    }
    Ok(out)
}

fn write_length_prefixed_nal(
    out: &mut Vec<u8>,
    nal: &[u8],
    length_size: u8,
    max_len: u32,
) -> Result<(), CodecParseError> {
    let len = nal.len();
    if len as u64 > max_len as u64 {
        return Err(CodecParseError::NalLengthOverflow {
            nal_len: len.min(u32::MAX as usize) as u32,
            length_size,
        });
    }
    let len = len as u32;
    match length_size {
        1 => out.push(len as u8),
        2 => out.extend_from_slice(&(len as u16).to_be_bytes()),
        4 => out.extend_from_slice(&len.to_be_bytes()),
        _ => unreachable!("length_size validated by max_encodable_len before this is called"),
    }
    out.extend_from_slice(nal);
    Ok(())
}

/// Convert a length-prefixed buffer (each NAL preceded by a `length_size`-
/// byte big-endian length, no start codes) into Annex-B framing: each NAL
/// is emitted as a 4-byte `00 00 00 01` start code followed by the NAL
/// bytes, back to back.
///
/// `length_size` must be 1, 2, or 4 — [`CodecParseError::InvalidLengthSize`]
/// otherwise. If `data` ends mid-length-prefix or mid-NAL (fewer bytes
/// remain than the just-read length declares), returns
/// [`CodecParseError::Truncated`].
///
/// Inverse of [`annexb_to_length_prefixed`] up to start-code width
/// normalization: a 3-byte Annex-B start code round-trips through this
/// pair as a 4-byte one, since length-prefixed framing carries no
/// start-code-width information to preserve.
pub fn length_prefixed_to_annexb(data: &[u8], length_size: u8) -> Result<Vec<u8>, CodecParseError> {
    max_encodable_len(length_size)?;
    let length_size = length_size as usize;
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let remaining = data.len() - pos;
        if remaining < length_size {
            return Err(CodecParseError::Truncated {
                needed: length_size as u32,
                had: remaining as u32,
            });
        }
        let nal_len = read_length_prefix(&data[pos..pos + length_size]);
        pos += length_size;

        let remaining = data.len() - pos;
        let nal_len = nal_len as usize;
        if remaining < nal_len {
            return Err(CodecParseError::Truncated {
                needed: nal_len as u32,
                had: remaining as u32,
            });
        }
        out.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        out.extend_from_slice(&data[pos..pos + nal_len]);
        pos += nal_len;
    }
    Ok(out)
}

/// Read a big-endian length prefix of 1, 2, or 4 bytes. `prefix.len()`
/// must equal one of those widths (the only callers slice exactly
/// `length_size` bytes, and `length_size` is validated by
/// [`max_encodable_len`] before this is ever reached).
fn read_length_prefix(prefix: &[u8]) -> u32 {
    match prefix.len() {
        1 => prefix[0] as u32,
        2 => u16::from_be_bytes([prefix[0], prefix[1]]) as u32,
        4 => u32::from_be_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]),
        other => unreachable!("length_size validated to 1/2/4 before this is called, got {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-built two-NAL Annex B buffer: NAL 1 uses a 4-byte start code,
    /// NAL 2 uses a 3-byte start code (mixed on purpose, since both are
    /// legal anywhere in an Annex-B stream).
    fn two_nal_annexb() -> Vec<u8> {
        vec![
            0x00, 0x00, 0x00, 0x01, // 4-byte start code
            0x67, 0xAA, 0xBB, 0xCC, // NAL 1: header 0x67 + 3 payload bytes (len 4)
            0x00, 0x00, 0x01, // 3-byte start code
            0x65, 0xDD, 0xEE, // NAL 2: header 0x65 + 2 payload bytes (len 3)
        ]
    }

    #[test]
    fn annexb_to_length_prefixed_emits_be_lengths_and_header_bytes() {
        let out = annexb_to_length_prefixed(&two_nal_annexb(), 4).unwrap();
        assert_eq!(
            out,
            vec![
                0x00, 0x00, 0x00, 0x04, // NAL 1 length, big-endian
                0x67, 0xAA, 0xBB, 0xCC, // NAL 1 bytes, header included
                0x00, 0x00, 0x00, 0x03, // NAL 2 length, big-endian
                0x65, 0xDD, 0xEE, // NAL 2 bytes, header included
            ]
        );
    }

    #[test]
    fn round_trip_normalizes_start_code_width_to_four_bytes() {
        let annexb = two_nal_annexb();
        let length_prefixed = annexb_to_length_prefixed(&annexb, 4).unwrap();
        let round_tripped = length_prefixed_to_annexb(&length_prefixed, 4).unwrap();
        // NAL 2's 3-byte start code in the input normalizes to 4 bytes —
        // length-prefixed framing carries no start-code-width information.
        let normalized = vec![
            0x00, 0x00, 0x00, 0x01, 0x67, 0xAA, 0xBB, 0xCC, // NAL 1
            0x00, 0x00, 0x00, 0x01, 0x65, 0xDD, 0xEE, // NAL 2, now 4-byte start code
        ];
        assert_eq!(round_tripped, normalized);
    }

    #[test]
    fn annexb_to_length_prefixed_rejects_invalid_length_size() {
        let err = annexb_to_length_prefixed(&two_nal_annexb(), 3).unwrap_err();
        assert_eq!(err, CodecParseError::InvalidLengthSize { got: 3 });
    }

    #[test]
    fn length_prefixed_to_annexb_rejects_invalid_length_size() {
        let err = length_prefixed_to_annexb(&[0x00, 0x01, 0xAA], 3).unwrap_err();
        assert_eq!(err, CodecParseError::InvalidLengthSize { got: 3 });
    }

    #[test]
    fn annexb_to_length_prefixed_one_byte_length_size_round_trips() {
        let out = annexb_to_length_prefixed(&two_nal_annexb(), 1).unwrap();
        assert_eq!(
            out,
            vec![
                0x04, 0x67, 0xAA, 0xBB, 0xCC, // NAL 1: 1-byte length + bytes
                0x03, 0x65, 0xDD, 0xEE, // NAL 2: 1-byte length + bytes
            ]
        );
        let annexb = length_prefixed_to_annexb(&out, 1).unwrap();
        assert_eq!(
            annexb,
            vec![
                0x00, 0x00, 0x00, 0x01, 0x67, 0xAA, 0xBB, 0xCC, 0x00, 0x00, 0x00, 0x01, 0x65, 0xDD,
                0xEE,
            ]
        );
    }

    #[test]
    fn nal_exceeding_one_byte_length_size_overflows() {
        // 256 payload bytes -> NAL length 257, too large for a 1-byte prefix.
        let mut annexb = vec![0x00, 0x00, 0x00, 0x01, 0x67];
        annexb.extend(core::iter::repeat(0xAA).take(256));
        let err = annexb_to_length_prefixed(&annexb, 1).unwrap_err();
        assert_eq!(
            err,
            CodecParseError::NalLengthOverflow {
                nal_len: 257,
                length_size: 1,
            }
        );
    }

    #[test]
    fn length_prefixed_to_annexb_truncated_prefix_errors() {
        // Only 1 byte present where a 2-byte prefix is required.
        let err = length_prefixed_to_annexb(&[0xAA], 2).unwrap_err();
        assert_eq!(err, CodecParseError::Truncated { needed: 2, had: 1 });
    }

    #[test]
    fn length_prefixed_to_annexb_truncated_nal_body_errors() {
        // 4-byte length prefix declares 10 bytes of NAL, only 2 follow.
        let data = vec![0x00, 0x00, 0x00, 0x0A, 0x67, 0xAA];
        let err = length_prefixed_to_annexb(&data, 4).unwrap_err();
        assert_eq!(err, CodecParseError::Truncated { needed: 10, had: 2 });
    }

    #[test]
    fn empty_input_round_trips_to_empty_output() {
        assert_eq!(annexb_to_length_prefixed(&[], 4).unwrap(), Vec::<u8>::new());
        assert_eq!(length_prefixed_to_annexb(&[], 4).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn bytes_before_first_start_code_are_dropped() {
        let mut annexb = vec![0xDE, 0xAD, 0xBE, 0xEF]; // no start code here
        annexb.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x67, 0x01]);
        let out = annexb_to_length_prefixed(&annexb, 4).unwrap();
        assert_eq!(out, vec![0x00, 0x00, 0x00, 0x02, 0x67, 0x01]);
    }
}
