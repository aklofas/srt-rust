//! Generate hand-crafted H.266 parameter-set fixtures for unit tests.
//! Run via `cargo run -p tst-core --bin gen-h266-fixtures`.
//!
//! Fixtures land at `crates/tst-core/tests/fixtures/codec/h266/`. Tests
//! load them and assert the parsed fields. Hand-crafting is preferred
//! over reference-encoder output for unit tests because (a) the bytes
//! are pinned, (b) we exercise the smallest valid path, (c) no external
//! tool dependency at test time.

use std::fs;
use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    // CARGO_MANIFEST_DIR is now crates/tst-core (this is a [[bin]] in tst-core).
    // The fixtures live in this same crate's tests/ tree.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/codec/h266")
}

/// Minimal VPS RBSP — vps_id=0, max_layers=1, max_sub_layers=1.
/// Same bytes as codec::h266::vps::tests::minimal_vps_rbsp().
fn vps_main10() -> Vec<u8> {
    vec![0x00, 0x02]
}

/// Minimal SPS RBSP — 320x240, Main 10, 8-bit 4:2:0.
/// Same bytes as codec::h266::sps::tests::minimal_sps_rbsp(). Covers
/// the full SPS body walk through sps_vui_parameters_present_flag with
/// all optional fields disabled. Re-capture via `dbg!(rbsp.clone())`
/// in that test if the body walk evolves.
fn sps_main10() -> Vec<u8> {
    vec![
        0x00, 0x09, 0x02, 0x3f, 0x00, 0x00, 0x00, 0x28, 0x20, 0x3c, 0x48, 0x00, 0x5d, 0xb0, 0xf8,
        0x06, 0x02, 0x08, 0x00, 0x02,
    ]
}

/// Minimal PPS RBSP — pps_id=0, sps_id=0.
/// Same bytes as codec::h266::pps::tests::minimal_pps_rbsp().
fn pps_main10() -> Vec<u8> {
    vec![0x00, 0x20]
}

fn main() {
    let dir = fixtures_dir();
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("h266_320x240_main10_vps.bin"), vps_main10()).unwrap();
    fs::write(dir.join("h266_320x240_main10_sps.bin"), sps_main10()).unwrap();
    fs::write(dir.join("h266_320x240_main10_pps.bin"), pps_main10()).unwrap();
    println!("wrote H.266 fixtures to {}", dir.display());
}
