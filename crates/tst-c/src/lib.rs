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

/// Major version of the C ABI contract. Bumped only on **breaking
/// C-ABI change** — i.e., a change that would force a consumer to
/// rebuild against a different `tstrans.h`. NOT bumped on:
///
/// - Cargo package version bumps (track `TST_VERSION_*` for that).
/// - Adding new `tst_*` functions or `TST_*` macros (backwards-compatible).
/// - Adding new `TstError::*` codes (backwards-compatible; old codes
///   remain stable per the documented `#[repr(i32)]` policy in
///   `crates/tst-c/src/error.rs`).
///
/// Bumped on:
///
/// - Removing or renaming any `tst_*` symbol.
/// - Changing a struct layout / size in a way that breaks
///   `_Static_assert(sizeof(...) == N)` lines in the header trailer.
/// - Changing the signature of an existing `tst_*` function.
/// - Changing the semantic contract of an existing function in a way
///   that would silently miscompile or misbehave in pre-existing
///   consumers (e.g., flipping return-code polarity, changing
///   buffer-ownership semantics).
///
/// **Initial value:** `0.1` (pre-1.0). Bumps to `0.2`, `0.3`, ... during
/// pre-1.0 breakage per `feedback_break_freely_prerelease.md`. Settles
/// to `1.0` at first stable release.
///
/// Cbindgen emits this as `#define TST_ABI_VERSION_MAJOR 0` in the
/// generated header. Runtime accessor: [`tst_get_abi_version_major`].
pub const TST_ABI_VERSION_MAJOR: libc::c_int = 0;

/// Minor version of the C ABI contract. See [`TST_ABI_VERSION_MAJOR`]
/// for the bump policy.
///
/// Cbindgen emits this as `#define TST_ABI_VERSION_MINOR 1` in the
/// generated header. Runtime accessor: [`tst_get_abi_version_minor`].
pub const TST_ABI_VERSION_MINOR: libc::c_int = 1;
