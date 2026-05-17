//! `tst-c` — stable C ABI for the ts-transformer pipeline.
//!
//! This crate is binding-only. Rust callers should consume `tst-pipeline`
//! and `tst-srt` directly. The C ABI is documented in `include/tstrans.h`
//! (cbindgen-generated, committed to the source tree).
//!
//! Sender side complete; receiver side raw-layer complete (Phase 1
//! shipped 2026-05-15). Receiver TS-aligned layer (Phase 2) and typed
//! demux event surface (Phase 3) pending — see the receiver-surface
//! design doc + ROADMAP.

#![allow(clippy::missing_safety_doc)] // every extern "C" fn has a /// header documenting the contract

pub mod config;
mod connect;
pub mod demux_config;
pub mod demux_receiver;
pub mod error;
pub mod event;
pub mod handle;
mod listen;
pub mod mux_sender;
pub mod muxer;
mod panic;
pub mod raw_receiver;
pub mod raw_sender;
pub mod stats;
pub mod ts_receiver;
pub mod ts_sender;
/// Re-exports of internal error helpers for integration tests. These are not
/// `extern "C"` and do not appear in `tstrans.h`. Named with `test_` prefix
/// to mark their test-only intent; not gated on `#[cfg(test)]` because
/// integration tests in `tests/` are separate crates that cannot see
/// `pub(crate)` items.
pub use error::{
    test_clear_last_error, test_last_error_code, test_last_error_msg, test_record_shell_error,
};
/// Major version (compile-time macro in the generated header).
pub const TST_VERSION_MAJOR: libc::c_int = 0;
/// Minor version.
pub const TST_VERSION_MINOR: libc::c_int = 1;
/// Patch version.
pub const TST_VERSION_PATCH: libc::c_int = 0;
