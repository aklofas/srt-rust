// crates/srt-core/src/mpegts/demux/psi.rs
//! Program-specific information (PAT, PMT, descriptors).

use crate::mpegts::common::crc32::crc32_mpeg2;
use thiserror::Error;

/// Decoded Program Association Table (ISO/IEC 13818-1 §2.4.4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pat {
    pub transport_stream_id: u16,
    pub version: u8,
    pub current_next_indicator: bool,
    pub programs: Vec<PatEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PatEntry {
    pub program_number: u16,
    /// `0x0000` is the network PID; everything else is a PMT PID.
    pub pid: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PsiParseError {
    #[error("section too short ({have} bytes, need {need})")]
    Truncated { have: usize, need: usize },
    #[error("table_id mismatch (got 0x{got:02X}, expected 0x{expected:02X})")]
    TableIdMismatch { got: u8, expected: u8 },
    #[error("CRC32 mismatch (computed 0x{computed:08X}, declared 0x{declared:08X})")]
    CrcMismatch { computed: u32, declared: u32 },
    #[error("section_length {0} declares more than fits in 1024 bytes")]
    SectionTooLong(u16),
    #[error("malformed program loop entry at offset {offset}")]
    MalformedProgramEntry { offset: usize },
    #[error("descriptor loop length {declared} exceeds remaining {remaining}")]
    DescriptorLoopOverflow { declared: usize, remaining: usize },
}

/// Parse a fully-assembled PAT section (ISO/IEC 13818-1 §2.4.4.3).
///
/// Bytes must include the 4-byte CRC-32/MPEG-2 trailer; the function
/// verifies the CRC over the section header + payload.
pub fn parse_pat(section: &[u8]) -> Result<Pat, PsiParseError> {
    // Minimum: 8-byte fixed header + 4-byte CRC32 = 12 bytes; add 4 per program.
    if section.len() < 12 {
        return Err(PsiParseError::Truncated {
            have: section.len(),
            need: 12,
        });
    }
    if section[0] != 0x00 {
        return Err(PsiParseError::TableIdMismatch {
            got: section[0],
            expected: 0x00,
        });
    }
    let section_length = (((section[1] & 0x0F) as u16) << 8) | section[2] as u16;
    if section_length as usize > 1021 {
        return Err(PsiParseError::SectionTooLong(section_length));
    }
    let total_len = 3 + section_length as usize;
    if section.len() < total_len {
        return Err(PsiParseError::Truncated {
            have: section.len(),
            need: total_len,
        });
    }
    // CRC over bytes [0 .. total_len - 4] should equal the trailer.
    let computed = crc32_mpeg2(&section[..total_len - 4]);
    let declared = u32::from_be_bytes([
        section[total_len - 4],
        section[total_len - 3],
        section[total_len - 2],
        section[total_len - 1],
    ]);
    if computed != declared {
        return Err(PsiParseError::CrcMismatch { computed, declared });
    }
    let transport_stream_id = u16::from_be_bytes([section[3], section[4]]);
    let version = (section[5] >> 1) & 0x1F;
    let current_next_indicator = (section[5] & 0x01) != 0;
    // Program loop runs from byte 8 to byte total_len - 4 (exclusive — that's the CRC).
    let mut programs = Vec::new();
    let mut off = 8;
    while off + 4 <= total_len - 4 {
        let pn = u16::from_be_bytes([section[off], section[off + 1]]);
        let pid = u16::from_be_bytes([section[off + 2] & 0x1F, section[off + 3]]);
        programs.push(PatEntry {
            program_number: pn,
            pid,
        });
        off += 4;
    }
    if off != total_len - 4 {
        return Err(PsiParseError::MalformedProgramEntry { offset: off });
    }
    Ok(Pat {
        transport_stream_id,
        version,
        current_next_indicator,
        programs,
    })
}

#[cfg(test)]
mod pat_tests {
    use super::*;

    /// Build a minimal valid PAT section bytes for testing.
    /// `programs` is `(program_number, pid)` tuples.
    fn build_pat_section(
        transport_stream_id: u16,
        version: u8,
        programs: &[(u16, u16)],
    ) -> Vec<u8> {
        let mut s = Vec::new();
        s.push(0x00); // table_id = PAT
        // section_syntax_indicator (1) + reserved-zero (1) + reserved (2) + section_length (12)
        let section_length = 5 + 4 * programs.len() + 4; // header tail + program loop + CRC32
        let sl = section_length as u16;
        s.push(0xB0 | ((sl >> 8) as u8 & 0x0F)); // 1011_xxxx
        s.push((sl & 0xFF) as u8);
        s.push((transport_stream_id >> 8) as u8);
        s.push((transport_stream_id & 0xFF) as u8);
        s.push(0xC1 | ((version & 0x1F) << 1)); // reserved(2)=11 + version(5) + current_next(1)=1
        s.push(0x00); // section_number
        s.push(0x00); // last_section_number
        for (pn, pid) in programs {
            s.push((pn >> 8) as u8);
            s.push((pn & 0xFF) as u8);
            s.push(0xE0 | ((pid >> 8) as u8 & 0x1F));
            s.push((pid & 0xFF) as u8);
        }
        let crc = crc32_mpeg2(&s);
        s.push((crc >> 24) as u8);
        s.push((crc >> 16) as u8);
        s.push((crc >> 8) as u8);
        s.push(crc as u8);
        s
    }

    #[test]
    fn parses_minimal_pat() {
        let bytes = build_pat_section(0x1234, 7, &[(1, 0x100)]);
        let pat = parse_pat(&bytes).unwrap();
        assert_eq!(pat.transport_stream_id, 0x1234);
        assert_eq!(pat.version, 7);
        assert!(pat.current_next_indicator);
        assert_eq!(
            pat.programs,
            vec![PatEntry {
                program_number: 1,
                pid: 0x100
            }]
        );
    }

    #[test]
    fn parses_two_programs() {
        let bytes = build_pat_section(1, 0, &[(1, 0x100), (2, 0x200)]);
        let pat = parse_pat(&bytes).unwrap();
        assert_eq!(pat.programs.len(), 2);
        assert_eq!(pat.programs[1].pid, 0x200);
    }

    #[test]
    fn rejects_wrong_table_id() {
        let mut bytes = build_pat_section(1, 0, &[(1, 0x100)]);
        bytes[0] = 0x02; // PMT table_id
        let err = parse_pat(&bytes).unwrap_err();
        assert!(matches!(err, PsiParseError::TableIdMismatch { .. }));
    }

    #[test]
    fn rejects_bad_crc() {
        let mut bytes = build_pat_section(1, 0, &[(1, 0x100)]);
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        let err = parse_pat(&bytes).unwrap_err();
        assert!(matches!(err, PsiParseError::CrcMismatch { .. }));
    }

    #[test]
    fn rejects_truncated() {
        let bytes = build_pat_section(1, 0, &[(1, 0x100)]);
        let err = parse_pat(&bytes[..5]).unwrap_err();
        assert!(matches!(err, PsiParseError::Truncated { .. }));
    }
}
