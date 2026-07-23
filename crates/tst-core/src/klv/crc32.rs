//! CRC-32/MPEG-2 (ISO/IEC 13818-1) — the ST 0806 RVT Local Set checksum
//! (ST 0806.4 §8 Tag 1 + §9 Appendix). Distinct from the ST 0601 16-bit
//! running-sum in `klv::checksum`. Table-driven, MSB-first, no reflection,
//! no final XOR — the §9 reference implementation returns the accumulator
//! directly.

#[allow(dead_code)]
const fn make_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = (i as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

#[allow(dead_code)]
static TABLE: [u32; 256] = make_table();

#[allow(dead_code)]
pub(crate) fn crc32_mpeg2(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0xFFFF_FFFF_u32, |acc, &b| {
        (acc << 8) ^ TABLE[(((acc >> 24) ^ b as u32) & 0xFF) as usize]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_mpeg2_known_answer() {
        // Canonical CRC-32/MPEG-2 check value: ASCII "123456789" -> 0x0376E6E7
        // (poly 0x04C11DB7, init 0xFFFFFFFF, refin=false, refout=false, xorout=0).
        assert_eq!(crc32_mpeg2(b"123456789"), 0x0376_E6E7);
    }

    #[test]
    fn crc32_mpeg2_empty_is_init() {
        // Zero bytes processed: the accumulator is returned untouched.
        assert_eq!(crc32_mpeg2(&[]), 0xFFFF_FFFF);
    }
}
