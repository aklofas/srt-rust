//! PyO3 bindings for the ts-transformer Rust workspace.
//!
//! The compiled cdylib is imported from Python as `tstrans._native`;
//! the top-level `tstrans/__init__.py` re-exports the public surface
//! into submodules so users `from tstrans.mpegts import Muxer`.
//!
//! This crate currently exports only `__version__`, the exception
//! handles, and a test helper — type wrappers ship in later phase
//! plans.

mod errors;
mod mpegts;

use pyo3::prelude::*;

// `_py` is prefixed because Phase 0+1 doesn't use it directly; Phase
// 2+ (PyClass type wrappers) will. CI runs `clippy -D warnings` so an
// unprefixed unused parameter would fail the workspace.
#[pymodule]
fn _native(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_function(wrap_pyfunction!(errors::raise_mux_error_for_test, m)?)?;
    Ok(())
}
