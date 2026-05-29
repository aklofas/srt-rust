use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Build the workspace-shared vendored mbedTLS (`vendor/mbedtls`, 3.6.x) to a
/// private install prefix and return that prefix. Mirrors
/// `srt-sys::build_mbedtls` so librist links the SAME mbedTLS version libsrt
/// does (see the `mbedtls` feature note in Cargo.toml). Only called when the
/// `mbedtls` feature is on + the vendored build path is taken.
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

    let mut cfg = cmake::Config::new(&mbedtls_dir);
    cfg.define("ENABLE_PROGRAMS", "OFF")
        .define("ENABLE_TESTING", "OFF")
        .define("USE_SHARED_MBEDTLS_LIBRARY", "OFF")
        .define("USE_STATIC_MBEDTLS_LIBRARY", "ON")
        .define("MBEDTLS_FATAL_WARNINGS", "OFF")
        // Hide mbedTLS from -Wall sweeps; we don't author this code.
        .define("CMAKE_C_FLAGS", "-w");

    // On windows-msvc, Rust links the dynamic release UCRT (`/MD`) regardless
    // of the cargo profile, but cmake defaults to `/MDd` for a Debug build —
    // mixing the two trips LNK2038 (RuntimeLibrary mismatch). Pin mbedTLS to
    // the dynamic release CRT so it matches Rust and the cl-built librist
    // (which uses `-Db_vscrt=md`). No-op off MSVC.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        cfg.define("CMAKE_POLICY_DEFAULT_CMP0091", "NEW")
            .define("CMAKE_MSVC_RUNTIME_LIBRARY", "MultiThreadedDLL");
    }

    cfg.build()
}

fn main() {
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=RIST_NO_PKG_CONFIG");
    println!("cargo:rerun-if-env-changed=RIST_FORCE_VENDORED");

    // Same symbol-hygiene recipe as srt-sys: hide librist's static-library
    // exports from downstream cdylib export tables (tst-c, future tst-jni).
    if cfg!(target_os = "linux") {
        println!("cargo:rustc-link-arg-cdylib=-Wl,--exclude-libs=ALL");
    }

    let force_vendored = env::var_os("RIST_FORCE_VENDORED").is_some();
    let no_pkg_config = env::var_os("RIST_NO_PKG_CONFIG").is_some();
    let feat_mbedtls = env::var_os("CARGO_FEATURE_MBEDTLS").is_some();

    let include_paths: Vec<PathBuf> = if force_vendored || no_pkg_config {
        build_vendored(feat_mbedtls)
    } else {
        match pkg_config::Config::new()
            .atleast_version("0.2.10")
            .probe("librist")
        {
            Ok(lib) => lib.include_paths,
            Err(_) => build_vendored(feat_mbedtls),
        }
    };

    // ===== Generate bindings =====
    let mut builder = bindgen::Builder::default()
        .header("wrapper.h")
        .rust_edition(bindgen::RustEdition::Edition2024)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .ctypes_prefix("libc")
        .derive_debug(true)
        .derive_default(true)
        .derive_copy(true)
        .layout_tests(false)
        // Allowlist librist symbols + macros.
        .allowlist_function("rist_.*")
        .allowlist_function("librist_.*")
        .allowlist_type("rist_.*")
        .allowlist_var("RIST_.*")
        .allowlist_var("LIBRIST_.*");

    // librist's logging.h exposes `FILE *log_stream` inside
    // `rist_logging_settings`, so bindgen needs a `FILE` type in scope.
    // Substitute libc's FILE on Unix; let bindgen emit an opaque FILE on
    // Windows (libc doesn't export FILE under windows-msvc).
    let target_family = env::var("CARGO_CFG_TARGET_FAMILY").unwrap_or_default();
    if target_family == "unix" {
        builder = builder
            .blocklist_type("FILE")
            .blocklist_type("__sFILE")
            .blocklist_type("fpos_t")
            .raw_line("use libc::FILE;");
    }

    for inc in &include_paths {
        builder = builder.clang_arg(format!("-I{}", inc.display()));
    }

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    builder
        .generate()
        .expect("Failed to generate librist bindings")
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Failed to write bindings.rs to OUT_DIR");
}

/// Build librist from `vendor/librist` via meson + ninja.
///
/// Unlike srt-sys (which uses cmake), librist is a meson project. We invoke
/// `meson setup` + `meson compile` via `std::process::Command`. Both `meson`
/// and `ninja` must be on `$PATH` (Debian: `apt install meson ninja-build`).
fn build_vendored(want_mbedtls: bool) -> Vec<PathBuf> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let vendor_dir = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("vendor/librist"))
        .expect("Cannot resolve vendor/librist path from CARGO_MANIFEST_DIR");

    if !vendor_dir.join("meson.build").exists() {
        panic!(
            "Vendored librist not found at {}. \
             Run `git submodule update --init --recursive` from the workspace root.",
            vendor_dir.display()
        );
    }

    require_tool("meson");
    require_tool("ninja");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let build_dir = out_dir.join("librist-build");

    // ===== meson setup =====
    //
    // Args:
    //   --default-library=static  static lib only
    //   --buildtype=release       optimized build
    //   -Dbuilt_tools=false       skip CLI tools (faster build)
    //   -Dtest=false              skip test suite
    //   -Dbuiltin_cjson=true      bundle contrib/cjson (avoids system cjson dep)
    //   -Dbuiltin_mbedtls=<bool>  bundle contrib/mbedtls when mbedtls feature is on
    //   -Duse_mbedtls=<bool>      enable/disable encryption entirely
    let mut args: Vec<String> = vec![
        "setup".into(),
        build_dir.to_string_lossy().into_owned(),
        vendor_dir.to_string_lossy().into_owned(),
        "--default-library=static".into(),
        "--buildtype=release".into(),
        "-Dbuilt_tools=false".into(),
        "-Dtest=false".into(),
        "-Dbuiltin_cjson=true".into(),
    ];
    let mut meson_envs: Vec<(String, String)> = Vec::new();

    // On Windows the Rust target is `x86_64-pc-windows-msvc`, so librist (and
    // its bundled contrib/mbedtls + cJSON) MUST be compiled with MSVC `cl` —
    // not the mingw `gcc` that meson would otherwise pick up from Strawberry
    // Perl on PATH. A mingw-built `librist.a` references mingw/winpthreads
    // runtime symbols (`__mingw_printf`, `___chkstk_ms`, `pthread_once`, ...)
    // that the MSVC linker can't resolve (LNK1120).
    //
    // We locate `cl` and its environment (INCLUDE/LIB/PATH) via the same MSVC
    // registry discovery the `cmake` crate uses for srt-sys, and inject it into
    // ONLY the meson/ninja subprocesses (see `run_cmd`). This deliberately does
    // NOT pollute the whole process env (the earlier `ilammy/msvc-dev-cmd`
    // approach did, which made bindgen's libclang pick up MSVC's bundled
    // clang headers and fail parsing `__m64` in `mmintrin.h`). With `cl`, MSVC's
    // `<time.h>` has no `clock_gettime`, so librist's `contrib/time-shim.c`
    // shim compiles cleanly (no redefinition) and its Windows-native threading
    // path needs no winpthreads — hence we do NOT enable `have_mingw_pthreads`.
    // `-Db_vscrt=md` matches Rust's dynamic UCRT (Rust links `/MD` even in
    // debug) so the CRTs don't clash at link time.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let target = env::var("TARGET").unwrap_or_default();
        let cl = cc::windows_registry::find_tool(&target, "cl.exe").unwrap_or_else(|| {
            panic!(
                "Could not locate MSVC `cl.exe` for target `{target}`. \
                 librist's meson build needs the MSVC toolchain on windows-msvc; \
                 ensure Visual Studio Build Tools (VC++) are installed."
            )
        });
        // Tool::env() yields the INCLUDE/LIB/PATH (and friends) the MSVC
        // compiler needs; forward each into the meson subprocess env. For PATH
        // we APPEND the discovered MSVC dirs to the inherited PATH rather than
        // replacing it, so meson + ninja (installed on the original PATH) stay
        // findable by the `meson compile` step.
        for (k, v) in cl.env() {
            let key = k.to_string_lossy().into_owned();
            let mut val = v.to_string_lossy().into_owned();
            if key.eq_ignore_ascii_case("path") {
                if let Some(existing) = env::var_os(&key) {
                    val.push(';');
                    val.push_str(&existing.to_string_lossy());
                }
            }
            meson_envs.push((key, val));
        }
        // Point meson at the discovered cl.exe explicitly so it doesn't fall
        // back to the mingw gcc that is also on PATH.
        meson_envs.push(("CC".to_string(), cl.path().to_string_lossy().into_owned()));
        meson_envs.push(("CXX".to_string(), cl.path().to_string_lossy().into_owned()));
        args.push("-Db_vscrt=md".into());
    }
    // When encryption is wanted, build the SHARED workspace vendor/mbedtls
    // (3.6.x) and point librist's meson at it via PKG_CONFIG_PATH so it links
    // that `mbedcrypto` (`-Dbuiltin_mbedtls=false`) instead of its bundled
    // contrib/mbedtls 2.26.0 — see the `mbedtls` feature note in Cargo.toml.
    let mbedtls_prefix: Option<PathBuf> = if want_mbedtls {
        let prefix = build_mbedtls();
        let pc_dir = prefix.join("lib").join("pkgconfig");
        let pkg_path = match env::var("PKG_CONFIG_PATH") {
            Ok(existing) if !existing.is_empty() => {
                format!("{}:{}", pc_dir.display(), existing)
            }
            _ => pc_dir.to_string_lossy().into_owned(),
        };
        meson_envs.push(("PKG_CONFIG_PATH".to_string(), pkg_path));
        args.push("-Dbuiltin_mbedtls=false".into());
        args.push("-Duse_mbedtls=true".into());
        Some(prefix)
    } else {
        args.push("-Duse_mbedtls=false".into());
        None
    };

    // If a previous build exists, reconfigure it instead of erroring.
    if build_dir.join("build.ninja").exists() {
        args[0] = "setup".into();
        args.insert(1, "--reconfigure".into());
    }

    run_cmd("meson", &args, &vendor_dir, &meson_envs);

    // ===== meson compile =====
    run_cmd(
        "meson",
        &[
            "compile".to_string(),
            "-C".to_string(),
            build_dir.to_string_lossy().into_owned(),
        ],
        &vendor_dir,
        &meson_envs,
    );

    // librist's meson outputs `librist.a` directly in builddir (with
    // --default-library=static). Headers stay in source tree under
    // `vendor/librist/include/`.
    println!(
        "cargo:rustc-link-search=native={}",
        build_dir.to_string_lossy()
    );
    println!("cargo:rustc-link-lib=static=rist");

    // With `-Dbuiltin_mbedtls=false`, librist.a does NOT bundle mbedTLS — its
    // `mbedtls_*` references resolve against the shared vendor/mbedtls 3.6.x
    // static libs we built above. Link them after librist.a (link order:
    // mbedtls -> mbedx509 -> mbedcrypto, matching srt-sys). The downstream
    // cdylib's `-Wl,--allow-multiple-definition` collapses these with libsrt's
    // identical 3.6.x copy.
    if let Some(prefix) = &mbedtls_prefix {
        println!(
            "cargo:rustc-link-search=native={}",
            prefix.join("lib").to_string_lossy()
        );
        println!("cargo:rustc-link-lib=static=mbedtls");
        println!("cargo:rustc-link-lib=static=mbedx509");
        println!("cargo:rustc-link-lib=static=mbedcrypto");
    }

    // Platform link needs. librist is pure C; no C++ stdlib link required.
    // (The mbedTLS link line, when encryption is enabled, is emitted above —
    // the shared vendor/mbedtls static libs, since builtin_mbedtls=false.)
    if cfg!(target_os = "linux") {
        // Some librist sources call pthread fns; the link line needs -pthread.
        println!("cargo:rustc-link-lib=dylib=pthread");
    }

    // librist's build also generates `librist_config.h` (and vcs_version.h)
    // into <build_dir>/include/librist/. peer.h `#include`s this generated
    // header via a relative include (`#include "librist_config.h"`), so the
    // include path must reach the librist/ subdirectory directly.
    vec![
        vendor_dir.join("include"),
        build_dir.join("include").join("librist"),
        build_dir.join("include"),
    ]
}

fn require_tool(name: &str) {
    if which(name).is_none() {
        panic!(
            "Required build tool `{name}` not found on $PATH. \
             librist requires meson + ninja for the vendored build path. \
             Debian/Ubuntu: `sudo apt install meson ninja-build`. \
             macOS: `brew install meson ninja`. \
             Windows: `choco install meson ninja`."
        );
    }
}

fn which(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    // On Windows an executable carries an extension (meson.exe, ninja.exe, and
    // meson is often a .cmd/.bat shim), so the bare `name` is rarely a file on
    // disk. Probe `name` plus each PATHEXT extension. On Unix the candidate
    // list is just the bare name, preserving the original behaviour.
    let mut names = vec![name.to_string()];
    if cfg!(windows) {
        let pathext = env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.BAT;.CMD".to_string());
        for ext in pathext.split(';').filter(|e| !e.is_empty()) {
            names.push(format!("{name}{ext}"));
        }
    }
    for dir in env::split_paths(&path) {
        for candidate_name in &names {
            let candidate = dir.join(candidate_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn run_cmd(prog: &str, args: &[String], cwd: &Path, envs: &[(String, String)]) {
    let mut cmd = Command::new(prog);
    cmd.args(args).current_dir(cwd);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("Failed to spawn `{prog}`: {e}"));
    if !status.success() {
        panic!(
            "Command `{prog} {}` exited with status {status} (cwd={})",
            args.join(" "),
            cwd.display()
        );
    }
}
