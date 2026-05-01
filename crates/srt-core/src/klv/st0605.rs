//! MISB ST 0605 §7 Precision Time Stamp Pack: PES-emit-time auxiliary
//! KLV record commonly multiplexed alongside an ST 0601 LS in real
//! captures. Body is a 1-byte Time Status (per MISB ST 0603 §7.4) plus
//! an 8-byte big-endian microsecond timestamp (per MISB ST 0603 §7.1).
//!
//! Registered in MISB ST 0807.27 row 1061 (UL CRC 23259).

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
}
