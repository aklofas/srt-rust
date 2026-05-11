//! ST 1201.5 §7 IMAPB — bit-packed mapping between unsigned integers and a
//! defined floating-point range.
//!
//! Given parameters `(min, max, length)` with `min < max` and `length ∈ 1..=8`:
//! - The integer occupies `length` bytes, big-endian, **unsigned** (ST 1201.5
//!   §7.2.3 Table 1 reserves MSB-set values for special-value indicators).
//! - Per ST 1201.5 §8.9 Summary:
//!   - `bPow = ceil(log2(max − min))`
//!   - `dPow = 8L − 1`
//!   - `sF = 2^(dPow − bPow)`  *(forward scale)*
//!   - `Zoffset = sF·min − floor(sF·min)` when `min<0 and max>0`; else 0
//! - Encode (§7.2.1): `y = truncate(sF·(value − min) + Zoffset)`, L-byte unsigned BE.
//! - Decode: `value = (y − Zoffset)·sR + min`, where `sR = 1/sF`.
//!
//! Special integer values per ST 1201.5 §7.2.3 (PDF p.8) are not modeled here —
//! escape-hatch users handle them at the next layer if needed. ST 0601 fixed-range
//! mappings (which use a different convention with INT_MIN as INVALID sentinel)
//! live in `klv::st0601::mapping`.

use crate::error::{KlvEncodeError, KlvFieldError};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImapbParams {
    pub min: f64,
    pub max: f64,
    /// Encoded width in bytes. Must be in `1..=8` (ST 1201.5 §6 allows any L;
    /// internal math uses `u64` which holds 8 bytes). L > 8 needs `u128`
    /// and is not currently supported.
    pub length: usize,
}

impl ImapbParams {
    /// Forward scale factor `sF = 2^(dPow − bPow)` per ST 1201.5 §8.9.
    /// Returns `f64::INFINITY` for the degenerate `max == min` case (caller
    /// pre-checks parameters; this function is internal).
    fn sf(&self) -> f64 {
        let span = self.max - self.min;
        let b_pow = span.log2().ceil();
        let d_pow = (8 * self.length as i32 - 1) as f64;
        2f64.powf(d_pow - b_pow)
    }

    /// Zero-Point offset per ST 1201.5 §7.1.2 step 6.
    /// Only nonzero when the range straddles zero (`min < 0 < max`).
    fn z_offset(&self) -> f64 {
        if self.min < 0.0 && self.max > 0.0 {
            let scaled = self.sf() * self.min;
            scaled - scaled.floor()
        } else {
            0.0
        }
    }

    // TODO(Task 3): the two helpers below exist only so `decode_imapb` still
    // compiles between Task 2 (encode rewrite) and Task 3 (decode rewrite).
    // Task 3 removes both call sites and these helpers.
    fn scale(&self) -> f64 {
        let span = self.max - self.min;
        let log2_ceil = span.log2().ceil();
        let pow2 = 2f64.powf(log2_ceil);
        pow2 / 2f64.powi(8 * self.length as i32 - 1)
    }

    fn signed_offset(&self) -> i64 {
        2i64.pow(8 * self.length as u32 - 1)
    }
}

pub fn encode_imapb(p: &ImapbParams, value: f64, out: &mut [u8]) -> Result<(), KlvEncodeError> {
    // ST 1201.5 §6 allows any L; internal math uses u64 (max 8 bytes).
    if !(1..=8).contains(&p.length) {
        return Err(KlvEncodeError::UnsupportedImapbLength { length: p.length });
    }
    if out.len() < p.length {
        return Err(KlvEncodeError::BufferTooSmall {
            needed: p.length,
            got: out.len(),
        });
    }
    if !value.is_finite() || value < p.min || value > p.max {
        return Err(KlvEncodeError::OutOfRange {
            tag: 0,
            value,
            min: p.min,
            max: p.max,
        });
    }
    // ST 1201.5 §7.2.1 step 4a: y = truncate(sF*(x - min) + Zoffset).
    // `truncate` = `.floor()` for x >= min (which the bounds check above guarantees).
    let y_f = p.sf() * (value - p.min) + p.z_offset();
    let y = y_f.floor() as u64;
    let mask = if p.length == 8 {
        u64::MAX
    } else {
        (1u64 << (8 * p.length as u32)) - 1
    };
    let y_clamped = y & mask;
    for (i, slot) in out.iter_mut().enumerate().take(p.length) {
        *slot = ((y_clamped >> (8 * (p.length - 1 - i))) & 0xFF) as u8;
    }
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

// Bridge: encode rewrite in Task 2 no longer needs this; Task 4 deletes both
// `write_signed_be` and `read_signed_be` after Task 3 rewrites decode.
#[allow(dead_code)]
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
        let p = ImapbParams {
            min: 0.0,
            max: 180.0,
            length: 2,
        };
        let mut buf = [0u8; 2];
        encode_imapb(&p, 12.5, &mut buf).unwrap();
        assert_eq!(
            buf,
            [0x06, 0x40],
            "spec says 0x0640, got {:#04X}{:02X}",
            buf[0],
            buf[1]
        );
        let back = decode_imapb(&p, &buf).unwrap();
        assert!((back - 12.5).abs() < 1e-2, "decoded {back}, expected 12.5");
    }

    #[test]
    fn st_0903_section_10_1_12_fov_10_0_deg() {
        // ST 0903.6 §10.1.12 worked example: IMAPB(0, 180, 2) for 10.0° → 0x0500.
        let p = ImapbParams {
            min: 0.0,
            max: 180.0,
            length: 2,
        };
        let mut buf = [0u8; 2];
        encode_imapb(&p, 10.0, &mut buf).unwrap();
        assert_eq!(buf, [0x05, 0x00]);
    }

    #[test]
    fn st_0903_section_10_1_11_fov_90_0_deg() {
        // Mid-range cross-check: IMAPB(0, 180, 2) for 90.0° → 0x2D00 (= 128 * 90 = 11520).
        // Pre-fix code emits 0xAD00 (MSB flipped by signed-midpoint shift).
        let p = ImapbParams {
            min: 0.0,
            max: 180.0,
            length: 2,
        };
        let mut buf = [0u8; 2];
        encode_imapb(&p, 90.0, &mut buf).unwrap();
        assert_eq!(buf, [0x2D, 0x00]);
    }

    #[test]
    fn st_1201_5_appendix_a_test_2_unsigned_be() {
        // ST 1201.5 Appendix A Test 2: IMAPB(0.0, 100.0, 3) value 100 → 0x640000.
        let p = ImapbParams {
            min: 0.0,
            max: 100.0,
            length: 3,
        };
        let mut buf = [0u8; 3];
        encode_imapb(&p, 100.0, &mut buf).unwrap();
        assert_eq!(
            buf,
            [0x64, 0x00, 0x00],
            "spec mandates unsigned BE; pre-fix code emits 0xE40000"
        );
    }

    #[test]
    fn st_1201_5_appendix_a_test_3_zero_mapping() {
        // ST 1201.5 Appendix A Test 3: IMAPB(-9.9, 110.0, 3) value 0.0 → 0x09E667
        // (the Zero mapping case — requires Zoffset = sF*a - floor(sF*a)).
        let p = ImapbParams {
            min: -9.9,
            max: 110.0,
            length: 3,
        };
        let mut buf = [0u8; 3];
        encode_imapb(&p, 0.0, &mut buf).unwrap();
        assert_eq!(
            buf,
            [0x09, 0xE6, 0x67],
            "Zoffset rule unimplemented; pre-fix code emits 0x89E666"
        );
        let back = decode_imapb(&p, &buf).unwrap();
        assert!(
            back.abs() < 1e-4,
            "Zero mapping must round-trip to 0.0, got {back}"
        );
    }

    #[test]
    fn length_8_round_trip() {
        // ST 1201.5 allows any L; pre-fix code rejects L=8 due to i64 overflow.
        let p = ImapbParams {
            min: -1000.0,
            max: 1000.0,
            length: 8,
        };
        let mut buf = [0u8; 8];
        encode_imapb(&p, 123.456, &mut buf).unwrap();
        let back = decode_imapb(&p, &buf).unwrap();
        assert!(
            (back - 123.456).abs() < 1e-9,
            "L=8 round-trip failed: {back}"
        );
    }
}
