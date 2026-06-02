// Parse the committed C golden (crates/baremetal-qemu-c/firmware/golden.h) into
// a Rust byte slice so the host verifier compares against the SAME bytes every
// other P7 proof uses. No new golden is introduced.
use std::{env, fs, path::Path};

fn main() {
    let here = env::var("CARGO_MANIFEST_DIR").unwrap();
    let golden_h = Path::new(&here).join("../../../crates/baremetal-qemu-c/firmware/golden.h");
    println!("cargo:rerun-if-changed={}", golden_h.display());
    let src = fs::read_to_string(&golden_h).expect("read golden.h");

    // Extract everything between `GOLDEN[] = {` and the closing `};`.
    let body = src
        .split_once("GOLDEN[] = {").expect("GOLDEN[] start").1
        .split_once("};").expect("GOLDEN[] end").0;
    let bytes: Vec<u8> = body
        .split(',')
        .filter_map(|t| {
            let t = t.trim();
            t.strip_prefix("0x").map(|h| u8::from_str_radix(h, 16).expect("hex byte"))
        })
        .collect();
    assert_eq!(bytes.len(), 564, "GOLDEN must be 564 bytes, got {}", bytes.len());

    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("golden.rs");
    let listed = bytes.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ");
    fs::write(&out, format!("pub static GOLDEN: [u8; 564] = [{listed}];\n")).unwrap();
}
