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

use crate::error::{KlvDecodeError, KlvEncodeError, KlvFieldError};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImapbParams {
    pub min: f64,
    pub max: f64,
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

// Avoid unused warnings on KlvDecodeError import — placeholder for future use
// at the call sites that use this codec.
#[allow(dead_code)]
fn _unused(_: KlvDecodeError) {}

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
}
