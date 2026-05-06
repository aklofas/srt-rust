//! Asserts every globally-exported symbol in libtstrans.so starts with
//! `tst_` or `TST_`. Catches Rust-mangled leaks (e.g., a forgotten
//! `#[unsafe(no_mangle)]`) before they reach a downstream consumer.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn cdylib_exports_only_prefixed_symbols() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(manifest_dir).join("../../target"));
    let cdylib = target_dir.join("debug/libtstrans.so");
    assert!(
        cdylib.exists(),
        "expected libtstrans.so at {}; build tst-c first",
        cdylib.display(),
    );

    let out = Command::new("nm")
        .args(["-D", "-g", "--defined-only", cdylib.to_str().unwrap()])
        .output()
        .expect("run nm");
    assert!(out.status.success(), "nm failed: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);

    let mut bad: Vec<String> = Vec::new();
    for line in stdout.lines() {
        let name = match line.split_whitespace().nth(2) {
            Some(n) => n,
            None => continue,
        };
        // Standard ELF housekeeping symbols.
        if matches!(name, "_init" | "_fini" | "__bss_start" | "_edata" | "_end") {
            continue;
        }
        // Rust runtime symbols emitted by the toolchain.
        if name.starts_with("rust_") || name.starts_with("__rust_") {
            continue;
        }
        // libsrt's own C API surfaces through our static link. Allowlisted as
        // a known impurity. A future version-script build option will restrict
        // exports to tst_*/TST_* only.
        if name.starts_with("srt_") {
            continue;
        }
        if !name.starts_with("tst_") && !name.starts_with("TST_") {
            bad.push(name.to_string());
        }
    }
    assert!(
        bad.is_empty(),
        "exported symbols without tst_/TST_ prefix: {bad:?}",
    );
}
