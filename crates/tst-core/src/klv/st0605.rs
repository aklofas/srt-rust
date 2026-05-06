//! MISB ST 0605 §7 Precision Time Stamp Pack: PES-emit-time auxiliary
//! KLV record commonly multiplexed alongside an ST 0601 LS in real
//! captures. Body is a 1-byte Time Status (per MISB ST 0603 §7.4) plus
//! an 8-byte big-endian microsecond timestamp (per MISB ST 0603 §7.1).
//!
//! Registered in MISB ST 0807.27 row 1061 (UL CRC 23259).

use crate::error::KlvDecodeError;
use crate::klv::length::read_ber;
use crate::klv::universal_label::UniversalLabel;

/// Time Status byte per MISB ST 0603 §7.4 Table 3.
///
/// - bit 7: 0 = Locked, 1 = Lock Unknown
/// - bit 6: 0 = Normal, 1 = Discontinuity
/// - bit 5: 0 = Forward, 1 = Reverse (only meaningful when bit 6=1)
/// - bits 4-0: reserved, must be 0b11111
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeStatus(pub u8);

impl TimeStatus {
    /// True if bit 7 = 0 (clock locked to absolute time reference).
    pub const fn is_locked(self) -> bool {
        self.0 & 0x80 == 0
    }

    /// True if bit 6 = 1 (time has not incremented forward in a linear
    /// fashion — i.e., a reset, jump, or correction occurred).
    pub const fn has_discontinuity(self) -> bool {
        self.0 & 0x40 != 0
    }

    /// True if bit 5 = 1 (only meaningful when `has_discontinuity()` —
    /// indicates a backward time jump rather than forward).
    pub const fn is_reverse_jump(self) -> bool {
        self.0 & 0x20 != 0
    }

    /// True if reserved bits 4-0 are the spec-required `0b11111`.
    pub const fn reserved_bits_valid(self) -> bool {
        self.0 & 0x1F == 0x1F
    }
}

/// MISB ST 0605 §7 Precision Time Stamp Pack typed view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrecisionTimeStampPack {
    pub time_status: TimeStatus,
    /// Microseconds since 1970-01-01T00:00:00Z (POSIX epoch), big-endian.
    pub timestamp_us: u64,
}

/// Decode a Precision Time Stamp Pack from a buffer that starts with
/// the 16-byte UL. Returns the typed view; verifies the UL and body
/// length match MISB ST 0605 §7. Does **not** validate the reserved
/// bits in the status byte — call `time_status.reserved_bits_valid()`
/// on the returned struct if you need that check.
pub fn decode(buf: &[u8]) -> Result<PrecisionTimeStampPack, KlvDecodeError> {
    if buf.len() < 16 {
        return Err(KlvDecodeError::Truncated {
            offset: 0,
            needed: 16,
            have: buf.len(),
        });
    }
    let mut ul = [0u8; 16];
    ul.copy_from_slice(&buf[..16]);
    let label = UniversalLabel::new(ul);
    if label != UniversalLabel::PRECISION_TIMESTAMP_PACK_UL {
        return Err(KlvDecodeError::UnexpectedUniversalLabel {
            expected: UniversalLabel::PRECISION_TIMESTAMP_PACK_UL,
            found: label,
        });
    }
    let (declared_len, after_len) = read_ber(&buf[16..])?;
    if declared_len != 9 {
        return Err(KlvDecodeError::BadTimeStampPackLength { got: declared_len });
    }
    if after_len.len() < 9 {
        return Err(KlvDecodeError::Truncated {
            offset: buf.len() - after_len.len(),
            needed: 9,
            have: after_len.len(),
        });
    }
    let body = &after_len[..9];
    let mut ts_bytes = [0u8; 8];
    ts_bytes.copy_from_slice(&body[1..9]);
    Ok(PrecisionTimeStampPack {
        time_status: TimeStatus(body[0]),
        timestamp_us: u64::from_be_bytes(ts_bytes),
    })
}

/// Encode a Precision Time Stamp Pack to a 26-byte buffer:
/// `[UL:16][BER 0x09:1][status:1][microseconds:8 BE]`.
pub fn encode(pack: &PrecisionTimeStampPack) -> [u8; 26] {
    let mut out = [0u8; 26];
    out[..16].copy_from_slice(&UniversalLabel::PRECISION_TIMESTAMP_PACK_UL.0);
    out[16] = 0x09;
    out[17] = pack.time_status.0;
    out[18..26].copy_from_slice(&pack.timestamp_us.to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_status_locked_normal() {
        // 0x1F = 0b 0001 1111: locked, normal increment, reserved bits ok
        let s = TimeStatus(0x1F);
        assert!(s.is_locked());
        assert!(!s.has_discontinuity());
        assert!(!s.is_reverse_jump());
        assert!(s.reserved_bits_valid());
    }

    #[test]
    fn time_status_lock_unknown_normal() {
        // 0x9F = 0b 1001 1111: lock unknown, normal increment
        let s = TimeStatus(0x9F);
        assert!(!s.is_locked());
        assert!(!s.has_discontinuity());
        assert!(s.reserved_bits_valid());
    }

    #[test]
    fn time_status_discontinuity_reverse() {
        // 0xFF = 0b 1111 1111: lock unknown, discontinuity, reverse jump
        let s = TimeStatus(0xFF);
        assert!(!s.is_locked());
        assert!(s.has_discontinuity());
        assert!(s.is_reverse_jump());
        assert!(s.reserved_bits_valid());
    }

    #[test]
    fn time_status_invalid_reserved() {
        // Reserved bits must be 11111; 0x10 = 0b 0001 0000 has bits 3-0 = 0
        let s = TimeStatus(0x10);
        assert!(!s.reserved_bits_valid());
    }

    #[test]
    fn decode_locked_pack() {
        // UL + BER 0x09 + status 0x1F + 8-byte BE timestamp
        let mut buf = Vec::new();
        buf.extend_from_slice(&UniversalLabel::PRECISION_TIMESTAMP_PACK_UL.0);
        buf.push(0x09);
        buf.push(0x1F); // locked, normal
        buf.extend_from_slice(&1_753_983_356_565_441u64.to_be_bytes());
        let pack = decode(&buf).unwrap();
        assert!(pack.time_status.is_locked());
        assert!(pack.time_status.reserved_bits_valid());
        assert_eq!(pack.timestamp_us, 1_753_983_356_565_441);
    }

    #[test]
    fn decode_rejects_wrong_ul() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&UniversalLabel::ST_0601_LS.0);
        buf.push(0x09);
        buf.push(0x1F);
        buf.extend_from_slice(&[0u8; 8]);
        let err = decode(&buf).unwrap_err();
        assert!(matches!(
            err,
            KlvDecodeError::UnexpectedUniversalLabel { .. }
        ));
    }

    #[test]
    fn decode_rejects_short_buffer() {
        let buf = [0u8; 8];
        let err = decode(&buf).unwrap_err();
        assert!(matches!(err, KlvDecodeError::Truncated { .. }));
    }

    #[test]
    fn decode_rejects_wrong_body_length() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&UniversalLabel::PRECISION_TIMESTAMP_PACK_UL.0);
        buf.push(0x05); // declared 5, not 9
        buf.extend_from_slice(&[0u8; 5]);
        let err = decode(&buf).unwrap_err();
        assert!(matches!(
            err,
            KlvDecodeError::BadTimeStampPackLength { got: 5 }
        ));
    }

    #[test]
    fn encode_round_trip() {
        let pack = PrecisionTimeStampPack {
            time_status: TimeStatus(0x1F),
            timestamp_us: 1_700_000_000_123_456,
        };
        let buf = encode(&pack);
        assert_eq!(buf.len(), 26); // 16 UL + 1 BER + 9 body
        let back = decode(&buf).unwrap();
        assert_eq!(back, pack);
    }

    #[test]
    fn encode_starts_with_ul_and_length() {
        let pack = PrecisionTimeStampPack {
            time_status: TimeStatus(0x9F),
            timestamp_us: 0,
        };
        let buf = encode(&pack);
        assert_eq!(&buf[..16], &UniversalLabel::PRECISION_TIMESTAMP_PACK_UL.0);
        assert_eq!(buf[16], 0x09);
        assert_eq!(buf[17], 0x9F);
        assert_eq!(&buf[18..26], &[0u8; 8]);
    }
}
