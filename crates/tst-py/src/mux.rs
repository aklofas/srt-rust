//! PyO3 wrappers for the Rust `tst_core::mpegts::mux` family. Houses
//! the config types (Tasks 4-5), the Muxer (Tasks 6-9), stream
//! handles (Task 3), and stats (Task 10).
//!
//! Python-side enums (`KlvStreamType`, `Av1CarriageMode`) and the
//! `StreamSpec` hierarchy live in `python/tstrans/mpegts.py` as pure
//! Python — no PyO3 wrap needed for them. The converters in this
//! file translate the string `.value` of the Python enum to the
//! Rust counterpart (and back), so later tasks can lift Python
//! configs onto the Rust muxer without re-deriving the mapping.

#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion, dead_code)]

use pyo3::prelude::*;

use tst_core::mpegts::mux::{
    Av1CarriageMode as RustAv1CarriageMode, KlvStreamType as RustKlvStreamType,
};

/// Translate a Python `KlvStreamType` enum value (string-valued) to
/// the Rust enum variant.
pub(crate) fn py_klv_stream_type(v: &Bound<'_, PyAny>) -> PyResult<RustKlvStreamType> {
    let s: String = v.getattr("value")?.extract()?;
    match s.as_str() {
        "synchronous_metadata" => Ok(RustKlvStreamType::SynchronousMetadata),
        "private_data" => Ok(RustKlvStreamType::PrivateData),
        other => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "unknown KlvStreamType: {other}"
        ))),
    }
}

/// Translate a Python `Av1CarriageMode` enum value (string-valued)
/// to the Rust enum variant.
pub(crate) fn py_av1_carriage(v: &Bound<'_, PyAny>) -> PyResult<RustAv1CarriageMode> {
    let s: String = v.getattr("value")?.extract()?;
    match s.as_str() {
        "mpeg2_ts_binding" => Ok(RustAv1CarriageMode::Mpeg2TsBinding),
        "interop_raw_obu" => Ok(RustAv1CarriageMode::InteropRawObu),
        other => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "unknown Av1CarriageMode: {other}"
        ))),
    }
}

/// Look up the Python `KlvStreamType.<NAME>` enum member matching
/// the given Rust variant.
pub(crate) fn klv_stream_type_to_py(
    py: Python<'_>,
    t: RustKlvStreamType,
) -> PyResult<Bound<'_, PyAny>> {
    let cls = py
        .import_bound("tstrans.mpegts")?
        .getattr("KlvStreamType")?;
    let name = match t {
        RustKlvStreamType::SynchronousMetadata => "SYNCHRONOUS_METADATA",
        RustKlvStreamType::PrivateData => "PRIVATE_DATA",
    };
    cls.getattr(name)
}

/// Look up the Python `Av1CarriageMode.<NAME>` enum member matching
/// the given Rust variant. The Rust enum is `#[non_exhaustive]`, so
/// unknown future variants surface as a `ValueError`.
pub(crate) fn av1_carriage_to_py(
    py: Python<'_>,
    m: RustAv1CarriageMode,
) -> PyResult<Bound<'_, PyAny>> {
    let cls = py
        .import_bound("tstrans.mpegts")?
        .getattr("Av1CarriageMode")?;
    let name = match m {
        RustAv1CarriageMode::Mpeg2TsBinding => "MPEG2_TS_BINDING",
        RustAv1CarriageMode::InteropRawObu => "INTEROP_RAW_OBU",
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "unknown Av1CarriageMode variant",
            ));
        }
    };
    cls.getattr(name)
}
