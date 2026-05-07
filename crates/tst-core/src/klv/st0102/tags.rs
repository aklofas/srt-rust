//! ST 0102.12 LS tag schema as data. Decoder and encoder loop this
//! table.
//!
//! Pinned against MISB ST 0102.12 §6.7 Table 2.

#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Encoding {
    /// Raw 1-byte typed enum codepoint (Tags 1, 2, 12).
    U8Enum,
    /// Raw 2-byte big-endian unsigned (Tag 22).
    U16Be,
    /// ISO/IEC 646 string (effectively ASCII-7), variable-length.
    /// Most string tags (3, 4, 5, 6, 7, 8, 9, 11, 14).
    Iso646,
    /// ISO/IEC 646 string with fixed length: "YYYYMMDD" (Tag 10) or
    /// "YYYY-MM-DD" (Tags 23, 24). Decoder accepts any length the
    /// `expected_len` permits; strict-mode rejects mismatches.
    FixedAscii { expected_len: usize },
    /// RFC 2781 UTF-16 with optional BOM (Tag 13). Default endianness
    /// is BE per RFC 2781 §4.3.
    Utf16Bom,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TagSpec {
    pub id: u8,
    pub encoding: Encoding,
    /// True if ST 0102.12 §6.7 Table 2 marks this tag as Required.
    pub required: bool,
}

pub(crate) const TAGS: &[TagSpec] = &[
    // populated in Task 3
];

pub(crate) fn lookup(tag: u8) -> Option<&'static TagSpec> {
    TAGS.iter().find(|t| t.id == tag)
}
