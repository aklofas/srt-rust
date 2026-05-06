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
