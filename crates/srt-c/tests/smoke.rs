//! Compiles tests/smoke.c against the cdylib produced by `cargo build`
//! and runs the resulting binary.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn smoke_c_compiles_links_runs() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(manifest_dir).join("../../target"));
    let cdylib_dir = target_dir.join("debug");
    let header = PathBuf::from(manifest_dir).join("include/srtc.h");
    let smoke_c = PathBuf::from(manifest_dir).join("tests/smoke.c");

    assert!(cdylib_dir.join("libsrtc.so").exists(), "build srt-c first");
    assert!(header.exists(), "header missing at {}", header.display());

    let bin_path = std::env::temp_dir().join("srtc_smoke");
    let status = Command::new("cc")
        .args([
            "-Wall",
            "-Wextra",
            "-Werror",
            "-I",
            header.parent().unwrap().to_str().unwrap(),
            "-L",
            cdylib_dir.to_str().unwrap(),
            smoke_c.to_str().unwrap(),
            "-lsrtc",
            "-o",
            bin_path.to_str().unwrap(),
        ])
        .status()
        .expect("invoke cc");
    assert!(status.success(), "cc failed");

    let mut cmd = Command::new(&bin_path);
    cmd.env("LD_LIBRARY_PATH", &cdylib_dir);
    let out = cmd.output().expect("run smoke binary");
    if !out.status.success() {
        eprintln!("stdout:\n{}", String::from_utf8_lossy(&out.stdout));
        eprintln!("stderr:\n{}", String::from_utf8_lossy(&out.stderr));
        panic!("smoke binary failed: {:?}", out.status);
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("smoke OK"),
        "stdout missing 'smoke OK': {stdout}"
    );
}
