//! Fixed-range linear int↔float helpers used by ST 0601 typed tags.
//!
//! Two flavors per `LinearRange`:
//! - Signed: integer in `[INT_MIN+1, INT_MAX]`, `INT_MIN` reserved as INVALID.
//! - Unsigned: integer in `[0, UINT_MAX]`, no INVALID.

use crate::error::{KlvEncodeError, KlvFieldError};
#[cfg(not(feature = "std"))]
use crate::float_ext::FloatExt;
use crate::klv::st0601::tags::LinearRange;

/// Encode a float value into `out` according to `range`.
/// `tag` is for error reporting only.
pub(crate) fn encode_fixed_range(
    range: &LinearRange,
    tag: u32,
    value: f64,
    out: &mut [u8],
) -> Result<(), KlvEncodeError> {
    if out.len() < range.byte_length {
        return Err(KlvEncodeError::BufferTooSmall {
            needed: range.byte_length,
            got: out.len(),
        });
    }
    if !value.is_finite() || value < range.min || value > range.max {
        return Err(KlvEncodeError::OutOfRange {
            tag,
            value,
            min: range.min,
            max: range.max,
        });
    }
    if range.signed {
        let int_max = signed_max(range.byte_length);
        let int_min_plus_one = -int_max;
        let span = range.max - range.min;
        let scale = span / (int_max as f64 - int_min_plus_one as f64);
        let midpoint = (range.min + range.max) / 2.0;
        let mut i = ((value - midpoint) / scale).round() as i64;
        if i > int_max {
            i = int_max;
        }
        if i < int_min_plus_one {
            i = int_min_plus_one;
        }
        write_signed_be(i, &mut out[..range.byte_length]);
    } else {
        let int_max = unsigned_max(range.byte_length);
        let span = range.max - range.min;
        let scale = span / int_max as f64;
        let mut i = ((value - range.min) / scale).round() as i64;
        if i > int_max {
            i = int_max;
        }
        if i < 0 {
            i = 0;
        }
        write_unsigned_be(i as u64, &mut out[..range.byte_length]);
    }
    Ok(())
}

/// Decode bytes into a float value according to `range`.
/// `tag` is for error reporting only.
pub(crate) fn decode_fixed_range(
    range: &LinearRange,
    tag: u32,
    bytes: &[u8],
) -> Result<f64, KlvFieldError> {
    if bytes.len() != range.byte_length {
        return Err(KlvFieldError::InvalidLength {
            tag,
            expected: range.byte_length,
            got: bytes.len(),
        });
    }
    if range.signed {
        let i = read_signed_be(bytes);
        let int_max = signed_max(range.byte_length);
        let int_min = -int_max - 1;
        if i == int_min {
            return Err(KlvFieldError::InvalidSentinel { tag });
        }
        let int_min_plus_one = int_min + 1;
        let span = range.max - range.min;
        let scale = span / (int_max as f64 - int_min_plus_one as f64);
        let midpoint = (range.min + range.max) / 2.0;
        Ok(i as f64 * scale + midpoint)
    } else {
        let i = read_unsigned_be(bytes);
        let int_max = unsigned_max(range.byte_length);
        let span = range.max - range.min;
        let scale = span / int_max as f64;
        Ok(i as f64 * scale + range.min)
    }
}

fn signed_max(n: usize) -> i64 {
    match n {
        1 => i8::MAX as i64,
        2 => i16::MAX as i64,
        4 => i32::MAX as i64,
        _ => unreachable!("byte_length validated by tags.rs"),
    }
}

fn unsigned_max(n: usize) -> i64 {
    match n {
        1 => u8::MAX as i64,
        2 => u16::MAX as i64,
        4 => u32::MAX as i64,
        _ => unreachable!("byte_length validated by tags.rs"),
    }
}

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

fn write_unsigned_be(value: u64, out: &mut [u8]) {
    let n = out.len();
    for (i, slot) in out.iter_mut().enumerate().take(n) {
        *slot = ((value >> (8 * (n - 1 - i))) & 0xFF) as u8;
    }
}

fn read_signed_be(bytes: &[u8]) -> i64 {
    let n = bytes.len();
    let mut bits: u64 = 0;
    for &b in bytes {
        bits = (bits << 8) | b as u64;
    }
    let sign_bit = 1u64 << (n as u32 * 8 - 1);
    if bits & sign_bit != 0 {
        let extension = !((1u64 << (n as u32 * 8)) - 1);
        (bits | extension) as i64
    } else {
        bits as i64
    }
}

fn read_unsigned_be(bytes: &[u8]) -> u64 {
    let mut bits: u64 = 0;
    for &b in bytes {
        bits = (bits << 8) | b as u64;
    }
    bits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_round_trip_lat() {
        let r = LinearRange {
            signed: true,
            byte_length: 4,
            min: -90.0,
            max: 90.0,
        };
        for v in [-89.999, -45.0, 0.0, 45.0, 89.999] {
            let mut buf = [0u8; 4];
            encode_fixed_range(&r, 13, v, &mut buf).unwrap();
            let back = decode_fixed_range(&r, 13, &buf).unwrap();
            assert!((back - v).abs() < 1e-6, "v={v} back={back}");
        }
    }

    #[test]
    fn signed_invalid_sentinel_decodes_to_error() {
        let r = LinearRange {
            signed: true,
            byte_length: 2,
            min: -20.0,
            max: 20.0,
        };
        let buf = [0x80, 0x00]; // INT16_MIN
        let err = decode_fixed_range(&r, 6, &buf).unwrap_err();
        matches!(err, KlvFieldError::InvalidSentinel { tag: 6 });
    }

    #[test]
    fn unsigned_round_trip_heading() {
        let r = LinearRange {
            signed: false,
            byte_length: 2,
            min: 0.0,
            max: 360.0,
        };
        for v in [0.0, 90.0, 180.0, 270.0, 359.99] {
            let mut buf = [0u8; 2];
            encode_fixed_range(&r, 5, v, &mut buf).unwrap();
            let back = decode_fixed_range(&r, 5, &buf).unwrap();
            assert!((back - v).abs() < 0.01, "v={v} back={back}");
        }
    }

    #[test]
    fn unsigned_round_trip_alt() {
        let r = LinearRange {
            signed: false,
            byte_length: 2,
            min: -900.0,
            max: 19000.0,
        };
        for v in [-900.0, -500.0, 0.0, 1000.0, 18000.0, 19000.0] {
            let mut buf = [0u8; 2];
            encode_fixed_range(&r, 15, v, &mut buf).unwrap();
            let back = decode_fixed_range(&r, 15, &buf).unwrap();
            assert!((back - v).abs() < 1.0, "v={v} back={back}");
        }
    }

    #[test]
    fn out_of_range_rejected() {
        let r = LinearRange {
            signed: true,
            byte_length: 4,
            min: -90.0,
            max: 90.0,
        };
        let mut buf = [0u8; 4];
        let err = encode_fixed_range(&r, 13, 100.0, &mut buf).unwrap_err();
        matches!(err, KlvEncodeError::OutOfRange { .. });
    }

    #[test]
    fn corner_offset_round_trip() {
        let r = LinearRange {
            signed: true,
            byte_length: 2,
            min: -0.075,
            max: 0.075,
        };
        for v in [-0.075, -0.05, 0.0, 0.05, 0.075] {
            let mut buf = [0u8; 2];
            encode_fixed_range(&r, 26, v, &mut buf).unwrap();
            let back = decode_fixed_range(&r, 26, &buf).unwrap();
            assert!((back - v).abs() < 1e-5, "v={v} back={back}");
        }
    }
}
