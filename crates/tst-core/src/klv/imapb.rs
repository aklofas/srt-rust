//! ST 1201.5 §7 IMAPB — bit-packed mapping between unsigned integers and a
//! defined floating-point range.
//!
//! Given parameters `(min, max, length)` with `min < max` and `length ∈ 1..=8`:
//! - The integer occupies `length` bytes, big-endian, **unsigned** (ST 1201.5
//!   §7.2.3 Table 1 reserves the top of the unsigned range for special-value
//!   indicators — the top two bits both set marks the special-value space).
//! - Per ST 1201.5 §8.9 Summary:
//!   - `bPow = ceil(log2(max − min))`
//!   - `dPow = 8L − 1`
//!   - `sF = 2^(dPow − bPow)`  *(forward scale)*
//!   - `Zoffset = sF·min − floor(sF·min)` when `min<0 and max>0`; else 0
//! - Encode (§7.2.1): `y = truncate(sF·(value − min) + Zoffset)`, L-byte unsigned BE.
//! - Decode: `value = (y − Zoffset)·sR + min`, where `sR = 1/sF`.
//!
//! ## Special values
//!
//! ST 1201.5 §7.1.3 reserves the top of the unsigned integer range for
//! out-of-band signaling values. §7.2.2 step 1 mandates the reverse-mapping
//! algorithm test `bit(msb) & bit(msb-1)` first; if both are set, the
//! integer encodes a special value rather than a normal-range float.
//! §7.2.3 Table 2 enumerates the patterns the decoder recognizes.
//!
//! `decode_imapb` returns a [`DecodedImapb`] enum that carries the special
//! value (or out-of-range diagnostic) inline alongside the `Value(f64)`
//! normal-case variant. Use [`DecodedImapb::value`] for legacy `Option<f64>`
//! ergonomics when callers don't care about which special value was signaled.
//!
//! ST 0601 fixed-range mappings (which use a different convention with
//! INT_MIN as INVALID sentinel) live in `klv::st0601::mapping`.

use crate::error::{KlvEncodeError, KlvFieldError};
#[cfg(not(feature = "std"))]
use crate::float_ext::FloatExt;

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
    ///
    /// Preconditions (`min < max` and `length ∈ [1, 8]`) are now enforced
    /// at every public call site — `encode_imapb` and `decode_imapb`
    /// pre-screen the params and surface `KlvEncodeError::InvalidImapbParams`
    /// / `KlvFieldError::InvalidImapbParams` before invoking `sf()`.
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
}

/// Result of decoding an IMAPB wire integer per ST 1201.5 §7.2.2 + §7.2.3.
///
/// The wire format reserves the top of the unsigned integer range for
/// out-of-band signaling (§7.1.3). The reverse algorithm in §7.2.2 step 1
/// tests `bit(msb) & bit(msb-1)`: when both are set, the integer is a
/// special value rather than a normal-range float.
///
/// Normal-range decodes also get a bounds check against the configured
/// `[min, max]` — wire integers in the inter-band reserved space (above
/// `floor(sF·(max-min))` but with top-2-bits != `0b11`, per ST 1201.5 §8.6
/// Eq.12) arithmetic-decode past `max` and surface as
/// [`DecodedImapb::OutOfRange`] carrying the raw arithmetic decode.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum DecodedImapb {
    /// Normal-range decode succeeded and the result falls within `[min, max]`.
    Value(f64),
    /// ST 1201.5 §7.2.3 special-value pattern `1100_1000` (byte 0 = `0xC8`).
    PositiveInfinity,
    /// ST 1201.5 §7.2.3 special-value pattern `1110_1000` (byte 0 = `0xE8`).
    NegativeInfinity,
    /// ST 1201.5 §7.2.3 special-value pattern `1101_0000` (byte 0 = `0xD0`).
    NaN,
    /// ST 1201.5 §7.2.3 special-value pattern `1110_0000` (byte 0 = `0xE0`):
    /// the producer signaled "value below the configured minimum."
    BelowMin,
    /// ST 1201.5 §7.2.3 special-value pattern `1110_0001` (byte 0 = `0xE1`):
    /// the producer signaled "value above the configured maximum."
    AboveMax,
    /// Top-two-bits `0b11` set but the remaining bits do not match a
    /// pattern this decoder recognizes (reserved / user-defined / future
    /// MISB-defined per ST 1201.5 §7.2.3 Table 2). `raw` is the L-byte
    /// integer the producer emitted; callers may map specific bit patterns
    /// themselves if they have out-of-band knowledge.
    ReservedSpecial { raw: u64 },
    /// Normal-range decode succeeded but the result falls outside
    /// `[min, max]`. This is the diagnostic for the inter-band reserved
    /// integer space (per ST 1201.5 §8.6 Eq.12) — bit patterns that aren't
    /// in the §7.2.3 special-value space but that arithmetic-decode past
    /// the declared range. `decoded` is the raw arithmetic result so
    /// callers can inspect how far out of band the producer was.
    OutOfRange { decoded: f64 },
}

impl DecodedImapb {
    /// Convenience accessor: returns `Some(f64)` only for the [`Value`]
    /// variant. Special values and out-of-range integers return `None`.
    ///
    /// Use this at call sites that pre-date the special-value-aware API
    /// and want the legacy "decode to f64" ergonomics. Sites that need to
    /// distinguish `+∞` / `NaN` / `BelowMin` etc. should pattern-match the
    /// enum directly.
    ///
    /// [`Value`]: DecodedImapb::Value
    #[must_use]
    pub fn value(self) -> Option<f64> {
        match self {
            DecodedImapb::Value(v) => Some(v),
            _ => None,
        }
    }
}

pub fn encode_imapb(p: &ImapbParams, value: f64, out: &mut [u8]) -> Result<(), KlvEncodeError> {
    // ST 1201.5 §6 allows any L; internal math uses u64 (max 8 bytes).
    if !(1..=8).contains(&p.length) {
        return Err(KlvEncodeError::UnsupportedImapbLength { length: p.length });
    }
    // ST 1201.5 §6 `min < max` precondition. The §8.9
    // `bPow = ceil(log2(max − min))` derivation is undefined when
    // `max <= min` (log2(0) = -∞, log2(neg) = NaN). NaN-tolerant form:
    // `partial_cmp` returns `None` for NaN inputs, which we treat as a
    // violation (clippy::neg_cmp_op_on_partial_ord forbids `!(min<max)`).
    if !matches!(p.min.partial_cmp(&p.max), Some(core::cmp::Ordering::Less)) {
        return Err(KlvEncodeError::InvalidImapbParams {
            min: p.min,
            max: p.max,
            length: p.length as u8,
        });
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

/// Decode an IMAPB wire integer per ST 1201.5 §7.2.2 + §7.2.3.
///
/// # Behavior
///
/// Per §7.2.2 step 1, this function first inspects `bit(msb) & bit(msb-1)`
/// of the L-byte unsigned big-endian integer (the top two bits of byte 0).
/// When both bits are set, the integer encodes a special value per §7.2.3
/// Table 2 and is returned as the matching [`DecodedImapb`] variant
/// ([`PositiveInfinity`], [`NegativeInfinity`], [`NaN`], [`BelowMin`],
/// [`AboveMax`], or [`ReservedSpecial`] for unrecognized patterns in the
/// top-two-bits-set space).
///
/// Otherwise the normal-range reverse map
/// `value = sR · (y − Zoffset) + min` is applied, and the result is
/// bounds-checked against `[min, max]`. Values that fall outside the
/// configured range (the inter-band reserved integer space per §8.6
/// Eq.12) are returned as [`OutOfRange`] carrying the raw arithmetic
/// decode.
///
/// # Errors
///
/// - [`KlvFieldError::UnsupportedImapbLength`] when `length` is outside
///   `1..=8` (substrate caps at L=8 because internal arithmetic uses `u64`).
/// - [`KlvFieldError::InvalidImapbParams`] when `min >= max` (ST 1201.5 §6
///   precondition; the §8.9 scale-factor derivation is undefined otherwise).
/// - [`KlvFieldError::InvalidLength`] when `bytes.len() != length`.
///
/// Note that special values and out-of-range bit patterns are **not**
/// errors — they are spec-defined wire signaling from a conformant peer.
/// Callers that want to treat them as failures can pattern-match the
/// returned [`DecodedImapb`] or chain [`DecodedImapb::value`] +
/// `.ok_or(...)`.
///
/// [`PositiveInfinity`]: DecodedImapb::PositiveInfinity
/// [`NegativeInfinity`]: DecodedImapb::NegativeInfinity
/// [`NaN`]: DecodedImapb::NaN
/// [`BelowMin`]: DecodedImapb::BelowMin
/// [`AboveMax`]: DecodedImapb::AboveMax
/// [`ReservedSpecial`]: DecodedImapb::ReservedSpecial
/// [`OutOfRange`]: DecodedImapb::OutOfRange
pub fn decode_imapb(p: &ImapbParams, bytes: &[u8]) -> Result<DecodedImapb, KlvFieldError> {
    if !(1..=8).contains(&p.length) {
        return Err(KlvFieldError::UnsupportedImapbLength { length: p.length });
    }
    // ST 1201.5 §6 `min < max` precondition — checked AFTER the L gate
    // so pure-length failures keep their narrow `UnsupportedImapbLength`
    // diagnostic, then BEFORE the `bytes.len()` and arithmetic steps so
    // a malformed `(min, max)` never reaches `sf()`. NaN-tolerant via
    // `partial_cmp` (clippy::neg_cmp_op_on_partial_ord forbids
    // `!(min<max)`).
    if !matches!(p.min.partial_cmp(&p.max), Some(core::cmp::Ordering::Less)) {
        return Err(KlvFieldError::InvalidImapbParams {
            min: p.min,
            max: p.max,
            length: p.length as u8,
        });
    }
    if bytes.len() != p.length {
        return Err(KlvFieldError::InvalidLength {
            tag: 0,
            expected: p.length,
            got: bytes.len(),
        });
    }
    // Read L bytes as unsigned BE.
    let mut y: u64 = 0;
    for &b in bytes {
        y = (y << 8) | b as u64;
    }

    // ST 1201.5 §7.2.2 step 1: special-value detection. The spec defines
    // `special_value = Bit(msb, y) & Bit(msb-1, y)` over the L-byte
    // integer — i.e., the top two bits of byte 0. When both are set, the
    // integer is decoded via the §7.2.3 Table 2 pattern map, not the
    // normal reverse arithmetic.
    //
    // The §7.2.3 patterns are defined on byte 0 alone (the remaining L-1
    // bytes are payload-zero / pattern-extension that this substrate
    // doesn't currently sub-classify beyond ReservedSpecial). We branch
    // on byte 0 only.
    let top_byte = bytes[0];
    if (top_byte & 0b1100_0000) == 0b1100_0000 {
        return Ok(match top_byte {
            0xC8 => DecodedImapb::PositiveInfinity,
            0xE8 => DecodedImapb::NegativeInfinity,
            0xD0 => DecodedImapb::NaN,
            0xE0 => DecodedImapb::BelowMin,
            0xE1 => DecodedImapb::AboveMax,
            _ => DecodedImapb::ReservedSpecial { raw: y },
        });
    }

    // Normal-range reverse map: x = sR * (y - Zoffset) + min.
    let s_r = 1.0 / p.sf();
    let value = s_r * (y as f64 - p.z_offset()) + p.min;

    // ST 1201.5 §8.6 Eq.12 / §7.2.3 Table 1 row 2: integer values past
    // `floor(sF * (b - a))` are reserved (inter-band) — they arithmetic-
    // decode past `max` but the bit pattern isn't in the §7.2.3 special-
    // value space. Surface as OutOfRange so strict callers can reject and
    // lenient callers can inspect the decoded value.
    //
    // The bounds check tolerance has TWO components:
    //
    //   (1) IMAPB quantization step `scale = 2^ceil(log2(span)) / 2^(8L-1)`
    //       — at small L (L=1, L=2) the grid spacing is coarse enough that
    //       round-trip error against the input value can exceed `span * EPS`.
    //       The Zoffset rounding term in `decode = sR*(y - Zoffset) + min`
    //       can push decoded by up to one quantization step outside `min`/
    //       `max` even when the encoded integer is exactly 0 or
    //       `floor(sF*span)`. Proptest at L=1 surfaced this.
    //   (2) f64 ULP propagated through `sR * (y as f64 - Zoffset) + min`.
    //
    // Mirrors the tolerance derivation in `tests/common::imapb_tol`.
    let span = p.max - p.min;
    let b_pow = span.log2().ceil();
    let d_pow = (8 * p.length as i32 - 1) as f64;
    let scale = 2f64.powf(b_pow - d_pow);
    let fp_eps = span.abs() * f64::EPSILON * 8.0;
    let epsilon = scale.max(fp_eps);
    if value < p.min - epsilon || value > p.max + epsilon {
        return Ok(DecodedImapb::OutOfRange { decoded: value });
    }

    Ok(DecodedImapb::Value(value))
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
            let back = decode_imapb(&p, &buf).unwrap().value().unwrap();
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
            let back = decode_imapb(&p, &buf).unwrap().value().unwrap();
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
        let back = decode_imapb(&p, &buf).unwrap().value().unwrap();
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
        let back = decode_imapb(&p, &buf).unwrap().value().unwrap();
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
        let back = decode_imapb(&p, &buf).unwrap().value().unwrap();
        assert!(
            (back - 123.456).abs() < 1e-9,
            "L=8 round-trip failed: {back}"
        );
    }

    // --- A7 tests: ST 1201.5 §7.2.2 step 1 special-value detection + bounds ---

    #[test]
    fn imapb_decode_positive_infinity_wire_pattern_returns_special_variant() {
        // ST 1201.5 §7.2.3 Table 2: byte 0 = 0xC8 → +∞.
        let p = ImapbParams {
            min: 0.0,
            max: 100.0,
            length: 3,
        };
        let decoded = decode_imapb(&p, &[0xC8, 0x00, 0x00]).unwrap();
        assert_eq!(decoded, DecodedImapb::PositiveInfinity);
        assert_eq!(decoded.value(), None);
    }

    #[test]
    fn imapb_decode_negative_infinity_wire_pattern_returns_special_variant() {
        // ST 1201.5 §7.2.3 Table 2: byte 0 = 0xE8 → -∞.
        let p = ImapbParams {
            min: -100.0,
            max: 100.0,
            length: 3,
        };
        let decoded = decode_imapb(&p, &[0xE8, 0x00, 0x00]).unwrap();
        assert_eq!(decoded, DecodedImapb::NegativeInfinity);
    }

    #[test]
    fn imapb_decode_nan_wire_pattern_returns_special_variant() {
        // ST 1201.5 §7.2.3 Table 2: byte 0 = 0xD0 → NaN.
        let p = ImapbParams {
            min: 0.0,
            max: 100.0,
            length: 3,
        };
        let decoded = decode_imapb(&p, &[0xD0, 0x00, 0x00]).unwrap();
        assert_eq!(decoded, DecodedImapb::NaN);
    }

    #[test]
    fn imapb_decode_below_min_wire_pattern_returns_special_variant() {
        // ST 1201.5 §7.2.3 Table 2: byte 0 = 0xE0 → IMAP_BELOW_MINIMUM.
        let p = ImapbParams {
            min: 0.0,
            max: 100.0,
            length: 3,
        };
        let decoded = decode_imapb(&p, &[0xE0, 0x00, 0x00]).unwrap();
        assert_eq!(decoded, DecodedImapb::BelowMin);
    }

    #[test]
    fn imapb_decode_above_max_wire_pattern_returns_special_variant() {
        // ST 1201.5 §7.2.3 Table 2: byte 0 = 0xE1 → IMAP_ABOVE_MAXIMUM.
        let p = ImapbParams {
            min: 0.0,
            max: 100.0,
            length: 3,
        };
        let decoded = decode_imapb(&p, &[0xE1, 0x00, 0x00]).unwrap();
        assert_eq!(decoded, DecodedImapb::AboveMax);
    }

    #[test]
    fn imapb_decode_unrecognized_special_pattern_returns_reserved_variant() {
        // Top 2 bits set (0b11xx_xxxx) but not one of the 5 named patterns
        // → ReservedSpecial carrying the raw integer. 0xCC = 0b1100_1100.
        let p = ImapbParams {
            min: 0.0,
            max: 100.0,
            length: 3,
        };
        let decoded = decode_imapb(&p, &[0xCC, 0x12, 0x34]).unwrap();
        assert!(
            matches!(decoded, DecodedImapb::ReservedSpecial { raw } if raw == 0x00CC_1234),
            "got {decoded:?}"
        );
    }

    #[test]
    fn imapb_decode_out_of_range_returns_typed_variant() {
        // IMAPB(0, 100, 3): top 2 bits NOT both set (0b10xx_xxxx is reserved
        // inter-band per §8.6 Eq.12), arithmetic decodes to ~128.0, past
        // max=100.0. Per H-02, surface as OutOfRange carrying the decode.
        let p = ImapbParams {
            min: 0.0,
            max: 100.0,
            length: 3,
        };
        let decoded = decode_imapb(&p, &[0x80, 0x00, 0x00]).unwrap();
        match decoded {
            DecodedImapb::OutOfRange { decoded } => {
                assert!(
                    (decoded - 128.0).abs() < 1e-6,
                    "expected ~128.0, got {decoded}"
                );
            }
            other => panic!("expected OutOfRange, got {other:?}"),
        }
        assert_eq!(decoded.value(), None);
    }

    #[test]
    fn imapb_decode_max_value_pattern_within_range() {
        // ST 1201.5 §7.2.3 Table 1 row 2: integer floor(sF·(b-a)) is the
        // legitimate max-value mapping even with MSB=1. For IMAPB(0,100,3)
        // that integer is 0x640000, which arithmetic-decodes to exactly
        // 100.0 — must NOT be flagged as OutOfRange.
        let p = ImapbParams {
            min: 0.0,
            max: 100.0,
            length: 3,
        };
        let decoded = decode_imapb(&p, &[0x64, 0x00, 0x00]).unwrap();
        match decoded {
            DecodedImapb::Value(v) => {
                assert!((v - 100.0).abs() < 1e-9, "expected 100.0, got {v}");
            }
            other => panic!("expected Value(100.0), got {other:?}"),
        }
    }

    #[test]
    fn imapb_decode_min_value_within_range() {
        // Integer 0 must decode to exactly `min` (the §7.1.2 Starting Point
        // B reference). For IMAPB(0, 100, 3), wire 0x000000 → 0.0.
        let p = ImapbParams {
            min: 0.0,
            max: 100.0,
            length: 3,
        };
        let decoded = decode_imapb(&p, &[0x00, 0x00, 0x00]).unwrap();
        match decoded {
            DecodedImapb::Value(v) => assert!(v.abs() < 1e-9, "expected 0.0, got {v}"),
            other => panic!("expected Value(0.0), got {other:?}"),
        }
    }

    // --- ST 1201.5 §6 IMAPB precondition guards (validate-1 act-now M-02) ---
    //
    // These tests pin the new `min < max` precondition on both the encode
    // and decode entry points, plus the evaluation-order contract between
    // the existing `UnsupportedImapbLength` check and the new
    // `InvalidImapbParams` check.

    #[test]
    fn encode_rejects_min_eq_max() {
        // ST 1201.5 §6 requires min < max. Degenerate ranges blow up §8.9's
        // `bPow = ceil(log2(max − min))` derivation (log2(0) = −∞).
        let p = ImapbParams {
            min: 5.0,
            max: 5.0,
            length: 2,
        };
        let mut buf = [0u8; 2];
        let err = encode_imapb(&p, 5.0, &mut buf).unwrap_err();
        assert!(
            matches!(
                err,
                KlvEncodeError::InvalidImapbParams {
                    min: 5.0,
                    max: 5.0,
                    length: 2,
                }
            ),
            "expected InvalidImapbParams, got {err:?}"
        );
    }

    #[test]
    fn encode_rejects_min_gt_max() {
        let p = ImapbParams {
            min: 10.0,
            max: -10.0,
            length: 4,
        };
        let mut buf = [0u8; 4];
        let err = encode_imapb(&p, 0.0, &mut buf).unwrap_err();
        assert!(
            matches!(
                err,
                KlvEncodeError::InvalidImapbParams {
                    min: 10.0,
                    max: -10.0,
                    length: 4,
                }
            ),
            "expected InvalidImapbParams, got {err:?}"
        );
    }

    #[test]
    fn encode_rejects_length_out_of_range() {
        // L=0 and L=9 both surface as the narrower UnsupportedImapbLength,
        // NOT InvalidImapbParams — the length check fires first.
        let p0 = ImapbParams {
            min: 0.0,
            max: 1.0,
            length: 0,
        };
        let mut buf = [0u8; 4];
        let err = encode_imapb(&p0, 0.0, &mut buf).unwrap_err();
        assert!(
            matches!(err, KlvEncodeError::UnsupportedImapbLength { length: 0 }),
            "L=0: expected UnsupportedImapbLength, got {err:?}"
        );

        let p9 = ImapbParams {
            min: 0.0,
            max: 1.0,
            length: 9,
        };
        let mut buf = [0u8; 16];
        let err = encode_imapb(&p9, 0.0, &mut buf).unwrap_err();
        assert!(
            matches!(err, KlvEncodeError::UnsupportedImapbLength { length: 9 }),
            "L=9: expected UnsupportedImapbLength, got {err:?}"
        );
    }

    #[test]
    fn decode_rejects_min_eq_max() {
        let p = ImapbParams {
            min: -1.0,
            max: -1.0,
            length: 2,
        };
        let err = decode_imapb(&p, &[0x00, 0x00]).unwrap_err();
        assert!(
            matches!(
                err,
                KlvFieldError::InvalidImapbParams {
                    min: -1.0,
                    max: -1.0,
                    length: 2,
                }
            ),
            "expected InvalidImapbParams, got {err:?}"
        );
    }

    #[test]
    fn decode_rejects_min_gt_max() {
        let p = ImapbParams {
            min: 100.0,
            max: 0.0,
            length: 3,
        };
        let err = decode_imapb(&p, &[0x00, 0x00, 0x00]).unwrap_err();
        assert!(
            matches!(
                err,
                KlvFieldError::InvalidImapbParams {
                    min: 100.0,
                    max: 0.0,
                    length: 3,
                }
            ),
            "expected InvalidImapbParams, got {err:?}"
        );
    }

    #[test]
    fn decode_length_zero_or_nine_still_caught_by_existing_check() {
        // Evaluation-order contract: when BOTH the L gate AND the min<max
        // gate would trigger, the existing UnsupportedImapbLength diagnostic
        // wins. This keeps pre-existing diagnostics narrow and stable.
        let p0 = ImapbParams {
            min: 5.0,
            max: 5.0, // would also trigger InvalidImapbParams
            length: 0,
        };
        let err = decode_imapb(&p0, &[]).unwrap_err();
        assert!(
            matches!(err, KlvFieldError::UnsupportedImapbLength { length: 0 }),
            "L=0+badrange: expected UnsupportedImapbLength to fire first, got {err:?}"
        );

        let p9 = ImapbParams {
            min: 10.0,
            max: -10.0, // would also trigger InvalidImapbParams
            length: 9,
        };
        let err = decode_imapb(&p9, &[0u8; 9]).unwrap_err();
        assert!(
            matches!(err, KlvFieldError::UnsupportedImapbLength { length: 9 }),
            "L=9+badrange: expected UnsupportedImapbLength to fire first, got {err:?}"
        );
    }
}
