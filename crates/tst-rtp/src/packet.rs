//! `RtpHeader` — the 12-byte fixed RTP header per RFC 3550 §5.1.
//!
//! Phase 1 supports only the fixed header (no CSRC list, no extension).
//! `V=2`, `P=0`, `X=0`, `CC=0`. `M` is always 0 in Phase 1 (we use a
//! system-clock timestamp source which has no discontinuity signal —
//! see [`RtpClock`](crate::clock::RtpClock)). `PT=33` (MP2T) per RFC
//! 3551 §6 Table 5.
//!
//! Receivers accept any payload type but only MP2T (33) is meaningful
//! for this crate; non-MP2T packets are silently dropped at the
//! transport boundary.

use thiserror::Error;

/// Fixed RTP header length per RFC 3550 §5.1.
pub const RTP_HEADER_LEN: usize = 12;

/// RTP payload type for MPEG-2 Transport Stream per RFC 3551 §6 Table 5.
pub const RTP_PT_MP2T: u8 = 33;

/// RTP version per RFC 3550 §5.1 (always 2).
pub const RTP_VERSION: u8 = 2;

/// One RTP fixed-header record.
///
/// Field accessors return scalars; the wire encoding lives in
/// [`Self::encode_into`]. Pre-1.0 the struct is `#[non_exhaustive]` so
/// adding a field (e.g., a `marker` setter once we have RTCP) is not a
/// breaking change.
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RtpHeader {
    /// Sequence number — wraps modulo 2^16.
    pub seq: u16,
    /// 32-bit timestamp at the rate of the payload's clock; for MP2T
    /// (PT=33) this is 90 kHz per RFC 3551 §6 Table 5.
    pub timestamp: u32,
    /// Synchronization source — random per-stream identifier.
    pub ssrc: u32,
}

impl RtpHeader {
    /// Construct a header from `seq` / `timestamp` / `ssrc`. Other
    /// fields are pinned: V=2, P=0, X=0, CC=0, M=0, PT=33.
    pub fn new(seq: u16, timestamp: u32, ssrc: u32) -> Self {
        Self {
            seq,
            timestamp,
            ssrc,
        }
    }

    /// Encode this header into the first 12 bytes of `buf`.
    ///
    /// # Panics
    ///
    /// Panics if `buf.len() < 12`.
    pub fn encode_into(&self, buf: &mut [u8]) {
        assert!(
            buf.len() >= RTP_HEADER_LEN,
            "RtpHeader::encode_into: buf too small ({} < {RTP_HEADER_LEN})",
            buf.len(),
        );
        // Octet 0: V(2) | P(1) | X(1) | CC(4) = 0b10_0_0_0000 = 0x80
        buf[0] = (RTP_VERSION << 6) & 0xC0;
        // Octet 1: M(1) | PT(7) = 0b0_0100001 = 33
        buf[1] = RTP_PT_MP2T & 0x7F;
        // Octets 2..4: sequence (big-endian)
        buf[2..4].copy_from_slice(&self.seq.to_be_bytes());
        // Octets 4..8: timestamp (big-endian)
        buf[4..8].copy_from_slice(&self.timestamp.to_be_bytes());
        // Octets 8..12: SSRC (big-endian)
        buf[8..12].copy_from_slice(&self.ssrc.to_be_bytes());
    }
}

/// Why an RTP packet failed to parse.
///
/// At the transport boundary these are silently dropped + counter-ticked
/// (see [`crate::transport::RtpStats::malformed_packets`]) rather than
/// surfaced as errors — RFC 3550 §5.1 expects receivers to ignore
/// unparseable packets and continue.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum RtpParseError {
    /// Packet too short to contain a fixed header (or fixed-header +
    /// CSRC list given the CC field's count).
    #[error("RTP packet too short: {got} < {need}")]
    Truncated { got: usize, need: usize },
    /// `V` (version) field was not 2.
    #[error("unsupported RTP version: {0}")]
    UnsupportedVersion(u8),
    /// `PT` (payload type) was not 33 (MP2T). This crate is
    /// MPEG-TS-over-RTP only; non-MP2T payloads are not parsed.
    #[error("unsupported RTP payload type: {0} (expected 33 MP2T)")]
    UnsupportedPayloadType(u8),
}

/// Result of [`RtpHeader::decode`] — header struct + offset where the
/// payload starts in the input buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parsed {
    /// Decoded fixed-header fields.
    pub header: RtpHeader,
    /// Byte offset in the input where the application payload begins.
    /// Equals `12 + 4*CC` for fixed-header-only packets.
    pub payload_offset: usize,
}

impl RtpHeader {
    /// Decode a 12-byte fixed RTP header from `buf[0..12]`, validate
    /// `V=2` and `PT=33`, and return the parsed header + the byte
    /// offset of the application payload (skipping any CSRC list).
    ///
    /// CSRC entries are skipped, not retained — Phase 1 doesn't need
    /// them. The `X` (extension) bit is honored: if set, the extension
    /// header is also skipped from the payload offset (added in a
    /// follow-up to this task if interop demands it; in Phase 1, X=1
    /// from peers is parsed but ignored — payload_offset still points
    /// at the bytes after the fixed+CSRC header).
    pub fn decode(buf: &[u8]) -> Result<Parsed, RtpParseError> {
        if buf.len() < RTP_HEADER_LEN {
            return Err(RtpParseError::Truncated {
                got: buf.len(),
                need: RTP_HEADER_LEN,
            });
        }
        let v = (buf[0] >> 6) & 0x03;
        if v != RTP_VERSION {
            return Err(RtpParseError::UnsupportedVersion(v));
        }
        let cc = (buf[0] & 0x0F) as usize;
        let pt = buf[1] & 0x7F;
        if pt != RTP_PT_MP2T {
            return Err(RtpParseError::UnsupportedPayloadType(pt));
        }
        let need = RTP_HEADER_LEN + cc * 4;
        if buf.len() < need {
            return Err(RtpParseError::Truncated {
                got: buf.len(),
                need,
            });
        }
        let seq = u16::from_be_bytes([buf[2], buf[3]]);
        let timestamp = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let ssrc = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);
        Ok(Parsed {
            header: RtpHeader::new(seq, timestamp, ssrc),
            payload_offset: need,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_writes_rfc_3550_byte_layout() {
        // RFC 3550 §5.1 octet layout:
        //  0                   1                   2                   3
        //  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
        // +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
        // |V=2|P|X|  CC   |M|     PT      |       sequence number         |
        // +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
        // |                           timestamp                           |
        // +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
        // |           synchronization source (SSRC) identifier            |
        // +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
        let h = RtpHeader::new(0x1234, 0xDEADBEEF, 0xCAFE_BABE);
        let mut buf = [0u8; 16];
        h.encode_into(&mut buf);
        assert_eq!(
            &buf[..12],
            &[
                0x80, // V=2, P=0, X=0, CC=0
                33,   // M=0, PT=33
                0x12, 0x34, // seq
                0xDE, 0xAD, 0xBE, 0xEF, // timestamp
                0xCA, 0xFE, 0xBA, 0xBE, // SSRC
            ],
        );
    }

    #[test]
    fn encode_zero_filled_fields() {
        let h = RtpHeader::new(0, 0, 0);
        let mut buf = [0xFFu8; 12];
        h.encode_into(&mut buf);
        // Octet 0 = 0x80, octet 1 = 33, rest = 0.
        assert_eq!(buf[0], 0x80);
        assert_eq!(buf[1], 33);
        assert!(buf[2..].iter().all(|b| *b == 0));
    }

    #[test]
    #[should_panic(expected = "buf too small")]
    fn encode_panics_on_short_buf() {
        let h = RtpHeader::new(0, 0, 0);
        let mut buf = [0u8; 11];
        h.encode_into(&mut buf);
    }

    #[test]
    fn decode_matches_encoded_bytes() {
        let h = RtpHeader::new(0x1234, 0xDEADBEEF, 0xCAFE_BABE);
        let mut buf = [0u8; 12];
        h.encode_into(&mut buf);
        let parsed = RtpHeader::decode(&buf).unwrap();
        // Decode returns header + payload offset.
        assert_eq!(parsed.header, h);
        assert_eq!(parsed.payload_offset, 12);
    }

    #[test]
    fn decode_rejects_wrong_version() {
        // V=3 (top two bits = 0b11_0_0_0000 = 0xC0) — invalid.
        let mut buf = [0u8; 12];
        buf[0] = 0xC0;
        buf[1] = 33;
        let err = RtpHeader::decode(&buf).unwrap_err();
        assert!(matches!(err, RtpParseError::UnsupportedVersion(3)));
    }

    #[test]
    fn decode_rejects_wrong_payload_type() {
        let mut buf = [0u8; 12];
        buf[0] = 0x80;
        buf[1] = 96; // dynamic PT range, not MP2T
        let err = RtpHeader::decode(&buf).unwrap_err();
        assert!(matches!(err, RtpParseError::UnsupportedPayloadType(96)));
    }

    #[test]
    fn decode_accepts_marker_bit() {
        // M=1 is informational only on receive; we don't reject it.
        let mut buf = [0u8; 12];
        buf[0] = 0x80;
        buf[1] = 0x80 | 33; // M=1, PT=33
        let parsed = RtpHeader::decode(&buf).unwrap();
        assert_eq!(parsed.header.seq, 0);
    }

    #[test]
    fn decode_skips_csrc_list() {
        // CC=2 → 8 extra CSRC bytes between header and payload.
        let mut buf = vec![0u8; 12 + 8];
        buf[0] = 0x82; // V=2, CC=2
        buf[1] = 33;
        let parsed = RtpHeader::decode(&buf).unwrap();
        assert_eq!(parsed.payload_offset, 20);
    }

    #[test]
    fn decode_rejects_truncated() {
        let buf = [0u8; 11];
        let err = RtpHeader::decode(&buf).unwrap_err();
        assert!(matches!(err, RtpParseError::Truncated { .. }));
    }

    #[test]
    fn decode_rejects_csrc_overflow() {
        // CC=5 needs 20 bytes of CSRC, but buf is only 12.
        let mut buf = [0u8; 12];
        buf[0] = 0x85; // V=2, CC=5
        buf[1] = 33;
        let err = RtpHeader::decode(&buf).unwrap_err();
        assert!(matches!(err, RtpParseError::Truncated { .. }));
    }

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn encode_decode_roundtrip(seq in any::<u16>(), ts in any::<u32>(), ssrc in any::<u32>()) {
            let h = RtpHeader::new(seq, ts, ssrc);
            let mut buf = [0u8; 12];
            h.encode_into(&mut buf);
            let parsed = RtpHeader::decode(&buf).unwrap();
            prop_assert_eq!(parsed.header, h);
            prop_assert_eq!(parsed.payload_offset, 12);
        }
    }
}
