//! BER short/long and BER-OID length codecs.
//!
//! BER short form: single byte `0x00..0x7F`.
//! BER long form: `0x80 | n` followed by `n` big-endian length bytes (n in 1..=8).
//! BER-OID: base-128 integer, big-endian, MSB set on every non-final byte.

use crate::error::{KlvDecodeError, KlvEncodeError};

/// Which length encoding a generic pack uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthEncoding {
    /// BER short form only (≤ 127). Errors on values that don't fit.
    BerShort,
    /// BER long form only.
    BerLong,
    /// BER (auto-selects short or long).
    Ber,
    /// BER-OID variable-length form.
    BerOid,
    /// Fixed length, no encoding bytes.
    Fixed(usize),
}

/// Read a BER length (short or long) from `buf`. Returns `(length, &buf[consumed..])`.
pub fn read_ber(buf: &[u8]) -> Result<(usize, &[u8]), KlvDecodeError> {
    let first = buf.first().ok_or(KlvDecodeError::Truncated {
        offset: 0,
        needed: 1,
        have: 0,
    })?;
    if *first < 0x80 {
        // Short form
        Ok((*first as usize, &buf[1..]))
    } else {
        // Long form: 0x80 | n
        let n = (*first & 0x7F) as usize;
        if n == 0 {
            // Indefinite length — not allowed for KLV.
            return Err(KlvDecodeError::MalformedLength { offset: 0 });
        }
        if n > 8 {
            // Length-of-length too large to fit in usize on most platforms.
            return Err(KlvDecodeError::MalformedLength { offset: 0 });
        }
        if buf.len() < 1 + n {
            return Err(KlvDecodeError::Truncated {
                offset: 1,
                needed: n,
                have: buf.len() - 1,
            });
        }
        let mut value: u64 = 0;
        for &b in &buf[1..1 + n] {
            value = (value << 8) | b as u64;
        }
        if value > usize::MAX as u64 {
            return Err(KlvDecodeError::LengthOverflow { value });
        }
        Ok((value as usize, &buf[1 + n..]))
    }
}

/// Read a BER-OID variable-length integer from `buf`. Returns `(value, &buf[consumed..])`.
pub fn read_ber_oid(buf: &[u8]) -> Result<(u32, &[u8]), KlvDecodeError> {
    let mut value: u64 = 0;
    let mut consumed = 0;
    loop {
        let b = buf.get(consumed).ok_or(KlvDecodeError::Truncated {
            offset: consumed,
            needed: 1,
            have: 0,
        })?;
        consumed += 1;
        value = (value << 7) | (*b as u64 & 0x7F);
        if value > u32::MAX as u64 {
            return Err(KlvDecodeError::LengthOverflow { value });
        }
        if b & 0x80 == 0 {
            return Ok((value as u32, &buf[consumed..]));
        }
        if consumed > 5 {
            // u32 fits in at most 5 BER-OID bytes.
            return Err(KlvDecodeError::MalformedTag { offset: 0 });
        }
    }
}

/// Number of bytes BER would use to encode `value`.
pub fn ber_len(value: usize) -> usize {
    if value < 0x80 {
        1
    } else {
        let mut bytes = 0usize;
        let mut v = value;
        while v > 0 {
            bytes += 1;
            v >>= 8;
        }
        1 + bytes
    }
}

/// Number of bytes BER-OID would use to encode `value`.
pub fn ber_oid_len(value: u32) -> usize {
    if value == 0 {
        return 1;
    }
    let mut bytes = 0usize;
    let mut v = value as u64;
    while v > 0 {
        bytes += 1;
        v >>= 7;
    }
    bytes
}

/// Write a BER length (auto short/long) to `out`. Returns bytes written.
pub fn write_ber(value: usize, out: &mut [u8]) -> Result<usize, KlvEncodeError> {
    let needed = ber_len(value);
    if out.len() < needed {
        return Err(KlvEncodeError::BufferTooSmall {
            needed,
            got: out.len(),
        });
    }
    if value < 0x80 {
        out[0] = value as u8;
        Ok(1)
    } else {
        let n = needed - 1;
        if n > 8 {
            return Err(KlvEncodeError::RecordTooLarge);
        }
        out[0] = 0x80 | n as u8;
        for i in 0..n {
            out[1 + i] = ((value >> (8 * (n - 1 - i))) & 0xFF) as u8;
        }
        Ok(needed)
    }
}

/// Write a BER-OID variable-length integer to `out`. Returns bytes written.
pub fn write_ber_oid(value: u32, out: &mut [u8]) -> Result<usize, KlvEncodeError> {
    let needed = ber_oid_len(value);
    if out.len() < needed {
        return Err(KlvEncodeError::BufferTooSmall {
            needed,
            got: out.len(),
        });
    }
    for (i, slot) in out.iter_mut().enumerate().take(needed) {
        let shift = 7 * (needed - 1 - i);
        let mut byte = ((value >> shift) & 0x7F) as u8;
        if i + 1 < needed {
            byte |= 0x80;
        }
        *slot = byte;
    }
    Ok(needed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- BER ----------

    #[test]
    fn ber_short_round_trip() {
        for v in [0usize, 1, 0x7E, 0x7F] {
            let mut buf = [0u8; 16];
            let n = write_ber(v, &mut buf).unwrap();
            assert_eq!(n, 1, "short form is one byte (v={v})");
            let (parsed, rest) = read_ber(&buf[..n]).unwrap();
            assert_eq!(parsed, v);
            assert!(rest.is_empty());
        }
    }

    #[test]
    fn ber_long_one_byte_boundary() {
        // 0x80 is the smallest long-form value
        let mut buf = [0u8; 16];
        let n = write_ber(0x80, &mut buf).unwrap();
        assert_eq!(n, 2);
        assert_eq!(buf[0], 0x81);
        assert_eq!(buf[1], 0x80);
        let (parsed, _) = read_ber(&buf[..n]).unwrap();
        assert_eq!(parsed, 0x80);
    }

    #[test]
    fn ber_long_two_byte_boundary() {
        let mut buf = [0u8; 16];
        let n = write_ber(0xFFFF, &mut buf).unwrap();
        assert_eq!(n, 3);
        assert_eq!(&buf[..n], &[0x82, 0xFF, 0xFF]);
    }

    #[test]
    fn ber_long_three_byte_boundary() {
        let mut buf = [0u8; 16];
        let n = write_ber(0x010000, &mut buf).unwrap();
        assert_eq!(n, 4);
        assert_eq!(&buf[..n], &[0x83, 0x01, 0x00, 0x00]);
    }

    #[test]
    fn ber_long_round_trip_sweep() {
        for v in [0xFFusize, 0x100, 0xFFFF, 0x10000, 0xFFFFFF, 0x1_000_000] {
            let mut buf = [0u8; 16];
            let n = write_ber(v, &mut buf).unwrap();
            let (parsed, rest) = read_ber(&buf[..n]).unwrap();
            assert_eq!(parsed, v, "round trip failed for v={v}");
            assert!(rest.is_empty());
        }
    }

    #[test]
    fn ber_buffer_too_small() {
        let mut buf = [0u8; 1];
        let err = write_ber(0x100, &mut buf).unwrap_err();
        matches!(err, KlvEncodeError::BufferTooSmall { needed: 3, got: 1 });
    }

    #[test]
    fn ber_truncated_long_form() {
        // 0x82 says "next 2 bytes are length" but we provide only 1
        let buf = [0x82, 0xFF];
        let err = read_ber(&buf).unwrap_err();
        matches!(err, KlvDecodeError::Truncated { .. });
    }

    #[test]
    fn ber_indefinite_form_rejected() {
        let buf = [0x80];
        let err = read_ber(&buf).unwrap_err();
        matches!(err, KlvDecodeError::MalformedLength { offset: 0 });
    }

    #[test]
    fn ber_long_form_too_many_bytes_rejected() {
        // 0x89 = "9 length bytes", bigger than u64
        let buf = [0x89, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        let err = read_ber(&buf).unwrap_err();
        matches!(err, KlvDecodeError::MalformedLength { offset: 0 });
    }

    #[test]
    fn ber_empty_input_truncated() {
        let buf: [u8; 0] = [];
        let err = read_ber(&buf).unwrap_err();
        matches!(err, KlvDecodeError::Truncated { .. });
    }

    // ---------- BER-OID ----------

    #[test]
    fn ber_oid_zero() {
        let mut buf = [0u8; 8];
        let n = write_ber_oid(0, &mut buf).unwrap();
        assert_eq!(n, 1);
        assert_eq!(buf[0], 0x00);
        let (parsed, _) = read_ber_oid(&buf[..n]).unwrap();
        assert_eq!(parsed, 0);
    }

    #[test]
    fn ber_oid_single_byte_max() {
        let mut buf = [0u8; 8];
        let n = write_ber_oid(0x7F, &mut buf).unwrap();
        assert_eq!(n, 1);
        assert_eq!(buf[0], 0x7F);
    }

    #[test]
    fn ber_oid_two_byte_min() {
        let mut buf = [0u8; 8];
        let n = write_ber_oid(0x80, &mut buf).unwrap();
        assert_eq!(n, 2);
        // 0x80 = 10000000_2; split into 7-bit groups: [1, 0]; with continuation: 0x81, 0x00.
        assert_eq!(&buf[..n], &[0x81, 0x00]);
    }

    #[test]
    fn ber_oid_round_trip_sweep() {
        for v in [
            0u32, 1, 0x7F, 0x80, 0xFF, 0x3FFF, 0x4000, 0xFFFF, 0xFFFFFFFF,
        ] {
            let mut buf = [0u8; 8];
            let n = write_ber_oid(v, &mut buf).unwrap();
            let (parsed, rest) = read_ber_oid(&buf[..n]).unwrap();
            assert_eq!(parsed, v, "round trip failed for v={v}");
            assert!(rest.is_empty());
        }
    }

    #[test]
    fn ber_oid_truncated_continuation() {
        // 0x80 alone has the continuation bit set but no following byte
        let buf = [0x80];
        let err = read_ber_oid(&buf).unwrap_err();
        matches!(err, KlvDecodeError::Truncated { .. });
    }

    #[test]
    fn ber_oid_too_long_rejected() {
        // 6 continuation bytes — overflows u32
        let buf = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F];
        let err = read_ber_oid(&buf).unwrap_err();
        matches!(
            err,
            KlvDecodeError::LengthOverflow { .. } | KlvDecodeError::MalformedTag { .. }
        );
    }

    #[test]
    fn ber_oid_buffer_too_small() {
        let mut buf = [0u8; 1];
        let err = write_ber_oid(0x80, &mut buf).unwrap_err();
        matches!(err, KlvEncodeError::BufferTooSmall { .. });
    }

    #[test]
    fn ber_oid_remaining_returns_correct_slice() {
        let buf = [0x05, 0xAA, 0xBB];
        let (v, rest) = read_ber_oid(&buf).unwrap();
        assert_eq!(v, 5);
        assert_eq!(rest, &[0xAA, 0xBB]);
    }
}
