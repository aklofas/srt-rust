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

    cmake::Config::new(&mbedtls_dir)
        .define("ENABLE_PROGRAMS", "OFF")
        .define("ENABLE_TESTING", "OFF")
        .define("USE_SHARED_MBEDTLS_LIBRARY", "OFF")
        .define("USE_STATIC_MBEDTLS_LIBRARY", "ON")
        .define("MBEDTLS_FATAL_WARNINGS", "OFF")
        // Hide mbedTLS from -Wall sweeps; we don't author this code.
        .define("CMAKE_C_FLAGS", "-w")
        .build()
}

fn main() {
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=SRT_NO_PKG_CONFIG");
    println!("cargo:rerun-if-env-changed=SRT_FORCE_VENDORED");

    // Symbol hygiene for downstream cdylib consumers (validate-1 D6).
    //
    // Any crate that depends on srt-sys and builds as a cdylib (today
    // tst-c; tomorrow tst-jni) should hide libsrt's static-library
    // exports from its own dynamic export table. The standard Linux
    // recipe is `-Wl,--exclude-libs=ALL`, which drops every symbol
    // sourced from a static archive while leaving the cdylib's own
    // `#[no_mangle]` exports untouched.
    //
    // `cargo:rustc-link-arg-cdylib` only flows into cdylib builds, so
    // staticlib consumers (and downstream Rust rlibs) are unaffected.
    // macOS / Windows linkers don't accept `--exclude-libs`; their
    // equivalent hygiene wiring lives in each cdylib's own build.rs
    // (see crates/tst-c/build.rs for the macOS exported_symbols_list
    // path and the Windows .def deferral).
    //
    // Cargo emits a warning here because srt-sys itself isn't a
    // cdylib (rust-lang/cargo#9562); the directive still flows to
    // downstream cdylib crates today. tst-c also emits the same
    // arg from its own build.rs as a defensive duplicate — passing
    // `--exclude-libs=ALL` twice is idempotent.
    if cfg!(target_os = "linux") {
        println!("cargo:rustc-link-arg-cdylib=-Wl,--exclude-libs=ALL");
    }

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
        .ctypes_prefix("libc")
        .derive_debug(true)
        .derive_default(true)
        .derive_copy(true)
        .layout_tests(false)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));

    // The libc crate exposes `sockaddr` / `sockaddr_storage` on Unix
    // targets, so we blocklist bindgen's auto-generated copies and
    // substitute libc's — this lets consumer crates reuse the same
    // sockaddr types as Rust's std::net layer.
    //
    // On Windows (`*-pc-windows-msvc`), libc does NOT export these
    // symbols at the crate root (Win32 uses its own `SOCKADDR` types
    // from <ws2def.h>), so the substitution would generate an
    // unresolved-import compile error. Let bindgen emit its own
    // copies on Windows — consumers cast through raw pointers in
    // either case, so the type identity doesn't matter end-to-end.
    let target_family = env::var("CARGO_CFG_TARGET_FAMILY").unwrap_or_default();
    if target_family == "unix" {
        builder = builder
            .blocklist_type("sockaddr.*")
            .raw_line("use libc::{sockaddr, sockaddr_storage};");
    }

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
/// Otherwise, libsrt is built with `ENABLE_ENCRYPTION=OFF`.
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

    // MSVC requires explicit /EHsc to enable C++ exception unwind
    // semantics; gcc/clang have it on by default. libsrt's sources use
    // try/catch, so without this every catch site errors C4530 and
    // libsrt fails to build under windows-msvc.
    let target = env::var("TARGET").unwrap_or_default();
    if target.contains("msvc") {
        cfg.cxxflag("/EHsc");
    }

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

    // libsrt's CMakeLists names the static library `srt_static.lib` on
    // MSVC to avoid colliding with the shared-library import lib (also
    // named `srt.lib`); on every other platform it's `libsrt.a` /
    // `libsrt.so` (rustc strips the lib prefix when linking by name).
    // See vendor/srt/CMakeLists.txt L1169-1181.
    let srt_lib_name = if target.contains("msvc") {
        "srt_static"
    } else {
        "srt"
    };
    println!("cargo:rustc-link-lib=static={srt_lib_name}");

    // Link the mbedTLS static libs (libsrt's pkg-config file references them
    // but our static link line doesn't go through pkg-config).
    if let Some(prefix) = mbedtls_prefix {
        println!("cargo:rustc-link-search=native={}/lib", prefix.display());
        println!("cargo:rustc-link-search=native={}/lib64", prefix.display());
        // Order matters: libsrt -> libmbedtls -> libmbedx509 -> libmbedcrypto.
        println!("cargo:rustc-link-lib=static=mbedtls");
        println!("cargo:rustc-link-lib=static=mbedx509");
        println!("cargo:rustc-link-lib=static=mbedcrypto");

        // mbedTLS on Windows uses BCryptGenRandom from bcrypt.dll for
        // entropy collection (mbedtls_platform_entropy_poll calls into
        // it). On Linux it uses /dev/urandom and no extra link is
        // needed; on Windows we need bcrypt.lib (system import lib
        // shipped with the Windows SDK).
        if target.contains("windows") {
            println!("cargo:rustc-link-lib=dylib=bcrypt");
        }
    }

    // libsrt is C++ internally; link the C++ stdlib explicitly.
    if cfg!(target_os = "linux") {
        println!("cargo:rustc-link-lib=stdc++");
    } else if cfg!(target_os = "macos") {
        println!("cargo:rustc-link-lib=c++");
    }

    vec![dst.join("include")]
}
