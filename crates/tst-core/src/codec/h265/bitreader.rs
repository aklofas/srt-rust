//! Minimal RBSP bit reader for hand-rolled H.265 parameter set parsing.
//!
//! Operates on RBSP bytes with emulation prevention bytes (`00 00 03`)
//! preserved on input — `read_*` functions transparently skip the `03`
//! every time the previous two bytes were `00 00`. This matches the
//! input contract our public parsers expose (raw NAL payload from the
//! demuxer, no additional preprocessing).
//!
//! Reference: H.265 §7.2 (raw byte sequence payload), §9.2 (parsing process).

// Dead-code lint suppressed: this is a private substrate module; callers
// arrive in subsequent tasks (VPS/SPS/PPS parsers).
#![allow(dead_code)]

use crate::codec::ParseError;

pub struct BitReader<'a> {
    bytes: &'a [u8],
    /// Bit position within the input, counting bits skipped over EP bytes
    /// as if they were not there.
    bit_pos: u32,
    /// Total bits available after EP-stripping (lazy: capped at byte_pos*8
    /// at any point but never recomputed).
    bit_cap: u32,
}

impl<'a> BitReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            bit_pos: 0,
            bit_cap: (bytes.len() as u32).saturating_mul(8),
        }
    }

    pub fn position(&self) -> u32 {
        self.bit_pos
    }

    fn byte_at(&self, idx: usize) -> Option<u8> {
        self.bytes.get(idx).copied()
    }

    /// Read `n` bits (n ≤ 32). RBSP reading: if the previous two bytes
    /// were `00 00`, skip a single `03` byte before reading further.
    pub fn read_u(&mut self, n: u32) -> Result<u32, ParseError> {
        if n > 32 {
            return Err(ParseError::EngineError(format!("read_u({n}) > 32")));
        }
        let mut acc = 0u32;
        for _ in 0..n {
            acc = (acc << 1) | self.read_one_bit()? as u32;
        }
        Ok(acc)
    }

    pub fn read_bool(&mut self) -> Result<bool, ParseError> {
        Ok(self.read_one_bit()? != 0)
    }

    fn read_one_bit(&mut self) -> Result<u8, ParseError> {
        loop {
            let byte_idx = (self.bit_pos / 8) as usize;
            let bit_in_byte = self.bit_pos % 8;
            // EP-byte detection: at the start of a byte, if the prior two
            // bytes are 00 00 and the current byte is 03, skip it.
            if bit_in_byte == 0
                && byte_idx >= 2
                && self.bytes.get(byte_idx) == Some(&0x03)
                && self.bytes.get(byte_idx - 1) == Some(&0x00)
                && self.bytes.get(byte_idx - 2) == Some(&0x00)
            {
                self.bit_pos += 8;
                continue;
            }
            let b = self.byte_at(byte_idx).ok_or(ParseError::TruncatedRbsp {
                offset_bits: self.bit_pos,
                needed_bits: 1,
            })?;
            let bit = (b >> (7 - bit_in_byte)) & 1;
            self.bit_pos += 1;
            return Ok(bit);
        }
    }

    /// Unsigned Exp-Golomb (ue(v)) per H.265 §9.2.2.
    pub fn read_ue(&mut self) -> Result<u32, ParseError> {
        let start = self.bit_pos;
        let mut zeros = 0u32;
        loop {
            // Guard at >= 32: codeNum = 2^zeros − 1 + suffix, where the
            // maximum representable codeNum in u32 is 2^32 − 1 (requires
            // zeros = 31, suffix = 2^31 − 1). zeros = 32 makes `1u32 << 32`
            // undefined behavior (panics in debug, wraps in release).
            if zeros >= 32 {
                return Err(ParseError::InvalidGolomb { offset_bits: start });
            }
            let b = self.read_one_bit()?;
            if b == 1 {
                break;
            }
            zeros += 1;
        }
        let suffix = if zeros == 0 { 0 } else { self.read_u(zeros)? };
        Ok((1u32 << zeros).saturating_sub(1).saturating_add(suffix))
    }

    /// Signed Exp-Golomb (se(v)) per H.265 §9.2.3.
    pub fn read_se(&mut self) -> Result<i32, ParseError> {
        let v = self.read_ue()?;
        Ok(if v & 1 == 1 {
            ((v >> 1) as i32) + 1
        } else {
            -((v >> 1) as i32)
        })
    }

    pub fn skip(&mut self, n: u32) -> Result<(), ParseError> {
        for _ in 0..n {
            self.read_one_bit()?;
        }
        Ok(())
    }

    pub fn at_end(&self) -> bool {
        self.bit_pos >= self.bit_cap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_u_basic() {
        let mut br = BitReader::new(&[0b10101010, 0b11110000]);
        assert_eq!(br.read_u(4).unwrap(), 0b1010);
        assert_eq!(br.read_u(4).unwrap(), 0b1010);
        assert_eq!(br.read_u(8).unwrap(), 0b11110000);
    }

    #[test]
    fn read_ue_zero_is_one_bit() {
        let mut br = BitReader::new(&[0b10000000]);
        assert_eq!(br.read_ue().unwrap(), 0);
    }

    #[test]
    fn read_ue_one_is_three_bits() {
        // 010 → 1
        let mut br = BitReader::new(&[0b01000000]);
        assert_eq!(br.read_ue().unwrap(), 1);
    }

    #[test]
    fn read_ue_seven_is_seven_bits() {
        // 0001000 → 7
        let mut br = BitReader::new(&[0b00010000]);
        assert_eq!(br.read_ue().unwrap(), 7);
    }

    #[test]
    fn read_se_basic() {
        // ue(0)=0 → se=0; ue(1)=1 → se=1; ue(2)=2 → se=-1
        let mut br = BitReader::new(&[0b10000000]);
        assert_eq!(br.read_se().unwrap(), 0);
        let mut br = BitReader::new(&[0b01000000]);
        assert_eq!(br.read_se().unwrap(), 1);
        let mut br = BitReader::new(&[0b01100000]);
        assert_eq!(br.read_se().unwrap(), -1);
    }

    #[test]
    fn ep_byte_skipped() {
        // Bytes: 00 00 03 FF → after skipping the 03, the FF is read.
        let mut br = BitReader::new(&[0x00, 0x00, 0x03, 0xff]);
        assert_eq!(br.read_u(8).unwrap(), 0);
        assert_eq!(br.read_u(8).unwrap(), 0);
        assert_eq!(br.read_u(8).unwrap(), 0xff);
    }

    #[test]
    fn truncated_input_errors() {
        let mut br = BitReader::new(&[0xff]);
        assert!(br.read_u(16).is_err());
    }

    #[test]
    fn invalid_golomb_long_zeros_errors() {
        let mut br = BitReader::new(&[0; 8]);
        assert!(matches!(
            br.read_ue(),
            Err(ParseError::InvalidGolomb { .. })
        ));
    }

    #[test]
    fn read_ue_32_leading_zeros_does_not_overflow() {
        // Per H.265 §9.2.1 Eq 9-2: codeNum = 2^leadingZeros − 1 + suffix.
        // With zeros = 32, codeNum requires (1u32 << 32) which is UB in u32:
        // panics in debug, wraps to 1 in release. The maximum representable
        // codeNum in u32 is 2^32 − 1, requiring zeros = 31 with all-ones
        // suffix yielding 2^32 − 2. zeros = 32 is unrepresentable.
        //
        // Bit stream: 32 zero bits + 1 marker bit + 32-bit suffix.
        // Bytes 0-3 = 0x00 (32 zeros), byte 4 = 0x80 (marker '1' + 7 zeros),
        // bytes 5-8 = 0x00 (suffix bits).
        let bytes = [0x00, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00];
        let mut br = BitReader::new(&bytes);
        let result = br.read_ue();
        assert!(
            matches!(result, Err(ParseError::InvalidGolomb { .. })),
            "expected InvalidGolomb on 32-zero codeword, got {result:?}",
        );
    }
}
