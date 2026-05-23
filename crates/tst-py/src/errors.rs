//! Rust-side helpers that construct the Python exception classes
//! defined in `tstrans.exceptions`. Phase 2+ plans use these from
//! within type wrappers — e.g. `Demuxer.feed_bytes` calls
//! `make_demux_error(py, "BAD_PMT", "...")` when the underlying
//! `tst_core::mpegts::Demuxer` returns an error.
//!
//! Implementation note: we deliberately do NOT use PyO3's
//! `create_exception!` (which would mint *new* exception classes on
//! the Rust side, distinct from the Python-defined `class MuxError`).
//! Users need `isinstance(err, tstrans.exceptions.MuxError)` to work
//! whether the error comes from Python or Rust — so the Rust side
//! must *call into* the Python-defined classes, which is what
//! `py.import_bound("tstrans.exceptions").getattr("MuxError")?` does.
//! This is slower than `create_exception!` (per-raise dict lookup +
//! Python call) but the tradeoff is required for the contract.

// PyO3's `#[pyfunction]` macro (Rust 2024 edition) generates extractor
// code that calls `pyo3::impl_::extract_argument::unwrap_required_argument`
// — an unsafe fn — without an explicit `unsafe {}` block in the expansion.
// The `useless_conversion` allow covers a `PyErr -> PyErr` `.into()` emitted
// by the same macro. Both suppressions are scoped to macro-generated code
// only; hand-written code in this file contains no unsafe blocks.
#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Build a `MuxError` Python exception with the right `.kind` Enum
/// value and `message` attribute. Phase 4 (Muxer wrap) uses this from
/// inside the `Muxer.push_video` / `push_klv` / `push_audio` wrappers.
///
/// `kind_variant` is the Python-side `MuxErrorKind` Enum variant name
/// (e.g. `"INVALID_CONFIG"`, `"INTERNAL"`). Caller must pass a valid
/// variant — invalid names raise `AttributeError` from
/// `MuxErrorKind.<NAME>` lookup, which surfaces as a `PyErr` and is
/// returned in place of the intended `MuxError`.
pub fn make_mux_error(py: Python<'_>, kind_variant: &str, message: &str) -> PyErr {
    let exceptions = match py.import_bound("tstrans.exceptions") {
        Ok(m) => m,
        Err(e) => return e,
    };
    let kind_enum = match exceptions.getattr(intern!(py, "MuxErrorKind")) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let kind_value = match kind_enum.getattr(kind_variant) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let mux_error_cls = match exceptions.getattr(intern!(py, "MuxError")) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let kwargs = PyDict::new_bound(py);
    if let Err(e) = kwargs.set_item("kind", kind_value) {
        return e;
    }
    if let Err(e) = kwargs.set_item("message", message) {
        return e;
    }
    match mux_error_cls.call((), Some(&kwargs)) {
        Ok(instance) => PyErr::from_value_bound(instance),
        Err(e) => e,
    }
}

/// Test helper: forces a `MuxError` raise from Rust, used by
/// `test_error_wiring.py` to confirm end-to-end wiring. Exposed only
/// under the `_native._raise_mux_error_for_test` name.
#[pyfunction]
#[pyo3(name = "_raise_mux_error_for_test")]
pub fn raise_mux_error_for_test(py: Python<'_>, message: &str) -> PyResult<()> {
    Err(make_mux_error(py, "INTERNAL", message))
}
