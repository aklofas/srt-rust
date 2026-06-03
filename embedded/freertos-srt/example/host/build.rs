// Embed the committed video-roundtrip golden so the host verifier compares
// against the SAME bytes every other P7 proof uses. Read the committed source
// of truth directly (crates/tst-integration/.../output.ts, 564 bytes) rather
// than the generated golden.h — output.ts is git-tracked, so this builds on a
// clean checkout; golden.h is generated at CI time by scripts/check/c/firmware-qemu.sh
// and is absent until then. No new golden is introduced.
use std::{env, fs, path::Path};

fn main() {
    let here = env::var("CARGO_MANIFEST_DIR").unwrap();
    let golden_ts = Path::new(&here)
        .join("../../../../crates/tst-integration/tests/fixtures/scenarios/video-roundtrip/output.ts");
    println!("cargo:rerun-if-changed={}", golden_ts.display());
    let bytes = fs::read(&golden_ts).expect("read video-roundtrip output.ts");
    assert_eq!(bytes.len(), 564, "GOLDEN must be 564 bytes, got {}", bytes.len());

    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("golden.rs");
    let len = bytes.len();
    let listed = bytes.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ");
    fs::write(
        &out,
        format!("pub static GOLDEN: [u8; {len}] = [{listed}];\npub const GOLDEN_LEN: usize = {len};\n"),
    )
    .unwrap();
}
