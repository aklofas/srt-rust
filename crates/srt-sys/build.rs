use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SRT_NO_PKG_CONFIG");
    println!("cargo:rerun-if-env-changed=SRT_FORCE_VENDORED");

    let force_vendored = env::var_os("SRT_FORCE_VENDORED").is_some();
    let no_pkg_config = env::var_os("SRT_NO_PKG_CONFIG").is_some();

    let include_paths: Vec<PathBuf> = if force_vendored || no_pkg_config {
        build_vendored()
    } else {
        match pkg_config::Config::new().atleast_version("1.5.0").probe("srt") {
            Ok(lib) => lib.include_paths,
            Err(_) => build_vendored(),
        }
    };

    let mut builder = bindgen::Builder::default()
        .header("wrapper.h")
        .allowlist_function("srt_.*")
        .allowlist_type("SRT_.*|SRTSOCKET|UDPSOCKET|CBytePerfMon")
        .allowlist_var("SRT_.*")
        .blocklist_type("sockaddr.*")
        .ctypes_prefix("libc")
        .derive_debug(true)
        .derive_default(true)
        .derive_copy(true)
        .layout_tests(false)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

    for path in &include_paths {
        builder = builder.clang_arg(format!("-I{}", path.display()));
    }

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    builder
        .generate()
        .expect("Failed to generate bindings against libsrt headers")
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Failed to write bindings.rs to OUT_DIR");
}

/// Build the vendored libsrt source via cmake (no encryption in v0).
/// Returns the include paths bindgen should use.
fn build_vendored() -> Vec<PathBuf> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // CARGO_MANIFEST_DIR points at crates/srt-sys; vendor/srt is two levels up.
    let vendor_dir = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("vendor/srt"))
        .expect("Cannot resolve vendor/srt path from CARGO_MANIFEST_DIR");

    if !vendor_dir.join("CMakeLists.txt").exists() {
        panic!(
            "Vendored libsrt not found at {}. \
             Run `git submodule update --init --recursive` from the workspace root.",
            vendor_dir.display()
        );
    }

    let dst = cmake::Config::new(&vendor_dir)
        .define("ENABLE_APPS", "OFF")
        .define("ENABLE_SHARED", "OFF")
        .define("ENABLE_STATIC", "ON")
        .define("ENABLE_ENCRYPTION", "OFF")
        .define("ENABLE_UNITTESTS", "OFF")
        .build();

    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-search=native={}/lib64", dst.display());
    println!("cargo:rustc-link-lib=static=srt");

    // libsrt is C++ internally; link the C++ stdlib explicitly.
    if cfg!(target_os = "linux") {
        println!("cargo:rustc-link-lib=stdc++");
    } else if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c++");
    }

    vec![dst.join("include")]
}
