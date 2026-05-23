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
    AudioStreamHandle as RustAudioStreamHandle, Av1CarriageMode as RustAv1CarriageMode,
    KlvStreamHandle as RustKlvStreamHandle, KlvStreamType as RustKlvStreamType,
    SubtitleStreamHandle as RustSubtitleStreamHandle, VideoStreamHandle as RustVideoStreamHandle,
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

// ---------------------------------------------------------------------------
// Stream handle newtypes — one PyO3 wrapper per Rust handle kind.
// ---------------------------------------------------------------------------
//
// The Rust handles are `Copy + Eq + Hash` u32 newtypes. PyO3 needs the
// `frozen + eq + hash` pyclass flags to expose `__eq__` + `__hash__`
// based on the derived `PartialEq + Eq + Hash` impls. `from_raw` is a
// staticmethod so callers can round-trip handles through the C ABI or
// reconstruct them from saved state. Deliberate per-kind repetition
// (~25 LoC × 4) keeps each class trivially auditable; a macro would
// hide the `name` / `module` / repr-string per-kind variation that the
// Python tests rely on.

/// Opaque handle for a video stream within a configured muxer.
/// Obtain from `Muxer.video_handles()`. Equality + hash by raw `u32`.
#[pyclass(
    name = "VideoStreamHandle",
    module = "tstrans.mpegts",
    frozen,
    eq,
    hash
)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyVideoStreamHandle(pub(crate) RustVideoStreamHandle);

#[pymethods]
impl PyVideoStreamHandle {
    #[staticmethod]
    pub fn from_raw(raw: u32) -> Self {
        Self(RustVideoStreamHandle::from_raw(raw))
    }

    #[getter]
    pub fn raw(&self) -> u32 {
        self.0.raw()
    }

    pub fn unpack(&self) -> (usize, usize) {
        self.0.unpack()
    }

    fn __repr__(&self) -> String {
        format!("VideoStreamHandle(raw={})", self.0.raw())
    }
}

/// Opaque handle for an audio stream within a configured muxer.
/// Obtain from `Muxer.audio_handles()`. Equality + hash by raw `u32`.
#[pyclass(
    name = "AudioStreamHandle",
    module = "tstrans.mpegts",
    frozen,
    eq,
    hash
)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyAudioStreamHandle(pub(crate) RustAudioStreamHandle);

#[pymethods]
impl PyAudioStreamHandle {
    #[staticmethod]
    pub fn from_raw(raw: u32) -> Self {
        Self(RustAudioStreamHandle::from_raw(raw))
    }

    #[getter]
    pub fn raw(&self) -> u32 {
        self.0.raw()
    }

    pub fn unpack(&self) -> (usize, usize) {
        self.0.unpack()
    }

    fn __repr__(&self) -> String {
        format!("AudioStreamHandle(raw={})", self.0.raw())
    }
}

/// Opaque handle for a KLV stream within a configured muxer.
/// Obtain from `Muxer.klv_handles()`. Equality + hash by raw `u32`.
#[pyclass(name = "KlvStreamHandle", module = "tstrans.mpegts", frozen, eq, hash)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PyKlvStreamHandle(pub(crate) RustKlvStreamHandle);

#[pymethods]
impl PyKlvStreamHandle {
    #[staticmethod]
    pub fn from_raw(raw: u32) -> Self {
        Self(RustKlvStreamHandle::from_raw(raw))
    }

    #[getter]
    pub fn raw(&self) -> u32 {
        self.0.raw()
    }

    pub fn unpack(&self) -> (usize, usize) {
        self.0.unpack()
    }

    fn __repr__(&self) -> String {
        format!("KlvStreamHandle(raw={})", self.0.raw())
    }
}

/// Opaque handle for a subtitle stream within a configured muxer.
/// Obtain from `Muxer.subtitle_handles()`. Equality + hash by raw `u32`.
#[pyclass(
    name = "SubtitleStreamHandle",
    module = "tstrans.mpegts",
    frozen,
    eq,
    hash
)]
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct PySubtitleStreamHandle(pub(crate) RustSubtitleStreamHandle);

#[pymethods]
impl PySubtitleStreamHandle {
    #[staticmethod]
    pub fn from_raw(raw: u32) -> Self {
        Self(RustSubtitleStreamHandle::from_raw(raw))
    }

    #[getter]
    pub fn raw(&self) -> u32 {
        self.0.raw()
    }

    pub fn unpack(&self) -> (usize, usize) {
        self.0.unpack()
    }

    fn __repr__(&self) -> String {
        format!("SubtitleStreamHandle(raw={})", self.0.raw())
    }
}
