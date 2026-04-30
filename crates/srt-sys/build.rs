use std::env;
use std::path::PathBuf;

/// Build the vendored mbedTLS to a private install prefix.
/// Returns the install prefix path.
///
/// Only called when the `mbedtls` cargo feature is enabled.
fn build_mbedtls() -> PathBuf {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let mbedtls_dir = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("vendor/mbedtls"))
        .expect("Cannot resolve vendor/mbedtls path from CARGO_MANIFEST_DIR");

    if !mbedtls_dir.join("CMakeLists.txt").exists() {
        panic!(
            "Vendored mbedTLS not found at {}. \
             Run `git submodule update --init --recursive` from the workspace root.",
            mbedtls_dir.display()
        );
    }

    let dst = cmake::Config::new(&mbedtls_dir)
        .define("ENABLE_PROGRAMS", "OFF")
        .define("ENABLE_TESTING", "OFF")
        .define("USE_SHARED_MBEDTLS_LIBRARY", "OFF")
        .define("USE_STATIC_MBEDTLS_LIBRARY", "ON")
        .define("MBEDTLS_FATAL_WARNINGS", "OFF")
        // Hide mbedTLS from -Wall sweeps; we don't author this code.
        .define("CMAKE_C_FLAGS", "-w")
        .build();

    dst
}

fn main() {
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SRT_NO_PKG_CONFIG");
    println!("cargo:rerun-if-env-changed=SRT_FORCE_VENDORED");

    let force_vendored = env::var_os("SRT_FORCE_VENDORED").is_some();
    let no_pkg_config = env::var_os("SRT_NO_PKG_CONFIG").is_some();
    let want_mbedtls = env::var_os("CARGO_FEATURE_MBEDTLS").is_some();

    let mbedtls_prefix: Option<PathBuf> = if want_mbedtls {
        Some(build_mbedtls())
    } else {
        None
    };

    let include_paths: Vec<PathBuf> = if force_vendored || no_pkg_config {
        build_vendored(mbedtls_prefix.as_ref())
    } else {
        match pkg_config::Config::new()
            .atleast_version("1.5.0")
            .probe("srt")
        {
            Ok(lib) => lib.include_paths,
            Err(_) => build_vendored(mbedtls_prefix.as_ref()),
        }
    };

    let mut builder = bindgen::Builder::default()
        .header("wrapper.h")
        .rust_edition(bindgen::RustEdition::Edition2024)
        .allowlist_function("srt_.*")
        .allowlist_type("SRT_.*|SRTSOCKET|UDPSOCKET|CBytePerfMon")
        .allowlist_var("SRT_.*")
        .blocklist_type("sockaddr.*")
        .ctypes_prefix("libc")
        .raw_line("use libc::{sockaddr, sockaddr_storage};")
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

/// Build the vendored libsrt source via cmake.
///
/// If `mbedtls_prefix` is `Some`, libsrt is configured with
/// `ENABLE_ENCRYPTION=ON` + `USE_ENCLIB=mbedtls` and uses the prefix
/// to locate mbedTLS via `find_package(MbedTLS)`.
///
/// Otherwise, libsrt is built with `ENABLE_ENCRYPTION=OFF` (v0 behavior).
fn build_vendored(mbedtls_prefix: Option<&PathBuf>) -> Vec<PathBuf> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
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

    let mut cfg = cmake::Config::new(&vendor_dir);
    cfg.define("ENABLE_APPS", "OFF")
        .define("ENABLE_SHARED", "OFF")
        .define("ENABLE_STATIC", "ON")
        .define("ENABLE_UNITTESTS", "OFF")
        // Heavy logging is ON by default in Debug builds and causes a C++
        // static-initialization-order fiasco: the CUDTException constructor
        // (called from a static CThreadError object) tries to log via aclog
        // before aclog and srt_logger_config are initialized.  Disabling it
        // avoids the SIGSEGV at process startup when libsrt is statically
        // linked.
        .define("ENABLE_HEAVY_LOGGING", "OFF");

    match mbedtls_prefix {
        Some(prefix) => {
            cfg.define("ENABLE_ENCRYPTION", "ON")
                .define("USE_ENCLIB", "mbedtls")
                .define("CMAKE_PREFIX_PATH", prefix);
        }
        None => {
            cfg.define("ENABLE_ENCRYPTION", "OFF");
        }
    }

    let dst = cfg.build();

    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-search=native={}/lib64", dst.display());
    println!("cargo:rustc-link-lib=static=srt");

    // Link the mbedTLS static libs (libsrt's pkg-config file references them
    // but our static link line doesn't go through pkg-config).
    if let Some(prefix) = mbedtls_prefix {
        println!("cargo:rustc-link-search=native={}/lib", prefix.display());
        println!("cargo:rustc-link-search=native={}/lib64", prefix.display());
        // Order matters: libsrt -> libmbedtls -> libmbedx509 -> libmbedcrypto.
        println!("cargo:rustc-link-lib=static=mbedtls");
        println!("cargo:rustc-link-lib=static=mbedx509");
        println!("cargo:rustc-link-lib=static=mbedcrypto");
    }

    // libsrt is C++ internally; link the C++ stdlib explicitly.
    if cfg!(target_os = "linux") {
        println!("cargo:rustc-link-lib=stdc++");
    } else if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c++");
    }

    vec![dst.join("include")]
}
