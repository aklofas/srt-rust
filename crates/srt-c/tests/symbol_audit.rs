//! Asserts every globally-exported symbol in libsrtc.so starts with
//! `srtc_` or `SRTC_`. Catches Rust-mangled leaks (e.g., a forgotten
//! `#[unsafe(no_mangle)]`) before they reach a downstream consumer.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn cdylib_exports_only_prefixed_symbols() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(manifest_dir).join("../../target"));
    let cdylib = target_dir.join("debug/libsrtc.so");
    assert!(
        cdylib.exists(),
        "expected libsrtc.so at {}; build srt-c first",
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
        if matches!(
            name,
            "_init" | "_fini" | "__bss_start" | "_edata" | "_end"
        ) {
            continue;
        }
        // Rust runtime symbols emitted by the toolchain.
        if name.starts_with("rust_") || name.starts_with("__rust_") {
            continue;
        }
        // libsrt's own C API surfaces through our static link. Allowlisted for
        // v0; tracked in docs/plans/2026-05-01-srt-c-design.md as a known
        // impurity. A future version-script build option will restrict exports
        // to srtc_*/SRTC_* only (option (b) from the audit design note).
        if name.starts_with("srt_") {
            continue;
        }
        if !name.starts_with("srtc_") && !name.starts_with("SRTC_") {
            bad.push(name.to_string());
        }
    }
    assert!(
        bad.is_empty(),
        "exported symbols without srtc_/SRTC_ prefix: {bad:?}",
    );
}
