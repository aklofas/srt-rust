//! Generate hand-crafted AV1 OBU fixtures for unit tests.
//! Run via `cargo run -p tst-srt --example gen_av1_fixtures`.
//!
//! Fixtures land at `crates/tst-core/tests/fixtures/codec/av1/`. Same
//! hand-crafting rationale as gen_h266_fixtures.

use std::fs;
use std::path::{Path, PathBuf};

fn fixtures_dir() -> PathBuf {
    // Resolve workspace-relative path: walks up from crates/tst-srt to the
    // workspace root, then into the tst-core fixtures tree where codec
    // tests look for them.
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("CARGO_MANIFEST_DIR is crates/tst-srt; walk up two levels");
    workspace_root.join("crates/tst-core/tests/fixtures/codec/av1")
}

/// Minimal Sequence Header OBU body — Main, 320x240, 8-bit 4:2:0.
/// Same bytes as codec::av1::sequence_header::tests::minimal_sequence_header().
fn seq_header_main_320x240() -> Vec<u8> {
    vec![0, 0, 0, 4, 60, 255, 188, 0, 0, 0]
}

/// Minimal Frame Header (keyframe) — show_existing_frame=0, frame_type=0,
/// show_frame=1.
/// Same byte as codec::av1::frame_header::tests::keyframe_header_body().
fn frame_header_keyframe() -> Vec<u8> {
    vec![0x10]
}

fn main() {
    let dir = fixtures_dir();
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("av1_320x240_main_seq_header.bin"),
        seq_header_main_320x240(),
    )
    .unwrap();
    fs::write(
        dir.join("av1_320x240_main_frame_header_keyframe.bin"),
        frame_header_keyframe(),
    )
    .unwrap();
    println!("wrote AV1 fixtures to {}", dir.display());
}
