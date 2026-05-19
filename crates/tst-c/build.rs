//! Runs cbindgen to generate `target/<profile>/include/tstrans.h`.

use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");

    // The header is emitted into target/<profile>/include/tstrans.h, which is a
    // sibling-of-OUT_DIR layout suitable for downstream consumers to find on
    // their include path. OUT_DIR is .../target/<profile>/build/tst-c-<hash>/out;
    // walk three levels up to get .../target/<profile>.
    let profile_dir = PathBuf::from(&out_dir)
        .ancestors()
        .nth(3)
        .expect("OUT_DIR ancestor walk failed")
        .to_path_buf();
    let include_dir = profile_dir.join("include");
    std::fs::create_dir_all(&include_dir).expect("create include dir");
    let header_path = include_dir.join("tstrans.h");

    let config =
        cbindgen::Config::from_file(format!("{crate_dir}/cbindgen.toml")).expect("cbindgen.toml");

    cbindgen::Builder::new()
        .with_config(config)
        .with_crate(&crate_dir)
        .generate()
        .expect("cbindgen generate")
        .write_to_file(&header_path);

    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-changed=src");

    // ────────────────────────────────────────────────────────────────
    // Symbol hygiene (audit 09-c-abi.md Finding 3): restrict the
    // libtstrans dynamic export table to tst_*/TST_* symbols on
    // platforms that support per-symbol linker-script export gates.
    // Linux uses --exclude-libs=ALL (hides all static-library symbols
    // from the dynamic export table; libsrt/mbedTLS are statically
    // linked so their srt_*/SRT_*/mbedtls_* exports are dropped while
    // our own #[no_mangle] tst_* symbols remain).
    // macOS uses -exported_symbols_list (whitelist by symbol-name
    // pattern). Windows MSVC is deferred to plan #65's follow-up
    // (runtime tests blocked on Windows hardware — see
    // project_plan_65 memory entry).
    //
    // Note: Plan B originally specified -Wl,--version-script=... for
    // Linux, but that conflicts with rustc's auto-emitted anonymous
    // version-script for cdylib targets (GNU BFD ld rejects mixing
    // anonymous and named version tags). --exclude-libs=ALL achieves
    // the same outcome (0 srt_*/SRT_* in libtstrans.so's export table)
    // without touching the auto-emitted script.
    // ────────────────────────────────────────────────────────────────

    #[cfg(target_os = "linux")]
    {
        println!("cargo:rustc-link-arg=-Wl,--exclude-libs=ALL");
    }

    #[cfg(target_os = "macos")]
    {
        let crate_dir_path = PathBuf::from(&crate_dir);
        let exports_path = crate_dir_path.join("exports.txt");
        println!("cargo:rerun-if-changed=exports.txt");
        println!(
            "cargo:rustc-link-arg=-Wl,-exported_symbols_list,{}",
            exports_path.display()
        );
    }

    #[cfg(target_os = "windows")]
    {
        // Defer: Windows MSVC linker uses .def files (/DEF:foo.def) or
        // per-symbol /EXPORT: args; both are mechanically straightforward
        // but runtime testing is blocked on Windows hardware. When the
        // plan #65 deferral lifts, ship a tst-c.def and add:
        //   println!("cargo:rerun-if-changed=tst-c.def");
        //   println!("cargo:rustc-link-arg=/DEF:tst-c.def");
        // For now: compile+link still works (no export-restriction means
        // all symbols remain exported, matching the pre-Plan-B Linux/macOS
        // behavior — this is the current Windows compile+link-only
        // status per plan #65).
    }

    // pkg-config substitution.
    let version = env!("CARGO_PKG_VERSION");
    let template_path = PathBuf::from(&crate_dir).join("tstrans.pc.in");
    let template = std::fs::read_to_string(&template_path).expect("read tstrans.pc.in");
    let pc = template
        .replace("@VERSION@", version)
        .replace("@PREFIX@", "/usr/local"); // tarball install default; consumer can sed it
    let pc_path = profile_dir.join("tstrans.pc");
    std::fs::write(&pc_path, pc).expect("write tstrans.pc");
    println!("cargo:rerun-if-changed=tstrans.pc.in");
}
