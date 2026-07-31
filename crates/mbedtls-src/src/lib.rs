//! Vendored Mbed TLS source tree for the tstrans `-sys` crates.
//!
//! This crate ships no bindings and builds no code. Its single function
//! resolves the packaged Mbed TLS source directory so consumer build
//! scripts (tstrans-srt-sys, tstrans-rist-sys) can compile it with their
//! own flags (`USE_ENCLIB=mbedtls`, sanitizer instrumentation, …).
//!
//! Upstream: <https://github.com/Mbed-TLS/mbedtls>, pinned by the
//! submodule at `vendor/mbedtls` (v3.6.x LTS). Licensed Apache-2.0 OR
//! GPL-2.0-or-later (see `vendor/mbedtls/LICENSE`).

use std::path::PathBuf;

/// Absolute path of the vendored Mbed TLS source tree.
pub fn source_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/mbedtls")
}

#[cfg(test)]
mod tests {
    #[test]
    fn source_tree_is_present() {
        let d = super::source_dir();
        assert!(d.join("CMakeLists.txt").is_file(), "mbedTLS tree missing at {d:?} — did `git submodule update --init` run?");
        assert!(d.join("LICENSE").is_file());
    }
}
