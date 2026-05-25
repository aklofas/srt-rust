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
}
