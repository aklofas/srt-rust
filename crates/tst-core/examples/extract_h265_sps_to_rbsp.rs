//! Extract the first SPS NAL's payload from an Annex-B HEVC file. Used to
//! regenerate the H.265 SPS test fixtures from real-encoder output.
//!
//! Why a dedicated extractor: ffmpeg can't natively dump SPS-only NALs
//! to a file in a clean form. This walks the bytes, strips the start
//! code + 2-byte NAL header, and writes the remaining EBSP bytes (with
//! emulation-prevention bytes preserved). The H.265 BitReader in this
//! crate takes EBSP as input — it handles EP-byte skipping internally.

use std::env;
use std::fs;
use std::io::{self, Write};

fn main() -> io::Result<()> {
    let path = env::args()
        .nth(1)
        .expect("usage: extract_h265_sps_to_rbsp <file.h265>");
    let bytes = fs::read(path)?;

    // Annex-B start codes: 0x00 0x00 0x00 0x01 or 0x00 0x00 0x01.
    let mut nals: Vec<&[u8]> = Vec::new();
    let mut i = 0;
    let mut last_start: Option<usize> = None;
    while i < bytes.len() {
        let is_4byte = bytes[i..].starts_with(&[0x00, 0x00, 0x00, 0x01]);
        let is_3byte = !is_4byte && bytes[i..].starts_with(&[0x00, 0x00, 0x01]);
        if is_4byte || is_3byte {
            if let Some(s) = last_start {
                nals.push(&bytes[s..i]);
            }
            last_start = Some(i + if is_4byte { 4 } else { 3 });
            i += if is_4byte { 4 } else { 3 };
        } else {
            i += 1;
        }
    }
    if let Some(s) = last_start {
        nals.push(&bytes[s..]);
    }

    // Find first NAL with H.265 nal_unit_type == 33 (SPS).
    // H.265 NAL header: 2 bytes; type lives in byte 0 bits 6..1.
    let sps_ebsp = nals
        .iter()
        .find(|nal| !nal.is_empty() && ((nal[0] >> 1) & 0x3F) == 33)
        .expect("no SPS NAL found in input");

    // Strip 2-byte NAL header; write the rest as-is (EBSP with EP bytes
    // preserved). The H.265 BitReader handles EP-byte skipping internally.
    io::stdout().write_all(&sps_ebsp[2..])
}
