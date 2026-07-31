# tstrans-mbedtls-src

Vendored Mbed TLS 3.6.x (LTS) source tree, packaged as a build-time source
provider for the tstrans native transport crates (`tstrans-srt-sys`,
`tstrans-rist-sys`). It ships no bindings and builds no code itself — its
entire API is `source_dir() -> PathBuf`, which resolves the packaged source
tree so a consumer's own `build.rs` can compile it with its own flags
(`USE_ENCLIB=mbedtls`, sanitizer instrumentation, …).

**Stability: Internal.** This crate is an implementation detail of the
tstrans workspace. You should not depend on it directly.

Upstream: <https://github.com/Mbed-TLS/mbedtls>, pinned via git submodule
at `vendor/mbedtls` (v3.6.x LTS). Licensed Apache-2.0 OR
GPL-2.0-or-later — see `vendor/mbedtls/LICENSE`.
