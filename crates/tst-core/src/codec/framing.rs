//! Shared byte-level sync-scan primitive for audio frame iterators.
//!
//! Both the MPEG audio (Layer I/II/III) and AAC ADTS iterators need to
//! scan forward for the next plausible sync word after a parse error.
//! The scan logic is identical except for the second-byte mask used to
//! match the sync word: `0xE0` for MPEG audio (11-bit sync: top 11 bits
//! `0x7FF`) vs `0xF0` for AAC ADTS (12-bit sync: top 12 bits `0xFFF`).
//!
//! `scan_for_sync` factors out the shared structure so each iterator
//! supplies only its match predicate.

/// Scan `buf[start..]` for the first position where `matches(buf[i], buf[i+1])`
/// is true. Returns the absolute position of the first candidate, or `None`
/// if no candidate exists before `buf.len() - 1`.
///
/// All callers (MPEG audio, AAC ADTS) check only the first two bytes of the
/// potential sync sequence, validating the remaining header fields downstream
/// in `parse_header`. Resync may therefore land on a false positive that
/// re-fails parse; the iterator will simply re-resync from `cursor + 1`.
pub(crate) fn scan_for_sync(
    buf: &[u8],
    start: usize,
    matches: impl Fn(u8, u8) -> bool,
) -> Option<usize> {
    if buf.len() < 2 || start >= buf.len() - 1 {
        return None;
    }
    let mut i = start;
    while i < buf.len() - 1 {
        if matches(buf[i], buf[i + 1]) {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_empty_buf_returns_none() {
        assert_eq!(scan_for_sync(&[], 0, |b0, _| b0 == 0xFF), None);
    }

    #[test]
    fn scan_single_byte_returns_none() {
        assert_eq!(scan_for_sync(&[0xFF], 0, |b0, _| b0 == 0xFF), None);
    }

    #[test]
    fn scan_finds_first_match() {
        // First candidate at index 2.
        let buf = [0x00, 0x11, 0xFF, 0xE0, 0xAA];
        let r = scan_for_sync(&buf, 0, |b0, b1| b0 == 0xFF && (b1 & 0xE0) == 0xE0);
        assert_eq!(r, Some(2));
    }

    #[test]
    fn scan_start_past_candidate_returns_none() {
        let buf = [0xFF, 0xE0, 0x00];
        // Start at index 1 — no room for a two-byte candidate at index 1 (buf[2] exists
        // but buf.len()-1 = 2, so i < 2 fails immediately for i=1).
        assert_eq!(
            scan_for_sync(&buf, 2, |b0, b1| b0 == 0xFF && (b1 & 0xE0) == 0xE0),
            None
        );
    }

    #[test]
    fn scan_no_match_returns_none() {
        let buf = [0x00, 0x00, 0x00];
        assert_eq!(scan_for_sync(&buf, 0, |b0, _| b0 == 0xFF), None);
    }
}
