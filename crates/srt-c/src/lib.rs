//! `srt-c` — stable C ABI for the `srt-core` sender pipeline.
//!
//! This crate is binding-only. Rust callers should consume `srt-core`
//! directly. The C ABI is documented in `include/srtc.h` (cbindgen-generated,
//! committed to the source tree); the design lives at
//! `~/Projects/srt/docs/specs/2026-05-01-srt-c-design.md` (in the parent
//! workspace, not in this repo).

#![allow(clippy::missing_safety_doc)] // every extern "C" fn has a /// header documenting the contract

pub mod config;
pub mod error;
mod handle;
mod url;

/// Major version (compile-time macro in the generated header).
#[unsafe(no_mangle)]
pub static SRTC_VERSION_MAJOR: libc::c_int = 0;
/// Minor version.
#[unsafe(no_mangle)]
pub static SRTC_VERSION_MINOR: libc::c_int = 1;
/// Patch version.
#[unsafe(no_mangle)]
pub static SRTC_VERSION_PATCH: libc::c_int = 0;
