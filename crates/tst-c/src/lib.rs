//! `tst-c` — stable C ABI for the ts-transformer pipeline.
//!
//! This crate is binding-only. Rust callers should consume `tst-pipeline`
//! and `tst-srt` directly. The C ABI is documented in `include/tstrans.h`
//! (cbindgen-generated, committed to the source tree).
//!
//! Sender side complete; receiver side complete — raw byte, TS-aligned,
//! and typed demux-event surfaces all ship today (`tst_raw_receiver_*` /
//! `tst_ts_receiver_*` / `tst_receiver_*` / `tst_demux_receiver_*`),
//! along with the reconnecting `tst_managed_*` variants. ABI minor is
//! `0.5` (see [`TST_ABI_VERSION_MINOR`]).

#![allow(clippy::missing_safety_doc)] // every extern "C" fn has a /// header documenting the contract

// Cross-cutting (shared by both sender and receiver):
pub mod config;
pub mod demux_config;
pub mod error;
pub mod event;
pub mod handle;
pub mod stats;
mod ffi_slice;
mod panic;

// Sender-side surface:
pub mod sender;

// Receiver-side surface:
pub mod receiver;
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
/// **Initial value:** `0.1` (pre-1.0). The minor field bumps freely
/// during pre-1.0 development on any breaking ABI change. Settles to
/// `1.0` at the first stable release.
///
/// Cbindgen emits this as `#define TST_ABI_VERSION_MAJOR 0` in the
/// generated header. Runtime accessor: [`tst_get_abi_version_major`].
pub const TST_ABI_VERSION_MAJOR: libc::c_int = 0;

/// Minor version of the C ABI contract. See [`TST_ABI_VERSION_MAJOR`]
/// for the bump policy.
///
/// Cbindgen emits this as `#define TST_ABI_VERSION_MINOR 5` in the
/// generated header. Runtime accessor: [`tst_get_abi_version_minor`].
///
/// History (additive bumps only — major stays at 0 pre-1.0):
/// - `1` (plan #62): receiver-surface initial drop.
/// - `2` (validate-1 Phase 2 wrap-up `d711ecb`): TS-bytes raw-receiver
///   pull-loop hardening + F2 C-ABI shape additions.
/// - `3` (AU cell reassembly `5527a9e`): `TstMultiCellAuReason` +
///   `multi_cell_au_reason` field on `TstEventNonConformant`.
/// - `4` (AU cell CFI tolerance): `TstNonConformantCode::CfiTolerated`
///   (= 32) + `TstCellFragmentIndication` enum + `tst_demux_config_set_cfi_tolerance`
///   setter. The new variant reuses the existing `cc_expected` + `cc_observed`
///   field carriers to surface `observed_cfi` + `treated_as` without growing
///   the struct.
/// - `5` (plan #96 demuxer-config parity, 2026-05-25):
///   `TstAv1CarriageMode` enum (mux side already had a mirror;
///   demux side reuses it) + three new C entry points —
///   `tst_demux_config_set_av1_carriage`,
///   `tst_demux_config_set_au_cell_cap_per_pid`, and
///   `tst_demux_config_set_lenient_psi_reassembly`. Bridges
///   Rust-only demux knobs through the C builder.
pub const TST_ABI_VERSION_MINOR: libc::c_int = 5;

// =========================================================================
// Runtime version accessors
// =========================================================================
//
// Bindings (srt-jni, srt-uniffi, pure-C consumers) use these to verify
// the loaded shared object matches the header they compiled against:
//
//     assert(tst_get_abi_version_major() == TST_ABI_VERSION_MAJOR);
//     assert(tst_get_abi_version_minor() >= TST_ABI_VERSION_MINOR);
//
// See examples/c/getting-started/version_check.c for the canonical pattern.

/// Returns the C ABI contract major version at runtime.
///
/// Always returns the value of [`TST_ABI_VERSION_MAJOR`] cast to `u32`.
/// Use the compile-time macro `TST_ABI_VERSION_MAJOR` in `tstrans.h` to
/// learn what the header you compiled against expects; compare against
/// this runtime value to detect SO/header mismatches.
///
/// # Safety
///
/// Sound under any caller invocation — no pointer arguments, no
/// mutating state, no internal locks. The `unsafe extern "C"`
/// annotation matches the convention of every other `tst_*` entry
/// point for consistency.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_get_abi_version_major() -> u32 {
    TST_ABI_VERSION_MAJOR as u32
}

/// Returns the C ABI contract minor version at runtime.
///
/// See [`tst_get_abi_version_major`] for the binding-author usage
/// pattern and the bump policy.
///
/// # Safety
///
/// Sound under any caller invocation; see [`tst_get_abi_version_major`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_get_abi_version_minor() -> u32 {
    TST_ABI_VERSION_MINOR as u32
}

// =========================================================================
// Package version (matches Cargo.toml)
// =========================================================================
//
// Bindings query these at runtime to surface the loaded library's
// version to their consumers (e.g., `tstrans.version` in a Java toString).
// Distinct from the ABI version above — the package version bumps every
// release (patches/minors/majors per SemVer), while the ABI version only
// bumps on breaking C-ABI change.

/// Returns the package major version at runtime — matches
/// `Cargo.toml`'s major field at the time `libtstrans` was built.
///
/// Always equal to [`TST_VERSION_MAJOR`] cast to `u32`. Cross-validate
/// against the compile-time header macro to detect SO/header mismatches:
///
/// ```c
/// if (tst_get_version_major() != TST_VERSION_MAJOR) {
///     fprintf(stderr, "tstrans header/SO version mismatch\n");
///     return 1;
/// }
/// ```
///
/// # Safety
///
/// Sound under any caller invocation; see [`tst_get_abi_version_major`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_get_version_major() -> u32 {
    TST_VERSION_MAJOR as u32
}

/// Returns the package minor version at runtime. See
/// [`tst_get_version_major`] for the usage pattern.
///
/// # Safety
///
/// Sound under any caller invocation; see [`tst_get_version_major`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_get_version_minor() -> u32 {
    TST_VERSION_MINOR as u32
}

/// Returns the package patch version at runtime. See
/// [`tst_get_version_major`] for the usage pattern.
///
/// # Safety
///
/// Sound under any caller invocation; see [`tst_get_version_major`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_get_version_patch() -> u32 {
    TST_VERSION_PATCH as u32
}

/// Returns the package version packed as `(M << 16) | (m << 8) | p`.
///
/// Lets binding authors compare versions as single integers:
///
/// ```c
/// // "at least 0.1.2" check
/// if (tst_get_version_packed() < ((0 << 16) | (1 << 8) | 2)) {
///     fprintf(stderr, "tstrans too old\n");
///     return 1;
/// }
/// ```
///
/// Each field caps at 255 (the encoding uses 8 bits per field with the
/// major field shifted into the upper 8 bits of a 24-bit window). Pre-1.0
/// values fit comfortably; revisit if any field ever exceeds 255.
///
/// Convention matches libsrt's `SRT_VERSION_*` packing.
///
/// # Safety
///
/// Sound under any caller invocation; see [`tst_get_version_major`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_get_version_packed() -> u32 {
    let m = TST_VERSION_MAJOR as u32;
    let n = TST_VERSION_MINOR as u32;
    let p = TST_VERSION_PATCH as u32;
    (m << 16) | (n << 8) | p
}

/// Returns a NUL-terminated `"<major>.<minor>.<patch>"` C string at
/// runtime.
///
/// Pointer is valid for the process lifetime (backed by a `'static`
/// Rust string created at compile time via `concat!` of the
/// `env!("CARGO_PKG_VERSION_*")` variables). Caller must NOT free.
///
/// ```c
/// printf("tstrans version: %s\n", tst_get_version_string());
/// ```
///
/// # Safety
///
/// Sound under any caller invocation; the returned pointer is always
/// non-NULL and process-lifetime stable. Reading past the NUL byte is
/// undefined behavior per usual C string rules.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_get_version_string() -> *const libc::c_char {
    // Construct the NUL-terminated string at compile time from the
    // Cargo-package env vars. The trailing `\0` extends the &str's
    // backing storage by one byte so `.as_ptr()` yields a valid C
    // string. Single source of truth: Cargo.toml.
    static VERSION: &str = concat!(
        env!("CARGO_PKG_VERSION_MAJOR"),
        ".",
        env!("CARGO_PKG_VERSION_MINOR"),
        ".",
        env!("CARGO_PKG_VERSION_PATCH"),
        "\0",
    );
    VERSION.as_ptr() as *const libc::c_char
}
