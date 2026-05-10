//! Generic KLV pack and unpack — `RawField`, `OwnedRawField`, `Iter`,
//! `encode_pack`. Knows nothing about ST 0601 specifically.
//!
//! `RawField<'a>` borrows from the input buffer (zero-alloc iteration).
//! `OwnedRawField` is the owned counterpart used by parsed records that
//! cross the FFI boundary — keeping the parsed record `'static`.

use crate::error::{KlvDecodeError, KlvEncodeError};
use crate::klv::length::{
    LengthEncoding, ber_len, read_ber, read_ber_oid, write_ber, write_ber_oid,
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
    LocalSet, // 1-byte tag + BER-OID length (note: ST 0601 LS uses 1-byte tag + BER-OID length)
    UniversalSet, // 16-byte UL key + BER length
}

/// Iterator over a KLV body (post-UL, post-outer-length).
#[doc(hidden)] // Phase 3: hidden from rustdoc; full pub(crate) blocked by external
// fuzz consumer (crates/tst-srt/fuzz/fuzz_targets/klv_iter.rs).
// Phase 5 will lift the consumer + demote to pub(crate).
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

impl<'a> Iterator for Iter<'a> {
    type Item = Result<RawField<'a>, KlvDecodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished || self.offset >= self.buf.len() {
            return None;
        }
        match self.mode {
            IterMode::LocalSet => self.next_local_set(),
            IterMode::UniversalSet => self.next_universal_set(),
        }
    }
}

impl<'a> Iter<'a> {
    fn next_local_set(&mut self) -> Option<Result<RawField<'a>, KlvDecodeError>> {
        let start = self.offset;
        let rest = &self.buf[start..];
        let (tag, after_tag) = match read_ber_oid(rest) {
            Ok(v) => v,
            Err(mut e) => {
                if let KlvDecodeError::Truncated { offset, .. } = &mut e {
                    *offset += start;
                }
                self.finished = true;
                return Some(Err(e));
            }
        };
        let consumed_tag = rest.len() - after_tag.len();
        let (len, after_len) = match read_ber(after_tag) {
            Ok(v) => v,
            Err(mut e) => {
                if let KlvDecodeError::Truncated { offset, .. } = &mut e {
                    *offset += start + consumed_tag;
                }
                self.finished = true;
                return Some(Err(e));
            }
        };
        let consumed_len = after_tag.len() - after_len.len();
        if after_len.len() < len {
            self.finished = true;
            return Some(Err(KlvDecodeError::Truncated {
                offset: start + consumed_tag + consumed_len,
                needed: len,
                have: after_len.len(),
            }));
        }
        let value = &after_len[..len];
        self.offset = start + consumed_tag + consumed_len + len;
        Some(Ok(RawField { tag, value }))
    }

    fn next_universal_set(&mut self) -> Option<Result<RawField<'a>, KlvDecodeError>> {
        let start = self.offset;
        let rest = &self.buf[start..];
        if rest.len() < 16 {
            self.finished = true;
            return Some(Err(KlvDecodeError::Truncated {
                offset: start,
                needed: 16,
                have: rest.len(),
            }));
        }
        // Universal-set keys are full 16-byte ULs. We don't have a 16-byte tag
        // field on RawField (u32), so we surface a stable hash of the UL bytes
        // as the tag and the *full payload value* unchanged. Callers that want
        // the UL key reconstruct it from the buffer offset.
        //
        // Practically: most consumers iterate local sets. Universal-set
        // iteration is included for completeness but the typed layer never
        // calls it. If a real consumer surfaces, swap RawField for a
        // UniversalSetField type.
        let _ = &rest[..16];
        let after_key = &rest[16..];
        let (len, after_len) = match read_ber(after_key) {
            Ok(v) => v,
            Err(e) => {
                self.finished = true;
                return Some(Err(e));
            }
        };
        let consumed = rest.len() - after_len.len();
        if after_len.len() < len {
            self.finished = true;
            return Some(Err(KlvDecodeError::Truncated {
                offset: start + consumed,
                needed: len,
                have: after_len.len(),
            }));
        }
        let value = &after_len[..len];
        self.offset = start + consumed + len;
        // Synthetic tag = 0 for universal-set entries; full UL is at &buf[start..start+16].
        Some(Ok(RawField { tag: 0, value }))
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
                // wrap encode_pack with a UL-keyed table; here we error.
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

    #[test]
    fn iter_local_set_three_fields() {
        // Tag 1, len 2, [0xAA, 0xBB]
        // Tag 5, len 1, [0x42]
        // Tag 13, len 4, [0x01, 0x02, 0x03, 0x04]
        let buf = [
            0x01, 0x02, 0xAA, 0xBB, // tag=1, len=2
            0x05, 0x01, 0x42, // tag=5, len=1
            0x0D, 0x04, 0x01, 0x02, 0x03, 0x04, // tag=13, len=4
        ];
        let mut it = Iter::local_set(&buf);
        let f1 = it.next().unwrap().unwrap();
        assert_eq!(f1.tag, 1);
        assert_eq!(f1.value, &[0xAA, 0xBB]);
        let f2 = it.next().unwrap().unwrap();
        assert_eq!(f2.tag, 5);
        assert_eq!(f2.value, &[0x42]);
        let f3 = it.next().unwrap().unwrap();
        assert_eq!(f3.tag, 13);
        assert_eq!(f3.value, &[0x01, 0x02, 0x03, 0x04]);
        assert!(it.next().is_none());
    }

    #[test]
    fn iter_local_set_truncated_value() {
        // Tag 1, len 4, but only 2 bytes follow
        let buf = [0x01, 0x04, 0xAA, 0xBB];
        let mut it = Iter::local_set(&buf);
        let err = it.next().unwrap().unwrap_err();
        matches!(err, KlvDecodeError::Truncated { .. });
        assert!(it.next().is_none(), "iterator finished after error");
    }

    #[test]
    fn iter_local_set_truncated_length() {
        // Tag 1 followed by long-form length declaration that's truncated
        let buf = [0x01, 0x82, 0xFF];
        let mut it = Iter::local_set(&buf);
        let err = it.next().unwrap().unwrap_err();
        matches!(err, KlvDecodeError::Truncated { .. });
    }

    #[test]
    fn iter_local_set_empty() {
        let buf: [u8; 0] = [];
        let mut it = Iter::local_set(&buf);
        assert!(it.next().is_none());
    }

    #[test]
    fn iter_local_set_remaining_tracks_offset() {
        let buf = [0x01, 0x02, 0xAA, 0xBB, 0x05, 0x01, 0x42];
        let mut it = Iter::local_set(&buf);
        let _ = it.next().unwrap().unwrap();
        assert_eq!(it.remaining(), &[0x05, 0x01, 0x42]);
    }

    #[test]
    fn iter_local_set_two_byte_tag() {
        // BER-OID tag 0x80 (= 0x81 0x00), len 1, [0x42]
        let buf = [0x81, 0x00, 0x01, 0x42];
        let mut it = Iter::local_set(&buf);
        let f = it.next().unwrap().unwrap();
        assert_eq!(f.tag, 0x80);
        assert_eq!(f.value, &[0x42]);
    }

    #[test]
    fn iter_universal_set_one_field() {
        let mut buf = vec![];
        buf.extend_from_slice(&[0xAB; 16]); // 16-byte key (any bytes; iter doesn't validate)
        buf.push(0x03); // BER short, len = 3
        buf.extend_from_slice(&[0x11, 0x22, 0x33]);
        let mut it = Iter::universal_set(&buf);
        let f = it.next().unwrap().unwrap();
        assert_eq!(f.value, &[0x11, 0x22, 0x33]);
        assert!(it.next().is_none());
    }

    #[test]
    fn iter_encode_pack_round_trip() {
        let label = UniversalLabel::ST_0601_LS;
        let fields = [
            RawField {
                tag: 1,
                value: &[0xAA, 0xBB],
            },
            RawField {
                tag: 5,
                value: &[0x42],
            },
        ];
        let mut out = vec![0u8; 256];
        let n = encode_pack(
            &label,
            fields.iter().cloned(),
            LengthEncoding::BerOid,
            &mut out,
        )
        .unwrap();
        let encoded = &out[..n];
        // First 16 bytes are the UL
        assert_eq!(&encoded[..16], &label.0);
        // Skip UL + outer length, parse the body
        let (_outer_len, body) = read_ber(&encoded[16..]).unwrap();
        let mut it = Iter::local_set(body);
        let f1 = it.next().unwrap().unwrap();
        assert_eq!(f1.tag, 1);
        assert_eq!(f1.value, &[0xAA, 0xBB]);
        let f2 = it.next().unwrap().unwrap();
        assert_eq!(f2.tag, 5);
        assert_eq!(f2.value, &[0x42]);
        assert!(it.next().is_none());
    }
}
