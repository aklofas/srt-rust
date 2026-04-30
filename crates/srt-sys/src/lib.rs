//! Raw FFI bindings to Haivision libsrt 1.5+.
//!
//! This crate exposes the C ABI of [libsrt] verbatim. Every symbol is
//! `unsafe`; calling them incorrectly will exhibit C-level UB.
//!
//! The safe wrapper lives in the `srt-core` crate. Application code
//! should depend on `srt-core`, not this crate.
//!
//! # Build
//!
//! By default, the build script tries [`pkg-config`] first. If a
//! system libsrt isn't found (or `SRT_FORCE_VENDORED` is set), it
//! compiles the vendored copy at `vendor/srt` (a git submodule pinned
//! at libsrt v1.5.5) via `cmake` and links it statically.
//!
//! Encryption is disabled in v0; re-enabling it (with mbedTLS or
//! vendored OpenSSL) is a focused follow-up plan.
//!
//! Build environment variables:
//!
//! - `SRT_FORCE_VENDORED=1` — skip pkg-config, always build the submodule
//! - `SRT_NO_PKG_CONFIG=1` — equivalent
//!
//! [libsrt]: https://github.com/Haivision/srt
//! [`pkg-config`]: https://docs.rs/pkg-config

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
