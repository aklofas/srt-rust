//! `srt-c` — stable C ABI for the `srt-core` sender pipeline.
//!
//! This crate is binding-only. Rust callers should consume `srt-core`
//! directly. The C ABI is documented in `include/srtc.h` (cbindgen-generated,
//! committed to the source tree).

#![allow(clippy::missing_safety_doc)] // every extern "C" fn has a /// header documenting the contract

pub mod config;
mod connect;
pub mod error;
mod handle;
pub mod mux_sender;
pub mod muxer;
pub mod raw_sender;
pub mod ts_sender;
/// Major version (compile-time macro in the generated header).
pub const SRTC_VERSION_MAJOR: libc::c_int = 0;
/// Minor version.
pub const SRTC_VERSION_MINOR: libc::c_int = 1;
/// Patch version.
pub const SRTC_VERSION_PATCH: libc::c_int = 0;
