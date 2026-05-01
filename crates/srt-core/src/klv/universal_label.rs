//! 16-byte SMPTE/MISB Universal Label. Non-validating constructor; introspection
//! helpers for the SMPTE structural fields (oid, category, registry, version
//! byte). Well-known constants for canonical labels.
//!
//! Per SMPTE 336M / MISB ST 0107, a Universal Label is a 16-byte key. Bytes
//! 0-3 are the SMPTE OID prefix, byte 4 is the category, byte 5 is the
//! registry, byte 6 is the structure designator. For ST 0601, byte 14 carries
//! the document version (e.g. `0x13` for ST 0601.19), and byte 15 is reserved
//! (must be `0x00` for the ST 0601 family).
//!
//! Real-world records contain malformed or non-standard labels. This type is
//! deliberately permissive: `UniversalLabel::new` accepts any 16 bytes;
//! validation is opt-in via `decode_strict` in the typed layer.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UniversalLabel(pub [u8; 16]);

impl UniversalLabel {
    /// Construct from raw bytes. Non-validating.
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// SMPTE OID prefix (bytes 0-3).
    pub const fn oid(&self) -> [u8; 4] {
        [self.0[0], self.0[1], self.0[2], self.0[3]]
    }

    /// SMPTE category designator (byte 4).
    pub const fn category(&self) -> u8 {
        self.0[4]
    }

    /// SMPTE registry designator (byte 5).
    pub const fn registry(&self) -> u8 {
        self.0[5]
    }

    /// SMPTE structure designator (byte 6).
    pub const fn structure(&self) -> u8 {
        self.0[6]
    }

    /// Document version byte at offset 13.
    /// For ST 0601: e.g. 0x0E = .14, 0x13 = .19.
    pub const fn version_byte(&self) -> u8 {
        self.0[13]
    }

    // --- Well-known constants ---

    /// Canonical ST 0601 UAS Datalink Local Set UL with version byte 0x13 (= ST 0601.19).
    /// Bytes per MISB ST 0601.19 §6.1: 06 0E 2B 34 02 0B 01 01 0E 01 03 01 01 13 00 00.
    pub const ST_0601_LS: UniversalLabel = UniversalLabel([
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x13, 0x00,
        0x00,
    ]);

    /// SMPTE 336M generic local set key prefix (used by various MISB sets;
    /// concrete labels override byte 14 for version + final bytes for set-id).
    pub const SMPTE_336M_LS_KEY: UniversalLabel = UniversalLabel([
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
        0x00,
    ]);

    /// MISB ST 0605 §7 Precision Time Stamp Pack UL.
    /// Registered in MISB ST 0807.27 row 1061 as the Microsecond Timestamp Pack.
    /// Body: `[time_status:1][microseconds_since_epoch:8 BE]`.
    pub const PRECISION_TIMESTAMP_PACK_UL: UniversalLabel = UniversalLabel([
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x03, 0x11, 0x00, 0x00,
        0x00,
    ]);

    /// True if this UL belongs to the ST 0601 family — bytes 0-13 match
    /// the canonical prefix `06 0E 2B 34 02 0B 01 01 0E 01 03 01 01 ??`,
    /// byte 14 may be any document version, byte 15 must be `0x00`.
    pub const fn is_st0601_family(&self) -> bool {
        let canonical = [
            0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01,
        ];
        let mut i = 0;
        while i < 13 {
            if self.0[i] != canonical[i] {
                return false;
            }
            i += 1;
        }
        self.0[15] == 0x00
    }
}

impl Default for UniversalLabel {
    fn default() -> Self {
        Self::ST_0601_LS
    }
}

impl fmt::Display for UniversalLabel {
    /// Dotted-hex form: `06.0E.2B.34.02.0B.01.01.0E.01.03.01.01.13.00.00`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, b) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            write!(f, "{b:02X}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn st0601_constant_well_formed() {
        let ul = UniversalLabel::ST_0601_LS;
        assert_eq!(ul.oid(), [0x06, 0x0E, 0x2B, 0x34]);
        assert_eq!(ul.category(), 0x02);
        assert_eq!(ul.registry(), 0x0B);
        assert_eq!(ul.structure(), 0x01);
        assert_eq!(ul.version_byte(), 0x13);
        assert!(ul.is_st0601_family());
    }

    #[test]
    fn display_dotted_hex() {
        let ul = UniversalLabel::ST_0601_LS;
        assert_eq!(
            ul.to_string(),
            "06.0E.2B.34.02.0B.01.01.0E.01.03.01.01.13.00.00"
        );
    }

    #[test]
    fn display_zero() {
        let ul = UniversalLabel::new([0; 16]);
        assert_eq!(
            ul.to_string(),
            "00.00.00.00.00.00.00.00.00.00.00.00.00.00.00.00"
        );
    }

    #[test]
    fn family_check_accepts_any_version_byte() {
        let mut bytes = UniversalLabel::ST_0601_LS.0;
        bytes[14] = 0x0E; // ST 0601.14
        assert!(UniversalLabel::new(bytes).is_st0601_family());
        bytes[14] = 0xFF; // out-of-spec but still family
        assert!(UniversalLabel::new(bytes).is_st0601_family());
    }

    #[test]
    fn family_check_rejects_byte15_nonzero() {
        let mut bytes = UniversalLabel::ST_0601_LS.0;
        bytes[15] = 0x01;
        assert!(!UniversalLabel::new(bytes).is_st0601_family());
    }

    #[test]
    fn family_check_rejects_oid_mismatch() {
        let mut bytes = UniversalLabel::ST_0601_LS.0;
        bytes[0] = 0x07;
        assert!(!UniversalLabel::new(bytes).is_st0601_family());
    }

    #[test]
    fn default_is_st0601() {
        assert_eq!(UniversalLabel::default(), UniversalLabel::ST_0601_LS);
    }

    #[test]
    fn new_accepts_anything() {
        // Non-validating constructor — every byte combination is legal.
        let ul = UniversalLabel::new([0xFF; 16]);
        assert_eq!(ul.0, [0xFF; 16]);
    }

    #[test]
    fn const_compatible() {
        // Verifies the helpers are usable in const contexts.
        const UL: UniversalLabel = UniversalLabel::ST_0601_LS;
        const VB: u8 = UL.version_byte();
        assert_eq!(VB, 0x13);
    }
}
