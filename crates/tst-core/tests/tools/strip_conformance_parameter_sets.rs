//! Strip parameter-set NAL/OBU bytes from official codec conformance
//! bitstreams. Driven by `manifest.toml`. See `plan
//! 2026-05-15-codec-conformance-bitstreams.md` for design.

// Stub: scanner + downloader land in later tasks; types/functions defined now
// for the manifest loader unit tests and to lock the TOML schema.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(default)]
    h264: Vec<Entry>,
    #[serde(default)]
    h265: Vec<Entry>,
    #[serde(default)]
    h266: Vec<Entry>,
    #[serde(default)]
    av1: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    name: String,
    archive_url: String,
    /// Path within the archive. Empty string => archive_url IS the raw bitstream.
    #[serde(default)]
    extract: String,
    sha256: String,
    kind: String,
    #[serde(default)]
    nal_index: u32,
    #[serde(default)]
    obu_index: u32,
    expected: Expected,
}

#[derive(Debug, Deserialize, Serialize)]
struct Expected {
    outcome: String, // "ok" or "err"
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fields: Option<BTreeMap<String, toml::Value>>,
}

fn load_manifest(path: &std::path::Path) -> Result<Manifest, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    toml::from_str(&s).map_err(|e| format!("parse {}: {}", path.display(), e))
}

/// Iterate Annex B NAL units. Yields each NAL unit as a byte slice
/// starting at the NAL header byte (start codes stripped, header
/// included). Trailing bytes after the last NAL until EOF form the
/// last yielded slice.
fn iter_annex_b(stream: &[u8]) -> impl Iterator<Item = &[u8]> {
    AnnexBIter { stream, cursor: 0 }
}

struct AnnexBIter<'a> {
    stream: &'a [u8],
    cursor: usize,
}

impl<'a> Iterator for AnnexBIter<'a> {
    type Item = &'a [u8];
    fn next(&mut self) -> Option<Self::Item> {
        // Find next start code from cursor.
        let nal_start = find_start_code(&self.stream[self.cursor..])?;
        let nal_data_start = self.cursor + nal_start.0 + nal_start.1; // skip past start code
        // Find next start code after this one (or EOF).
        let next = find_start_code(&self.stream[nal_data_start..])
            .map(|(off, _len)| nal_data_start + off)
            .unwrap_or(self.stream.len());
        self.cursor = next;
        Some(&self.stream[nal_data_start..next])
    }
}

/// Returns (offset_to_start_code, start_code_length) in haystack, or None.
fn find_start_code(haystack: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i + 3 <= haystack.len() {
        // 4-byte start code 00 00 00 01.
        // MUST check 4-byte before 3-byte: a `00 00 00 01` sequence would
        // otherwise alias as a 3-byte code starting at offset 1.
        if i + 4 <= haystack.len() && haystack[i..i + 4] == [0, 0, 0, 1] {
            return Some((i, 4));
        }
        // 3-byte start code 00 00 01
        if haystack[i..i + 3] == [0, 0, 1] {
            return Some((i, 3));
        }
        i += 1;
    }
    None
}

/// Extract the RBSP body of the `n`th NAL of the given `nal_unit_type`
/// from an Annex B H.264 bytestream. Returns None if not found.
fn extract_h264_parameter_set(stream: &[u8], nal_type: u8, n: u32) -> Option<Vec<u8>> {
    let mut matched = 0u32;
    for nal in iter_annex_b(stream) {
        if nal.is_empty() {
            continue;
        }
        // H.264 NAL header is 1 byte. Bits 1-5 = nal_unit_type.
        let header = nal[0];
        let this_type = header & 0x1F;
        if this_type == nal_type {
            if matched == n {
                return Some(nal[1..].to_vec());
            }
            matched += 1;
        }
    }
    None
}

/// Extract the RBSP body of the `n`th NAL of the given `nal_unit_type`
/// from an Annex B H.265 bytestream. H.265 NAL header is 2 bytes; the
/// first byte's bits 1-6 carry nal_unit_type.
fn extract_h265_parameter_set(stream: &[u8], nal_type: u8, n: u32) -> Option<Vec<u8>> {
    let mut matched = 0u32;
    for nal in iter_annex_b(stream) {
        if nal.len() < 2 {
            continue;
        }
        // H.265: first byte bit 0 = forbidden_zero_bit, bits 1-6 = nal_unit_type.
        let this_type = (nal[0] >> 1) & 0x3F;
        if this_type == nal_type {
            if matched == n {
                return Some(nal[2..].to_vec());
            }
            matched += 1;
        }
    }
    None
}

/// Extract the RBSP body of the `n`th NAL of the given `nal_unit_type`
/// from an Annex B H.266 / VVC bytestream. H.266 NAL header is 2 bytes;
/// nal_unit_type lives in byte 1, bits 3-7 (i.e. `(byte_1 >> 3) & 0x1F`)
/// per H.266 V4 §7.3.1.2.
fn extract_h266_parameter_set(stream: &[u8], nal_type: u8, n: u32) -> Option<Vec<u8>> {
    let mut matched = 0u32;
    for nal in iter_annex_b(stream) {
        if nal.len() < 2 {
            continue;
        }
        let this_type = (nal[1] >> 3) & 0x1F;
        if this_type == nal_type {
            if matched == n {
                return Some(nal[2..].to_vec());
            }
            matched += 1;
        }
    }
    None
}

fn main() {
    eprintln!("not yet implemented");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn manifest_loads_starter_set() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/conformance/manifest.toml");
        let m = load_manifest(&path).expect("starter manifest must parse");
        assert!(m.h264.len() >= 3, "h264 entries: {}", m.h264.len());
        assert!(m.h265.len() >= 3, "h265 entries: {}", m.h265.len());
        assert!(m.h266.len() >= 3, "h266 entries: {}", m.h266.len());
        assert!(m.av1.len() >= 2, "av1 entries: {}", m.av1.len());
    }

    #[test]
    fn manifest_entry_round_trips_required_fields() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/conformance/manifest.toml");
        let m = load_manifest(&path).expect("parse");
        for e in &m.h264 {
            assert!(!e.name.is_empty());
            assert!(!e.archive_url.is_empty());
            assert_eq!(e.sha256.len(), 64, "sha256 hex must be 64 chars");
            assert_eq!(e.kind, "h264_sps");
        }
    }

    /// Hand-crafted minimal Annex B stream: AUD + SPS + PPS. The SPS body
    /// here is 4 bytes of arbitrary placeholder — the scanner must extract
    /// those 4 bytes back out (NAL header stripped, NOT the start code).
    const FAKE_H264_STREAM: &[u8] = &[
        // 4-byte start code + AUD NAL (type 9)
        0x00, 0x00, 0x00, 0x01, 0x09, 0x10,
        // 3-byte start code + SPS NAL (type 7) + 4 bytes RBSP placeholder
        0x00, 0x00, 0x01, 0x67, 0xAA, 0xBB, 0xCC, 0xDD,
        // 3-byte start code + PPS NAL (type 8) + 2 bytes RBSP
        0x00, 0x00, 0x01, 0x68, 0xEE, 0xFF,
    ];

    #[test]
    fn scan_h264_sps_extracts_rbsp_body() {
        let extracted = extract_h264_parameter_set(FAKE_H264_STREAM, 7, 0)
            .expect("SPS should be found");
        assert_eq!(extracted, vec![0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn scan_h264_sps_respects_nal_index() {
        // Only one SPS in fixture; index 1 should miss.
        assert!(extract_h264_parameter_set(FAKE_H264_STREAM, 7, 1).is_none());
    }

    #[test]
    fn scan_h264_pps_extracts_rbsp_body() {
        let extracted = extract_h264_parameter_set(FAKE_H264_STREAM, 8, 0)
            .expect("PPS should be found");
        assert_eq!(extracted, vec![0xEE, 0xFF]);
    }

    const FAKE_H265_STREAM: &[u8] = &[
        // 4-byte start code + VPS NAL (type 32) + 3 bytes RBSP
        // 2-byte NAL header: 0100 0000 1<temporal_id+1>
        // type=32 -> first byte (0 << 7) | (32 << 1) | 0 = 0x40
        // second byte: layer_id=0, temporal_id_plus_1=1 -> 0x01
        0x00, 0x00, 0x00, 0x01, 0x40, 0x01, 0x11, 0x22, 0x33,
        // 3-byte start + SPS (type 33) + 4 bytes RBSP
        // first byte: (33 << 1) = 0x42
        0x00, 0x00, 0x01, 0x42, 0x01, 0x44, 0x55, 0x66, 0x77,
        // 3-byte start + PPS (type 34) + 2 bytes RBSP
        // first byte: (34 << 1) = 0x44
        0x00, 0x00, 0x01, 0x44, 0x01, 0x88, 0x99,
    ];

    #[test]
    fn scan_h265_sps_extracts_rbsp_body() {
        let extracted = extract_h265_parameter_set(FAKE_H265_STREAM, 33, 0)
            .expect("SPS should be found");
        assert_eq!(extracted, vec![0x44, 0x55, 0x66, 0x77]);
    }

    #[test]
    fn scan_h265_vps_extracts_rbsp_body() {
        let extracted = extract_h265_parameter_set(FAKE_H265_STREAM, 32, 0)
            .expect("VPS should be found");
        assert_eq!(extracted, vec![0x11, 0x22, 0x33]);
    }

    #[test]
    fn scan_h265_pps_extracts_rbsp_body() {
        let extracted = extract_h265_parameter_set(FAKE_H265_STREAM, 34, 0)
            .expect("PPS should be found");
        assert_eq!(extracted, vec![0x88, 0x99]);
    }

    const FAKE_H266_STREAM: &[u8] = &[
        // 4-byte start + H.266 VPS (type 14, in byte 1 high bits)
        // byte 0: 0 (forbidden) | 0 (reserved) | layer_id_hi<<2 = 0x00
        // byte 1: type(14)<<3 | temporal_id_plus_1(1) = (14<<3)|1 = 0x71
        0x00, 0x00, 0x00, 0x01, 0x00, 0x71, 0xAA, 0xBB,
        // SPS (type 15)
        // byte 1: (15<<3)|1 = 0x79
        0x00, 0x00, 0x01, 0x00, 0x79, 0xCC, 0xDD, 0xEE, 0xFF,
        // PPS (type 16)
        // byte 1: (16<<3)|1 = 0x81
        0x00, 0x00, 0x01, 0x00, 0x81, 0x11, 0x22,
    ];

    #[test]
    fn scan_h266_sps_extracts_rbsp_body() {
        let extracted = extract_h266_parameter_set(FAKE_H266_STREAM, 15, 0)
            .expect("SPS should be found");
        assert_eq!(extracted, vec![0xCC, 0xDD, 0xEE, 0xFF]);
    }

    #[test]
    fn scan_h266_vps_extracts_rbsp_body() {
        let extracted = extract_h266_parameter_set(FAKE_H266_STREAM, 14, 0)
            .expect("VPS should be found");
        assert_eq!(extracted, vec![0xAA, 0xBB]);
    }

    #[test]
    fn scan_h266_pps_extracts_rbsp_body() {
        let extracted = extract_h266_parameter_set(FAKE_H266_STREAM, 16, 0)
            .expect("PPS should be found");
        assert_eq!(extracted, vec![0x11, 0x22]);
    }
}
