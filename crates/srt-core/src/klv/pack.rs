//! Generic KLV pack and unpack — `RawField`, `OwnedRawField`, `Iter`,
//! `encode_pack`. Knows nothing about ST 0601 specifically.
//!
//! `RawField<'a>` borrows from the input buffer (zero-alloc iteration).
//! `OwnedRawField` is the owned counterpart used by parsed records that
//! cross the FFI boundary — keeping the parsed record `'static`.

use crate::error::{KlvDecodeError, KlvEncodeError};
use crate::klv::length::{
    self, LengthEncoding, ber_len, ber_oid_len, read_ber, read_ber_oid, write_ber, write_ber_oid,
};
use crate::klv::universal_label::UniversalLabel;

/// A KLV field that borrows its value bytes from a parent buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawField<'a> {
    pub tag: u32,
    pub value: &'a [u8],
}

/// A KLV field that owns its value bytes. Used by parsed records.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OwnedRawField {
    pub tag: u32,
    pub value: Vec<u8>,
}

impl<'a> From<&RawField<'a>> for OwnedRawField {
    fn from(r: &RawField<'a>) -> Self {
        Self {
            tag: r.tag,
            value: r.value.to_vec(),
        }
    }
}

impl<'a> From<RawField<'a>> for OwnedRawField {
    fn from(r: RawField<'a>) -> Self {
        Self {
            tag: r.tag,
            value: r.value.to_vec(),
        }
    }
}

impl OwnedRawField {
    pub fn as_ref(&self) -> RawField<'_> {
        RawField {
            tag: self.tag,
            value: &self.value,
        }
    }
}

/// What encoding `Iter` uses for tag and length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IterMode {
    LocalSet,     // 1-byte tag + BER-OID length (note: ST 0601 LS uses 1-byte tag + BER-OID length)
    UniversalSet, // 16-byte UL key + BER length
}

/// Iterator over a KLV body (post-UL, post-outer-length).
#[allow(dead_code)]
pub struct Iter<'a> {
    buf: &'a [u8],
    offset: usize,
    mode: IterMode,
    finished: bool,
}

impl<'a> Iter<'a> {
    /// Iterate a local-set body: 1-byte tag + BER-OID length, repeating.
    /// Caller is responsible for stripping the outer UL + total length first.
    pub fn local_set(buf: &'a [u8]) -> Self {
        Self {
            buf,
            offset: 0,
            mode: IterMode::LocalSet,
            finished: false,
        }
    }

    /// Iterate a universal-set body: 16-byte UL key + BER length, repeating.
    pub fn universal_set(buf: &'a [u8]) -> Self {
        Self {
            buf,
            offset: 0,
            mode: IterMode::UniversalSet,
            finished: false,
        }
    }

    pub fn remaining(&self) -> &'a [u8] {
        &self.buf[self.offset..]
    }
}

/// Encode a KLV pack: 16-byte UL + outer length + concatenated TLVs.
///
/// `length_encoding` controls *both* the outer total-length encoding and
/// the per-field tag-length encoding pattern that follows from it:
/// - `LengthEncoding::Ber` — outer BER, per-field 16-byte UL + BER length (universal set form).
/// - `LengthEncoding::BerOid` — outer BER, per-field 1-byte tag + BER-OID length (local set form).
///
/// Other variants are not currently supported by `encode_pack`; use
/// `klv::st0601::encode` for the ST 0601 specifics.
pub fn encode_pack<'a>(
    label: &UniversalLabel,
    fields: impl IntoIterator<Item = RawField<'a>>,
    length_encoding: LengthEncoding,
    out: &mut [u8],
) -> Result<usize, KlvEncodeError> {
    let mut tmp: Vec<u8> = Vec::new();
    for f in fields {
        match length_encoding {
            LengthEncoding::BerOid => {
                let mut tag_buf = [0u8; 8];
                let n = write_ber_oid(f.tag, &mut tag_buf)?;
                tmp.extend_from_slice(&tag_buf[..n]);
                let mut len_buf = [0u8; 16];
                let m = write_ber(f.value.len(), &mut len_buf)?;
                tmp.extend_from_slice(&len_buf[..m]);
                tmp.extend_from_slice(f.value);
            }
            LengthEncoding::Ber => {
                // For universal-set form, "tag" is a full UL — but RawField only
                // carries u32. encode_pack with Ber is reserved for use cases that
                // wrap encode_pack with a UL-keyed table; in v0 we error.
                return Err(KlvEncodeError::RecordTooLarge);
            }
            _ => return Err(KlvEncodeError::RecordTooLarge),
        }
    }
    let total_inner = tmp.len();
    let outer_len_bytes = ber_len(total_inner);
    let needed = 16 + outer_len_bytes + total_inner;
    if out.len() < needed {
        return Err(KlvEncodeError::BufferTooSmall {
            needed,
            got: out.len(),
        });
    }
    out[..16].copy_from_slice(&label.0);
    let written = write_ber(total_inner, &mut out[16..])?;
    out[16 + written..16 + written + total_inner].copy_from_slice(&tmp);
    Ok(16 + written + total_inner)
}

// Keep the unused-import clearer for refactors; LengthEncoding is part of the
// public API surface that future callers will use.
#[allow(dead_code)]
fn _unused(_: LengthEncoding, _: KlvDecodeError) {
    let _ = length::ber_oid_len;
    let _ = read_ber;
    let _ = read_ber_oid;
    let _ = ber_oid_len;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_field_into_owned() {
        let raw = RawField {
            tag: 13,
            value: &[1, 2, 3],
        };
        let owned: OwnedRawField = (&raw).into();
        assert_eq!(owned.tag, 13);
        assert_eq!(owned.value, vec![1, 2, 3]);
    }

    #[test]
    fn owned_as_ref() {
        let owned = OwnedRawField {
            tag: 5,
            value: vec![0xAA, 0xBB],
        };
        let r = owned.as_ref();
        assert_eq!(r.tag, 5);
        assert_eq!(r.value, &[0xAA, 0xBB]);
    }
}
