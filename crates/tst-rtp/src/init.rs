//! **Stability: Stable** — see the
//! [API stability reference](https://github.com/aklofas/ts-transformer/blob/main/docs/reference/api-stability.md).
//!
//! Crate-private no-op initialization, exposed for symmetry with
//! [`tst_srt::init`](https://docs.rs/tst-srt).
//!
//! `tst-rtp` is pure Rust: there's no library to start up the way
//! `srt_startup()` is required for libsrt. Binding-crate authors who
//! call `tst_srt::init()` in their startup path can also call
//! [`init`] here without special-casing; it's a no-op today and may
//! grow side effects (e.g., a tracing hook installation) in a later
//! phase without an ABI break.

/// Idempotent no-op. Safe to call from any thread, any number of times.
///
/// Provided for symmetry with `tst_srt::init` — see module-level docs.
pub fn init() {}
