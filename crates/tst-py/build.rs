fn main() {
    pyo3_build_config::add_extension_module_link_args();

    // Dual-mbedTLS coexistence (Plan A5b). The default tst-py build now
    // enables both `srt` (libsrt -> workspace vendor/mbedtls) and `rist`
    // (librist -> its own contrib/mbedtls), so the extension module links
    // two static mbedTLS copies with duplicate `mbedtls_*` symbols. On Linux
    // the default linker errors (`multiple definition of mbedtls_sha256_init`).
    // `-Wl,--allow-multiple-definition` collapses every reference onto the
    // first definition (libsrt's), so the extension links AND both libraries
    // share one consistent mbedTLS at runtime (same fix + rationale as
    // crates/tst-c/build.rs; the clean cross-crate-reuse fix is the rist-sys
    // v2 follow-up). Scoped to the srt+rist combo so single-transport builds
    // keep strict duplicate-symbol checking.
    #[cfg(target_os = "linux")]
    {
        if std::env::var("CARGO_FEATURE_SRT").is_ok() && std::env::var("CARGO_FEATURE_RIST").is_ok()
        {
            println!("cargo:rustc-link-arg=-Wl,--allow-multiple-definition");
        }
    }
}
