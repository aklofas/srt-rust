//! AV1 LEB128 (Little Endian Base 128) decoder.
//!
//! Per AV1 spec §4.10.5 (`leb128()`). Up to 8 bytes; each byte's
//! 0x80 bit signals continuation; the low 7 bits accumulate.

use crate::codec::ParseError;

/// Decode one LEB128 value from `buf` starting at `offset`. Returns
/// `(value, bytes_consumed)`. Errors per AV1 spec: continuation past
/// 8 bytes, or buffer exhausted before terminator.
pub fn read_leb128(buf: &[u8], offset: usize) -> Result<(u64, usize), ParseError> {
    let mut value: u64 = 0;
    let mut consumed = 0usize;
    for i in 0..8 {
        let pos = offset + i;
        if pos >= buf.len() {
            return Err(ParseError::InvalidLeb128 {
                offset_bytes: offset as u32,
            });
        }
        let byte = buf[pos];
        value |= u64::from(byte & 0x7F) << (i * 7);
        consumed += 1;
        if byte & 0x80 == 0 {
            return Ok((value, consumed));
        }
    }
    // 8 bytes consumed and last byte still had continuation bit set —
    // malformed per spec.
    Err(ParseError::InvalidLeb128 {
        offset_bytes: offset as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leb128_single_byte_zero() {
        let (v, n) = read_leb128(&[0x00], 0).unwrap();
        assert_eq!(v, 0);
        assert_eq!(n, 1);
    }

    #[test]
    fn leb128_single_byte_max() {
        // 0x7F = 127 (no continuation).
        let (v, n) = read_leb128(&[0x7F], 0).unwrap();
        assert_eq!(v, 127);
        assert_eq!(n, 1);
    }

    #[test]
    fn leb128_two_bytes() {
        // 0x80 | 0x01 → low 7 bits = 0; high 7 bits = 1; total = 128.
        let (v, n) = read_leb128(&[0x80, 0x01], 0).unwrap();
        assert_eq!(v, 128);
        assert_eq!(n, 2);
    }

    #[test]
    fn leb128_truncated_returns_err() {
        let r = read_leb128(&[0x80], 0);
        assert!(matches!(r, Err(ParseError::InvalidLeb128 { .. })));
    }

    #[test]
    fn leb128_overlong_returns_err() {
        let buf = [0x80; 9];
        let r = read_leb128(&buf, 0);
        assert!(matches!(r, Err(ParseError::InvalidLeb128 { .. })));
    }
}
