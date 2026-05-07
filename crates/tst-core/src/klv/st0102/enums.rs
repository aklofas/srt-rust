//! Typed value enums for ST 0102.12 LS tags 1, 2, and 12.
//!
//! Three distinct enums because Tags 2 and 12 use different uint8
//! codepoints for the same logical coding method (e.g. ISO-3166
//! Numeric is 0x05 in Tag 2 but 0x03 in Tag 12 per ST 0102.12 §6.7
//! Table 2). Sharing one enum would require two encode tables anyway
//! for no consumer-side benefit.

/// Tag 1 — Security Classification per ST 0102.12 §6.1.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityClassification {
    Unclassified, // 0x01
    Restricted,   // 0x02
    Confidential, // 0x03
    Secret,       // 0x04
    TopSecret,    // 0x05
    Unknown(u8),
}

/// Tag 2 — Classifying Country and Releasing Instructions Country
/// Coding Method per ST 0102.12 §6.1.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassifyingCountryCodingMethod {
    Iso3166TwoLetter,      // 0x01
    Iso3166ThreeLetter,    // 0x02
    Fips104TwoLetter,      // 0x03
    Fips104FourLetter,     // 0x04
    Iso3166Numeric,        // 0x05
    Stanag1059TwoLetter,   // 0x06
    Stanag1059ThreeLetter, // 0x07
    OmittedValue08,        // 0x08 (reserved/omitted by spec)
    OmittedValue09,        // 0x09 (reserved/omitted by spec)
    Fips104Mixed,          // 0x0A
    Iso3166Mixed,          // 0x0B
    Stanag1059Mixed,       // 0x0C
    GencTwoLetter,         // 0x0D
    GencThreeLetter,       // 0x0E
    GencNumeric,           // 0x0F
    GencMixed,             // 0x10
    Unknown(u8),
}

/// Tag 12 — Object Country Coding Method per ST 0102.12 §6.1.12.
/// Note: codepoints differ from Tag 2; the spec is non-contiguous
/// (jumps to 0x40 for `GencAdminSub`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectCountryCodingMethod {
    Iso3166TwoLetter,      // 0x01
    Iso3166ThreeLetter,    // 0x02
    Iso3166Numeric,        // 0x03 (≠ Tag 2's 0x05)
    Fips104TwoLetter,      // 0x04 (≠ Tag 2's 0x03)
    Fips104FourLetter,     // 0x05 (≠ Tag 2's 0x04)
    Stanag1059TwoLetter,   // 0x06
    Stanag1059ThreeLetter, // 0x07
    OmittedValue08,        // 0x08
    OmittedValue09,        // 0x09
    OmittedValue0A,        // 0x0A
    OmittedValue0B,        // 0x0B
    OmittedValue0C,        // 0x0C
    GencTwoLetter,         // 0x0D
    GencThreeLetter,       // 0x0E
    GencNumeric,           // 0x0F
    GencAdminSub,          // 0x40 (jumps; spec is non-contiguous)
    Unknown(u8),
}

impl SecurityClassification {
    pub fn from_u8(b: u8) -> Self {
        todo!("Task 2")
    }
    pub fn to_u8(self) -> u8 {
        todo!("Task 2")
    }
    /// True if this codepoint is in the spec's enumerated range
    /// (excluding `Unknown`). Used by strict-mode validation.
    pub fn is_known_codepoint(self) -> bool {
        todo!("Task 2")
    }
}

impl ClassifyingCountryCodingMethod {
    pub fn from_u8(b: u8) -> Self {
        todo!("Task 2")
    }
    pub fn to_u8(self) -> u8 {
        todo!("Task 2")
    }
    /// True if this codepoint is in the spec's enumerated range AND is
    /// not a reserved/omitted slot (`OmittedValueXX`). Used by
    /// strict-mode validation.
    pub fn is_known_codepoint(self) -> bool {
        todo!("Task 2")
    }
}

impl ObjectCountryCodingMethod {
    pub fn from_u8(b: u8) -> Self {
        todo!("Task 2")
    }
    pub fn to_u8(self) -> u8 {
        todo!("Task 2")
    }
    pub fn is_known_codepoint(self) -> bool {
        todo!("Task 2")
    }
}
