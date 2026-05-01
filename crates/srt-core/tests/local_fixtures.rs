//! Loads `tests/fixtures/local/*.klv` if the directory exists. No-op
//! otherwise — sensitive captures stay off the public repo, this test
//! passes silently in CI.
//!
//! The shape variants this test slot is meant to exercise are documented
//! in `TEST_CORPUS.md` (alongside this file). Filename prefixes drive
//! per-shape assertions:
//!
//! - `multi-record-pes-*.klv` — PES payload with a wrapper UL preceding
//!   the ST 0601 LS. Single-shot `decode` is expected to FAIL; the
//!   record-iterating path must succeed.
//! - `decode-unchecked-only-*.klv` — record with broken checksum.
//!   `decode` is expected to fail with `ChecksumMismatch`;
//!   `decode_unchecked` must succeed.
//! - everything else — single ST 0601 record at offset 0; `decode`
//!   should succeed (with `decode_unchecked` as a relaxed fallback).

use std::fs;
use std::path::Path;

use srt_core::klv::length::read_ber;
use srt_core::klv::st0601::{decode, decode_unchecked};
use srt_core::klv::universal_label::UniversalLabel;

const LOCAL_FIXTURE_DIR: &str = "tests/fixtures/local";

#[test]
fn local_fixtures_decode() {
    let dir = Path::new(LOCAL_FIXTURE_DIR);
    let Ok(entries) = fs::read_dir(dir) else {
        return; // directory absent — silent pass
    };
    let mut count = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("klv") {
            continue;
        }
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("skip {}: {}", path.display(), e);
                continue;
            }
        };
        count += 1;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("(unknown)");

        let result: Result<(), String> = if stem.starts_with("multi-record-pes-") {
            assert_multi_record_pes(&bytes)
        } else if stem.starts_with("decode-unchecked-only-") {
            assert_unchecked_only(&bytes)
        } else {
            assert_single_record(&bytes)
        };

        if let Err(msg) = result {
            failures.push(format!("{}: {msg}", path.display()));
        }
    }

    if count == 0 {
        return;
    }
    assert!(
        failures.is_empty(),
        "{} local fixture(s) failed:\n  - {}",
        failures.len(),
        failures.join("\n  - ")
    );
    eprintln!("local_fixtures: {count} fixture(s) parsed");
}

/// Single ST 0601 record at offset 0. `decode` must succeed; if its
/// checksum is broken, `decode_unchecked` must succeed instead.
fn assert_single_record(bytes: &[u8]) -> Result<(), String> {
    if decode(bytes).is_ok() {
        return Ok(());
    }
    decode_unchecked(bytes)
        .map(|_| ())
        .map_err(|e| format!("decode_unchecked failed: {e}"))
}

/// Record with broken checksum: `decode` must fail with
/// `ChecksumMismatch`; `decode_unchecked` must then succeed.
fn assert_unchecked_only(bytes: &[u8]) -> Result<(), String> {
    use srt_core::error::KlvDecodeError;
    match decode(bytes) {
        Ok(_) => {
            Err("decode unexpectedly succeeded — fixture is not actually checksum-broken".into())
        }
        Err(KlvDecodeError::ChecksumMismatch { .. }) => decode_unchecked(bytes)
            .map(|_| ())
            .map_err(|e| format!("decode_unchecked failed: {e}")),
        Err(other) => Err(format!("decode failed with non-checksum error: {other}")),
    }
}

/// PES payload with a wrapper UL before the ST 0601 LS. Shape:
/// `[wrapper UL][BER len][wrapper body][ST 0601 UL][BER len][body]`.
/// `decode` on the whole buffer is expected to FAIL (it tries to
/// parse the wrapper as ST 0601). The record-iterator path, gating on
/// `UniversalLabel::is_st0601_family`, must find at least one
/// successfully decoded ST 0601 record.
fn assert_multi_record_pes(bytes: &[u8]) -> Result<(), String> {
    if decode(bytes).is_ok() {
        return Err(
            "decode unexpectedly succeeded — multi-record fixture should require record-iter"
                .into(),
        );
    }

    let mut i = 0usize;
    let mut decoded = 0usize;
    while i + 16 <= bytes.len() {
        let mut ul = [0u8; 16];
        ul.copy_from_slice(&bytes[i..i + 16]);
        let label = UniversalLabel::new(ul);
        let after_ul = &bytes[i + 16..];
        let (decl, after_len) = match read_ber(after_ul) {
            Ok(v) => v,
            Err(_) => break,
        };
        let len_bytes = after_ul.len() - after_len.len();
        let body_start = i + 16 + len_bytes;
        if body_start + decl > bytes.len() {
            break;
        }
        let total = 16 + len_bytes + decl;
        if label.is_st0601_family()
            && decode(&bytes[i..i + total])
                .or_else(|_| decode_unchecked(&bytes[i..i + total]))
                .is_ok()
        {
            decoded += 1;
        }
        i += total;
    }

    if decoded == 0 {
        Err("record-iter found no decodable ST 0601 records in PES payload".into())
    } else {
        Ok(())
    }
}
