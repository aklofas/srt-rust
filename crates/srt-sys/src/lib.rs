//! Raw FFI bindings to Haivision libsrt 1.5+.
//!
//! This crate exposes the C ABI of [libsrt] verbatim. Every symbol is
//! `unsafe`; calling them incorrectly will exhibit C-level UB.
//!
//! The safe wrappers live in the `tst-srt` crate (`Socket` / `Listener`
//! / `SocketBuilder` / `ListenerBuilder`) and the `tst-pipeline` crate
//! (`MuxSender` / `Sender` / `RawSender` and their receive-side
//! counterparts). Application code should depend on those, not this
//! crate.
//!
//! # Build
//!
//! By default, the build script tries [`pkg-config`] first. If a
//! system libsrt isn't found (or `SRT_FORCE_VENDORED` is set), it
//! compiles the vendored copy at `vendor/srt` (a git submodule pinned
//! at libsrt v1.5.6) via `cmake` and links it statically.
//!
//! Encryption is on by default via the `mbedtls` cargo feature, which
//! builds the vendored mbedTLS submodule and links it into libsrt with
//! `USE_ENCLIB=mbedtls`. Disable with `--no-default-features` for an
//! `ENABLE_ENCRYPTION=OFF` build.
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
