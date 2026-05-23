//! PyO3 wrappers for `tst_core::klv::*` typed sets.
//!
//! Translation strategy: each Rust `*Ls` / `*Pack` struct is converted
//! to an instance of a Python-side dataclass under `tstrans.klv.*` via
//! per-set translator functions (`convert_uas_datalink`, etc.). Decode
//! entry points are `#[pyfunction]`s that map `KlvDecodeError` to
//! `tstrans.exceptions.KlvError` via `make_klv_error`.
//!
//! Phase 3 ships: ST 0601 / ST 0102 / ST 0605 / ST 0903 decode +
//! field-error surfacing. Encode lands with Phase 4 (Muxer wrap).
//!
//! `#![allow(...)]` mirrors the pattern in `errors.rs` and `mpegts.rs` —
//! PyO3 0.22 + Rust 2024 macro expansions trip these lints. Hand-
//! written code in this module has no unsafe blocks.
#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use pyo3::prelude::*;

/// Register all klv-domain bindings with the parent `_native` module.
/// Called from `lib.rs::_native`. Tasks 4-10 fill this in.
pub fn register(_m: &Bound<'_, PyModule>) -> PyResult<()> {
    Ok(())
}
