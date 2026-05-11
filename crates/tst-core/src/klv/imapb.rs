//! ST 1201.5 §7 IMAPB — bit-packed mapping between signed integers and a
//! defined floating-point range.
//!
//! Given parameters `(min, max, length)`:
//! - Value range is `[min, max]` (assumes `min < max`).
//! - The integer occupies `length` bytes, big-endian.
//! - Scale factor `sF = 2^(bPow − dPow)` where `bPow = ceil(log2(max − min))`
//!   and `dPow = 8L − 1` (= 8 * length − 1) — equivalently
//!   `2^bPow / 2^(8L−1)` (per ST 1201.5 §7.1.2 PDF p.5; numerator/denominator
//!   ordering matches the spec form).
//! - Encode: `i = round((value − min) / sF) − 2^(8L−1)`.
//! - Decode: `value = sF * (i + 2^(8L−1)) + min`.
//!
//! Special integer values per ST 1201.5 §7.2.3 (PDF p.8) are not modeled
//! here — escape-hatch users handle them at the next layer if needed. ST
//! 0601 fixed-range mappings (which use a slightly different convention
//! with INT_MIN as INVALID) live in `klv::st0601::mapping`.

use crate::error::{KlvEncodeError, KlvFieldError};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImapbParams {
    pub min: f64,
    pub max: f64,
    /// Encoded width in bytes. Must be in `1..=7`. `length >= 8` is
    /// permitted by ST 1201.5 but unsupported here (the implementation
    /// uses i64 arithmetic which overflows at L=8 — `signed_offset`
    /// would compute `2^63 > i64::MAX`). `length == 0` is degenerate.
    /// `encode_imapb` and `decode_imapb` return `UnsupportedImapbLength`
    /// for out-of-range values. In-tree consumers use L ∈ {1,2,3,4,5,6}.
    pub length: usize,
}

impl ImapbParams {
    /// Scale factor `sF`.
    fn scale(&self) -> f64 {
        let span = self.max - self.min;
        // 2^(ceil(log2(span))) — smallest power of two ≥ span.
        let log2_ceil = span.log2().ceil();
        let pow2 = 2f64.powf(log2_ceil);
        pow2 / 2f64.powi(8 * self.length as i32 - 1)
    }

    fn signed_offset(&self) -> i64 {
        2i64.pow(8 * self.length as u32 - 1)
    }
}

pub fn encode_imapb(p: &ImapbParams, value: f64, out: &mut [u8]) -> Result<(), KlvEncodeError> {
    // ST 1201.5 §7.1.2 defines IMAPB for any L-byte mapping, but this
    // implementation uses i64 arithmetic internally, which overflows
    // for length >= 8 (the signed_offset 2^(8L-1) would exceed i64::MAX).
    // length == 0 is a degenerate case (no bytes to encode into).
    if !(1..=7).contains(&p.length) {
        return Err(KlvEncodeError::UnsupportedImapbLength { length: p.length });
    }
    if out.len() < p.length {
        return Err(KlvEncodeError::BufferTooSmall {
            needed: p.length,
            got: out.len(),
        });
    }
    if !(value.is_finite()) || value < p.min || value > p.max {
        return Err(KlvEncodeError::OutOfRange {
            tag: 0,
            value,
            min: p.min,
            max: p.max,
        });
    }
    let sf = p.scale();
    let signed = ((value - p.min) / sf).round() as i64 - p.signed_offset();
    write_signed_be(signed, &mut out[..p.length]);
    Ok(())
}

pub fn decode_imapb(p: &ImapbParams, bytes: &[u8]) -> Result<f64, KlvFieldError> {
    // Same cap as encode_imapb — see UnsupportedImapbLength rationale.
    if !(1..=7).contains(&p.length) {
        return Err(KlvFieldError::UnsupportedImapbLength { length: p.length });
    }
    if bytes.len() != p.length {
        return Err(KlvFieldError::InvalidLength {
            tag: 0,
            expected: p.length,
            got: bytes.len(),
        });
    }
    let signed = read_signed_be(bytes);
    let sf = p.scale();
    let value = sf * (signed + p.signed_offset()) as f64 + p.min;
    Ok(value)
}

/// Write a signed integer to `out` in big-endian, two's complement.
fn write_signed_be(value: i64, out: &mut [u8]) {
    let n = out.len();
    let mask = if n == 8 {
        u64::MAX
    } else {
        (1u64 << (n as u32 * 8)) - 1
    };
    let bits = (value as u64) & mask;
    for (i, slot) in out.iter_mut().enumerate().take(n) {
        *slot = ((bits >> (8 * (n - 1 - i))) & 0xFF) as u8;
    }
}

/// Read a signed integer from `bytes` (big-endian, two's complement).
fn read_signed_be(bytes: &[u8]) -> i64 {
    let n = bytes.len();
    let mut bits: u64 = 0;
    for &b in bytes {
        bits = (bits << 8) | b as u64;
    }
    let sign_bit = 1u64 << (n as u32 * 8 - 1);
    if bits & sign_bit != 0 {
        // Sign-extend
        let extension = !((1u64 << (n as u32 * 8)) - 1);
        (bits | extension) as i64
    } else {
        bits as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_2byte_unit_range() {
        let p = ImapbParams {
            min: -1.0,
            max: 1.0,
            length: 2,
        };
        for value in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            let mut buf = [0u8; 2];
            encode_imapb(&p, value, &mut buf).unwrap();
            let back = decode_imapb(&p, &buf).unwrap();
            assert!(
                (back - value).abs() < 1e-4,
                "round trip failed: input={value}, output={back}"
            );
        }
    }

    #[test]
    fn round_trip_4byte_signed_range() {
        let p = ImapbParams {
            min: -180.0,
            max: 180.0,
            length: 4,
        };
        for value in [-180.0, -90.0, 0.0, 45.5, 179.999] {
            let mut buf = [0u8; 4];
            encode_imapb(&p, value, &mut buf).unwrap();
            let back = decode_imapb(&p, &buf).unwrap();
            assert!((back - value).abs() < 1e-6, "v={value}, back={back}");
        }
    }

    #[test]
    fn out_of_range_rejected() {
        let p = ImapbParams {
            min: -1.0,
            max: 1.0,
            length: 2,
        };
        let mut buf = [0u8; 2];
        let err = encode_imapb(&p, 2.0, &mut buf).unwrap_err();
        matches!(err, KlvEncodeError::OutOfRange { .. });
    }

    #[test]
    fn nan_rejected() {
        let p = ImapbParams {
            min: -1.0,
            max: 1.0,
            length: 2,
        };
        let mut buf = [0u8; 2];
        let err = encode_imapb(&p, f64::NAN, &mut buf).unwrap_err();
        matches!(err, KlvEncodeError::OutOfRange { .. });
    }

    #[test]
    fn buffer_too_small_rejected() {
        let p = ImapbParams {
            min: -1.0,
            max: 1.0,
            length: 4,
        };
        let mut buf = [0u8; 2];
        let err = encode_imapb(&p, 0.0, &mut buf).unwrap_err();
        matches!(err, KlvEncodeError::BufferTooSmall { .. });
    }

    #[test]
    fn decode_wrong_length_rejected() {
        let p = ImapbParams {
            min: -1.0,
            max: 1.0,
            length: 2,
        };
        let buf = [0u8; 3];
        let err = decode_imapb(&p, &buf).unwrap_err();
        matches!(err, KlvFieldError::InvalidLength { .. });
    }

    // --- Spec-vector regression tests (added 2026-05-10 for ST 1201.5 §10 + ST 0903.6 §10.1) ---

    #[test]
    fn st_0903_section_10_1_11_fov_12_5_deg() {
        // ST 0903.6 §10.1.11 worked example: IMAPB(0, 180, 2) for 12.5° → 0x0640.
        let p = ImapbParams { min: 0.0, max: 180.0, length: 2 };
        let mut buf = [0u8; 2];
        encode_imapb(&p, 12.5, &mut buf).unwrap();
        assert_eq!(buf, [0x06, 0x40], "spec says 0x0640, got {:#04X}{:02X}", buf[0], buf[1]);
        let back = decode_imapb(&p, &buf).unwrap();
        assert!((back - 12.5).abs() < 1e-2, "decoded {back}, expected 12.5");
    }

    #[test]
    fn st_0903_section_10_1_12_fov_10_0_deg() {
        // ST 0903.6 §10.1.12 worked example: IMAPB(0, 180, 2) for 10.0° → 0x0500.
        let p = ImapbParams { min: 0.0, max: 180.0, length: 2 };
        let mut buf = [0u8; 2];
        encode_imapb(&p, 10.0, &mut buf).unwrap();
        assert_eq!(buf, [0x05, 0x00]);
    }

    #[test]
    fn st_0903_section_10_1_11_fov_90_0_deg() {
        // Mid-range cross-check: IMAPB(0, 180, 2) for 90.0° → 0x2D00 (= 128 * 90 = 11520).
        // Pre-fix code emits 0xAD00 (MSB flipped by signed-midpoint shift).
        let p = ImapbParams { min: 0.0, max: 180.0, length: 2 };
        let mut buf = [0u8; 2];
        encode_imapb(&p, 90.0, &mut buf).unwrap();
        assert_eq!(buf, [0x2D, 0x00]);
    }

    #[test]
    fn st_1201_5_appendix_a_test_2_unsigned_be() {
        // ST 1201.5 Appendix A Test 2: IMAPB(0.0, 100.0, 3) value 100 → 0x640000.
        let p = ImapbParams { min: 0.0, max: 100.0, length: 3 };
        let mut buf = [0u8; 3];
        encode_imapb(&p, 100.0, &mut buf).unwrap();
        assert_eq!(buf, [0x64, 0x00, 0x00], "spec mandates unsigned BE; pre-fix code emits 0xE40000");
    }

    #[test]
    fn st_1201_5_appendix_a_test_3_zero_mapping() {
        // ST 1201.5 Appendix A Test 3: IMAPB(-9.9, 110.0, 3) value 0.0 → 0x09E667
        // (the Zero mapping case — requires Zoffset = sF*a - floor(sF*a)).
        let p = ImapbParams { min: -9.9, max: 110.0, length: 3 };
        let mut buf = [0u8; 3];
        encode_imapb(&p, 0.0, &mut buf).unwrap();
        assert_eq!(buf, [0x09, 0xE6, 0x67], "Zoffset rule unimplemented; pre-fix code emits 0x89E666");
        let back = decode_imapb(&p, &buf).unwrap();
        assert!(back.abs() < 1e-4, "Zero mapping must round-trip to 0.0, got {back}");
    }

    #[test]
    fn length_8_round_trip() {
        // ST 1201.5 allows any L; pre-fix code rejects L=8 due to i64 overflow.
        let p = ImapbParams { min: -1000.0, max: 1000.0, length: 8 };
        let mut buf = [0u8; 8];
        encode_imapb(&p, 123.456, &mut buf).unwrap();
        let back = decode_imapb(&p, &buf).unwrap();
        assert!((back - 123.456).abs() < 1e-9, "L=8 round-trip failed: {back}");
    }
}
