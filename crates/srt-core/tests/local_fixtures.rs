//! Loads `tests/fixtures/local/*.klv` if the directory exists. No-op
//! otherwise — sensitive captures stay off the public repo, this test
//! passes silently in CI.

use std::fs;
use std::path::Path;

use srt_core::klv::st0601::{decode, decode_unchecked};

#[test]
fn local_fixtures_decode() {
    let dir = Path::new("tests/fixtures/local");
    let Ok(entries) = fs::read_dir(dir) else {
        // Directory absent — silently pass.
        return;
    };
    let mut count = 0usize;
    let mut had_failure = false;
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
        // Try strict-checksum first; fall back to unchecked for known-broken-
        // checksum captures. Either path failing (with a non-checksum error)
        // is an actual failure.
        match decode(&bytes) {
            Ok(_) => {}
            Err(_) => match decode_unchecked(&bytes) {
                Ok(_) => eprintln!("{}: decoded with checksum skipped", path.display()),
                Err(e) => {
                    eprintln!("{}: decode_unchecked failed: {e}", path.display());
                    had_failure = true;
                }
            },
        }
    }
    if count == 0 {
        return; // dir present but no .klv files
    }
    assert!(!had_failure, "one or more local fixtures failed to decode");
    eprintln!("local_fixtures: {count} fixture(s) parsed");
}
