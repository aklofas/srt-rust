//! ST 0903.6 §6 Table 1 — top-level VMTI Local Set tag schema.
//!
//! The decoder and encoder loop this table. Adding or modifying a tag
//! is a one-entry change here.

// Placeholder skeleton — `Encoding`, `TagSpec`, `TAGS`, and `lookup`
// are populated in Task 2 and consumed by the decode/encode loops in
// Tasks 5–7. Mirrors the `klv::st0102::tags` precedent (where
// `TagSpec::required` is the sole dead-in-lib field after wiring).
#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Encoding {
    /// Raw 1-byte big-endian unsigned (e.g. Tag 4 Version Number).
    U8,
    /// Raw 2-byte big-endian unsigned (e.g. Tag 1 Checksum).
    U16Be,
    /// Raw 4-byte big-endian unsigned (e.g. Tag 7 Frame Number, when the
    /// spec uses a fixed-width form). Wire encoding is `length` bytes
    /// big-endian.
    U32Be,
    /// BER-OID variable-length unsigned (Tags 5, 6, 7, 8, 9 in v6).
    /// Decoded via `klv::length::read_ber_oid` (returns `u32`).
    BerOid,
    /// Raw 8-byte big-endian unsigned (Tag 2 Precision Time Stamp, µs
    /// since UNIX epoch).
    U64Be,
    /// UTF-8 string with caller-provided maximum length.
    Utf8 { max_bytes: usize },
    /// IMAPB-encoded floating-point with linear range. Wire form is
    /// `length` raw bytes mapped via `klv::imapb::decode`.
    ImapbF64 { min: f64, max: f64 },
    /// Raw bytes (variable length); pass-through. Used for nested LSes
    /// (Tags 102, 103, 104) and for VTargetSeries (Tag 101) — the latter
    /// is parsed in a second pass after the lenient decode walks the LS.
    RawBytes,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TagSpec {
    pub id: u8,
    pub name: &'static str,
    pub encoding: Encoding,
    /// True if ST 0903.6 §6 Table 1 marks this tag as Required.
    pub required: bool,
}

pub(crate) const TAGS: &[TagSpec] = &[
    // populated in Task 2
];

pub(crate) fn lookup(tag: u8) -> Option<&'static TagSpec> {
    TAGS.iter().find(|t| t.id == tag)
}
