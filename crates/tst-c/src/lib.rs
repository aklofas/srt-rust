//! `tst-c` — stable C ABI for the ts-transformer pipeline.
//!
//! This crate is binding-only. Rust callers should consume `tst-pipeline`
//! and `tst-srt` directly. The C ABI is documented in `include/tstrans.h`
//! (cbindgen-generated, committed to the source tree).
//!
//! Sender side is complete; receiver side is feature-complete in Rust
//! and pending C ABI exposure.

#![allow(clippy::missing_safety_doc)] // every extern "C" fn has a /// header documenting the contract

pub mod config;
mod connect;
mod listen;
pub mod error;
pub mod handle;
pub mod mux_sender;
pub mod muxer;
mod panic;
pub mod raw_sender;
pub mod stats;
pub mod ts_sender;
/// Major version (compile-time macro in the generated header).
pub const TST_VERSION_MAJOR: libc::c_int = 0;
/// Minor version.
pub const TST_VERSION_MINOR: libc::c_int = 1;
/// Patch version.
pub const TST_VERSION_PATCH: libc::c_int = 0;
