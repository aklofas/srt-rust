//! Variable-length truncated big-endian unsigned codec used pervasively
//! by ST 0903.6 LS and Pack value bytes per §9.1.
//!
//! Wire form: 1..=`max_bytes` raw bytes, big-endian, with leading zero
//! bytes elided. Value 0 encodes as `[0x00]` (single byte).
//!
//! Distinct from BER-OID — BER-OID uses a continuation-bit-set 7-bit
//! base-128 encoding; VarUint uses raw 8-bit big-endian with leading
//! zeros stripped. Both appear in ST 0903.6: BER-OID for the leading
//! `targetId` of a VTargetPack (§10.2.2.1), VarUint for inline value
//! bytes elsewhere (§9.1).
//!
//! u32 and u64 variants — `read_var_u32`/`write_var_u32`/`var_u32_len` for
//! ST 0601/0102 and smaller fields; `read_var_u64`/`write_var_u64`/`var_u64_len`
//! for ST 0903 V6 pixel numbers (up to 6 wire bytes) and wide fields.
//!
//! The per-tag spec-mandated max width (V2, V3, V4, V6) is enforced
//! by the decode caller (raises `LengthOverrun` / `InvalidLength`
//! when the BER outer length exceeds the tag's spec cap). The u32
//! codec accepts 1..=4 byte input; the u64 variants accept 1..=8.

use crate::error::KlvFieldError;
use alloc::vec::Vec;

/// Decode `bytes` as a truncated big-endian uint up to 4 bytes wide.
/// `bytes.len()` must be 1..=4.
pub(crate) fn read_var_u32(bytes: &[u8]) -> Result<u32, KlvFieldError> {
    if bytes.is_empty() || bytes.len() > 4 {
        return Err(KlvFieldError::InvalidLength {
            tag: 0, // caller knows the tag; this is a generic helper
            expected: 4,
            got: bytes.len(),
        });
    }
    let mut buf = [0u8; 4];
    buf[4 - bytes.len()..].copy_from_slice(bytes);
    Ok(u32::from_be_bytes(buf))
}

/// Number of wire bytes that `value` will encode to (1..=4).
pub(crate) fn var_u32_len(value: u32) -> usize {
    if value == 0 {
        1
    } else {
        let leading_zero_bytes = (value.leading_zeros() / 8) as usize;
        4 - leading_zero_bytes
    }
}

/// Write `value` as a truncated big-endian uint into `out`. Returns
/// bytes written (1..=4).
pub(crate) fn write_var_u32(value: u32, out: &mut Vec<u8>) -> usize {
    let n = var_u32_len(value);
    let bytes = value.to_be_bytes();
    out.extend_from_slice(&bytes[4 - n..]);
    n
}

/// `u64` sibling of [`read_var_u32`] — ST 0903.6 V6 pixel numbers permit up
/// to 6 wire bytes (§10.2.2.2). Accepts 1..=8 bytes.
pub(crate) fn read_var_u64(bytes: &[u8]) -> Result<u64, KlvFieldError> {
    if bytes.is_empty() || bytes.len() > 8 {
        return Err(KlvFieldError::InvalidLength {
            tag: 0, // caller knows the tag; this is a generic helper
            expected: 8,
            got: bytes.len(),
        });
    }
    let mut buf = [0u8; 8];
    buf[8 - bytes.len()..].copy_from_slice(bytes);
    Ok(u64::from_be_bytes(buf))
}

/// Number of wire bytes that `value` will encode to (1..=8).
pub(crate) fn var_u64_len(value: u64) -> usize {
    if value == 0 {
        1
    } else {
        let leading_zero_bytes = (value.leading_zeros() / 8) as usize;
        8 - leading_zero_bytes
    }
}

/// `u64` sibling of [`write_var_u32`]. Returns bytes written (1..=8).
pub(crate) fn write_var_u64(value: u64, out: &mut Vec<u8>) -> usize {
    let n = var_u64_len(value);
    let bytes = value.to_be_bytes();
    out.extend_from_slice(&bytes[8 - n..]);
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_zero() {
        let mut out = Vec::new();
        let n = write_var_u32(0, &mut out);
        assert_eq!(n, 1);
        assert_eq!(out, vec![0x00]);
        assert_eq!(read_var_u32(&out).unwrap(), 0);
    }

    #[test]
    fn round_trip_one_byte_max() {
        let mut out = Vec::new();
        let n = write_var_u32(0xFF, &mut out);
        assert_eq!(n, 1);
        assert_eq!(out, vec![0xFF]);
        assert_eq!(read_var_u32(&out).unwrap(), 0xFF);
    }

    #[test]
    fn round_trip_two_byte() {
        let mut out = Vec::new();
        let n = write_var_u32(0x1234, &mut out);
        assert_eq!(n, 2);
        assert_eq!(out, vec![0x12, 0x34]);
        assert_eq!(read_var_u32(&out).unwrap(), 0x1234);
    }

    #[test]
    fn round_trip_three_byte() {
        let mut out = Vec::new();
        let n = write_var_u32(0x123456, &mut out);
        assert_eq!(n, 3);
        assert_eq!(out, vec![0x12, 0x34, 0x56]);
        assert_eq!(read_var_u32(&out).unwrap(), 0x123456);
    }

    #[test]
    fn round_trip_four_byte_max() {
        let mut out = Vec::new();
        let n = write_var_u32(u32::MAX, &mut out);
        assert_eq!(n, 4);
        assert_eq!(out, vec![0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(read_var_u32(&out).unwrap(), u32::MAX);
    }

    #[test]
    fn empty_input_rejected() {
        let err = read_var_u32(&[]).unwrap_err();
        assert!(matches!(err, KlvFieldError::InvalidLength { .. }));
    }

    #[test]
    fn over_four_bytes_rejected() {
        let err = read_var_u32(&[0; 5]).unwrap_err();
        assert!(matches!(err, KlvFieldError::InvalidLength { .. }));
    }

    #[test]
    fn var_u32_len_boundaries() {
        assert_eq!(var_u32_len(0), 1);
        assert_eq!(var_u32_len(0x7F), 1);
        assert_eq!(var_u32_len(0xFF), 1);
        assert_eq!(var_u32_len(0x100), 2);
        assert_eq!(var_u32_len(0xFFFF), 2);
        assert_eq!(var_u32_len(0x10000), 3);
        assert_eq!(var_u32_len(0xFFFFFF), 3);
        assert_eq!(var_u32_len(0x1000000), 4);
        assert_eq!(var_u32_len(u32::MAX), 4);
    }

    // ---------- var-uint u64 ----------

    #[test]
    fn var_u64_round_trip_above_u32() {
        for v in [0u64, 0xFF, u32::MAX as u64, (u32::MAX as u64) + 1, 0xFFFF_FFFF_FFFF, u64::MAX] {
            let mut out = Vec::new();
            let n = write_var_u64(v, &mut out);
            assert_eq!(n, var_u64_len(v));
            assert_eq!(read_var_u64(&out).unwrap(), v, "round trip failed for v={v}");
        }
    }

    #[test]
    fn var_u64_len_boundaries() {
        assert_eq!(var_u64_len(0), 1);
        assert_eq!(var_u64_len(0xFF), 1);
        assert_eq!(var_u64_len(0x1_0000_0000), 5);
        assert_eq!(var_u64_len(u64::MAX), 8);
    }
}
