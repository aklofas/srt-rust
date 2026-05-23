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
use pyo3::types::{PyByteArray, PyBytes, PyTuple};

use tst_core::mpegts::mux::{
    AudioCodec as RustAudioCodec, AudioStreamHandle as RustAudioStreamHandle,
    Av1CarriageMode as RustAv1CarriageMode, KlvStreamHandle as RustKlvStreamHandle,
    KlvStreamType as RustKlvStreamType, Muxer as RustMuxer, MuxerConfig as RustMuxerConfig,
    MuxerConfigBuilder as RustMuxerConfigBuilder, MuxerProgramConfig as RustMuxerProgramConfig,
    MuxerProgramConfigBuilder as RustMuxerProgramConfigBuilder, StreamSpec as RustStreamSpec,
    SubtitleCodec as RustSubtitleCodec, SubtitleStreamHandle as RustSubtitleStreamHandle,
    VideoCodec as RustVideoCodec, VideoStreamHandle as RustVideoStreamHandle,
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

// ---------------------------------------------------------------------------
// Codec enum converters — Python ↔ Rust (mux-side enums).
// ---------------------------------------------------------------------------
//
// Demux-side codec enums are unit variants; mux-side `VideoCodec` and
// `AudioCodec` are also unit variants (same shape); mux-side
// `SubtitleCodec` is a struct-variant enum (DvbSubtitling carries
// language + page IDs, etc.). Task 4 supports the unit-variant
// converters; the mux-side `SubtitleCodec → Python` rendering uses the
// flat Python `SubtitleCodec` enum (variant tag only). Construction
// of mux-side subtitles from Python is deferred to a future task when
// the Python `SubtitleCodec` enum gains structured payloads.

/// Translate a Python `VideoCodec` enum to the mux-side Rust variant.
fn py_video_codec(v: &Bound<'_, PyAny>) -> PyResult<RustVideoCodec> {
    let s: String = v.getattr("value")?.extract()?;
    match s.as_str() {
        "h264" => Ok(RustVideoCodec::H264),
        "h265" => Ok(RustVideoCodec::H265),
        "h266" => Ok(RustVideoCodec::H266),
        "av1" => Ok(RustVideoCodec::Av1),
        other => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "unknown VideoCodec: {other}"
        ))),
    }
}

/// Translate a Python `AudioCodec` enum to the mux-side Rust variant.
fn py_audio_codec(v: &Bound<'_, PyAny>) -> PyResult<RustAudioCodec> {
    let s: String = v.getattr("value")?.extract()?;
    match s.as_str() {
        "mp2" => Ok(RustAudioCodec::Mp2),
        "aac" => Ok(RustAudioCodec::Aac),
        "aac_latm" => Ok(RustAudioCodec::AacLatm),
        "ac3" => Ok(RustAudioCodec::Ac3),
        other => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "unknown AudioCodec: {other}"
        ))),
    }
}

/// Look up `tstrans.mpegts.VideoCodec.<NAME>` for a mux-side Rust variant.
fn video_codec_to_py(py: Python<'_>, c: RustVideoCodec) -> PyResult<Bound<'_, PyAny>> {
    let cls = py.import_bound("tstrans.mpegts")?.getattr("VideoCodec")?;
    let name = match c {
        RustVideoCodec::H264 => "H264",
        RustVideoCodec::H265 => "H265",
        RustVideoCodec::H266 => "H266",
        RustVideoCodec::Av1 => "AV1",
    };
    cls.getattr(name)
}

/// Look up `tstrans.mpegts.AudioCodec.<NAME>` for a mux-side Rust variant.
fn audio_codec_to_py(py: Python<'_>, c: RustAudioCodec) -> PyResult<Bound<'_, PyAny>> {
    let cls = py.import_bound("tstrans.mpegts")?.getattr("AudioCodec")?;
    let name = match c {
        RustAudioCodec::Mp2 => "MP2",
        RustAudioCodec::Aac => "AAC",
        RustAudioCodec::AacLatm => "AAC_LATM",
        RustAudioCodec::Ac3 => "AC3",
    };
    cls.getattr(name)
}

/// Look up `tstrans.mpegts.SubtitleCodec.<NAME>` for a mux-side
/// Rust variant — variant tag only (structured DVB / teletext
/// parameters on the Rust side are not surfaced; the flat Python
/// enum only carries the codec discriminator).
fn subtitle_codec_to_py<'py>(
    py: Python<'py>,
    c: &RustSubtitleCodec,
) -> PyResult<Bound<'py, PyAny>> {
    let cls = py
        .import_bound("tstrans.mpegts")?
        .getattr("SubtitleCodec")?;
    let name = match c {
        RustSubtitleCodec::DvbSubtitling { .. } => "DVB_SUBTITLING",
        RustSubtitleCodec::DvbTeletext { .. } => "DVB_TELETEXT",
        RustSubtitleCodec::Cea708Standalone => "CEA708_STANDALONE",
        RustSubtitleCodec::WebVttInTs => "WEBVTT_IN_TS",
    };
    cls.getattr(name)
}

// ---------------------------------------------------------------------------
// MuxerProgramConfig + MuxerProgramConfigBuilder — Task 4.
// ---------------------------------------------------------------------------

/// Frozen view of a built [`MuxerProgramConfig`] — one program in a
/// multi-program TS multiplex. Holds the PMT PID, program number,
/// PCR PID override, stream specs, and descriptors. Construct via
/// [`MuxerProgramConfigBuilder`].
#[pyclass(name = "MuxerProgramConfig", module = "tstrans.mpegts", frozen)]
#[derive(Clone)]
pub struct PyMuxerProgramConfig {
    pub(crate) inner: RustMuxerProgramConfig,
}

#[pymethods]
impl PyMuxerProgramConfig {
    #[getter]
    pub fn program_number(&self) -> u16 {
        self.inner.program_number
    }

    #[getter]
    pub fn pmt_pid(&self) -> u16 {
        self.inner.pmt_pid
    }

    #[getter]
    pub fn pcr_pid(&self) -> Option<u16> {
        self.inner.pcr_pid
    }

    /// Tuple of `StreamSpec` subclass instances mirroring
    /// `inner.streams` add-order. Returns the Python-side
    /// `VideoStreamSpec / KlvStreamSpec / AudioStreamSpec /
    /// SubtitleStreamSpec` dataclasses from `tstrans.mpegts`.
    #[getter]
    pub fn streams(&self, py: Python<'_>) -> PyResult<PyObject> {
        let mpegts_mod = py.import_bound("tstrans.mpegts")?;
        let mut items: Vec<PyObject> = Vec::with_capacity(self.inner.streams.len());
        for s in &self.inner.streams {
            let obj = match s {
                RustStreamSpec::Video { pid, codec } => {
                    let cls = mpegts_mod.getattr("VideoStreamSpec")?;
                    let codec_obj = video_codec_to_py(py, *codec)?;
                    cls.call1((*pid, codec_obj))?.unbind()
                }
                RustStreamSpec::Klv {
                    pid,
                    stream_type,
                    carries_pts,
                } => {
                    let cls = mpegts_mod.getattr("KlvStreamSpec")?;
                    let st_obj = klv_stream_type_to_py(py, *stream_type)?;
                    cls.call1((*pid, st_obj, *carries_pts))?.unbind()
                }
                RustStreamSpec::Audio {
                    pid,
                    codec,
                    language,
                } => {
                    let cls = mpegts_mod.getattr("AudioStreamSpec")?;
                    let codec_obj = audio_codec_to_py(py, *codec)?;
                    let lang_obj: PyObject = match language {
                        Some(l) => PyBytes::new_bound(py, l).unbind().into(),
                        None => py.None(),
                    };
                    cls.call1((*pid, codec_obj, lang_obj))?.unbind()
                }
                RustStreamSpec::Subtitle { pid, codec } => {
                    let cls = mpegts_mod.getattr("SubtitleStreamSpec")?;
                    let codec_obj = subtitle_codec_to_py(py, codec)?;
                    cls.call1((*pid, codec_obj))?.unbind()
                }
            };
            items.push(obj);
        }
        Ok(PyTuple::new_bound(py, items).unbind().into())
    }

    /// Tuple of program-level descriptor TLV bytes (PMT program info
    /// loop, before per-stream entries).
    #[getter]
    pub fn program_descriptors(&self, py: Python<'_>) -> PyObject {
        let items: Vec<PyObject> = self
            .inner
            .program_descriptors
            .iter()
            .map(|d| PyBytes::new_bound(py, d).unbind().into())
            .collect();
        PyTuple::new_bound(py, items).unbind().into()
    }

    /// Tuple-of-tuples of per-stream descriptor TLVs. Outer indexed
    /// parallel to `streams`; inner is the descriptor list for that
    /// stream.
    #[getter]
    pub fn stream_descriptors(&self, py: Python<'_>) -> PyObject {
        let outer: Vec<PyObject> = self
            .inner
            .stream_descriptors
            .iter()
            .map(|descs| {
                let inner: Vec<PyObject> = descs
                    .iter()
                    .map(|d| PyBytes::new_bound(py, d).unbind().into())
                    .collect();
                PyTuple::new_bound(py, inner).unbind().into()
            })
            .collect();
        PyTuple::new_bound(py, outer).unbind().into()
    }
}

/// Chainable builder for [`PyMuxerProgramConfig`]. Each `add_*` call
/// appends an elementary stream and returns `self` for fluent
/// chaining. Terminal `build()` returns the frozen config.
///
/// Mirrors `tst_core::mpegts::mux::MuxerProgramConfigBuilder`. The
/// inner Rust builder is held in an `Option<...>` so `build()` can
/// take `&self` (the Rust API uses `&self` and clones internally).
#[pyclass(name = "MuxerProgramConfigBuilder", module = "tstrans.mpegts")]
pub struct PyMuxerProgramConfigBuilder {
    inner: Option<RustMuxerProgramConfigBuilder>,
}

impl PyMuxerProgramConfigBuilder {
    fn get_mut(&mut self) -> PyResult<&mut RustMuxerProgramConfigBuilder> {
        self.inner.as_mut().ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err("MuxerProgramConfigBuilder is consumed")
        })
    }
}

#[pymethods]
impl PyMuxerProgramConfigBuilder {
    #[new]
    pub fn new(program_number: u16, pmt_pid: u16) -> Self {
        Self {
            inner: Some(RustMuxerProgramConfigBuilder::new(program_number, pmt_pid)),
        }
    }

    pub fn add_video<'py>(
        mut slf: PyRefMut<'py, Self>,
        pid: u16,
        codec: &Bound<'_, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let rust_codec = py_video_codec(codec)?;
        slf.get_mut()?.add_video(pid, rust_codec);
        Ok(slf)
    }

    #[pyo3(signature = (pid, stream_type, *, carries_pts))]
    pub fn add_klv<'py>(
        mut slf: PyRefMut<'py, Self>,
        pid: u16,
        stream_type: &Bound<'_, PyAny>,
        carries_pts: bool,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let rust_st = py_klv_stream_type(stream_type)?;
        slf.get_mut()?.add_klv(pid, rust_st, carries_pts);
        Ok(slf)
    }

    pub fn add_audio<'py>(
        mut slf: PyRefMut<'py, Self>,
        pid: u16,
        codec: &Bound<'_, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let rust_codec = py_audio_codec(codec)?;
        slf.get_mut()?.add_audio(pid, rust_codec);
        Ok(slf)
    }

    #[pyo3(signature = (pid, codec, *, language))]
    pub fn add_audio_with_language<'py>(
        mut slf: PyRefMut<'py, Self>,
        pid: u16,
        codec: &Bound<'_, PyAny>,
        language: &[u8],
    ) -> PyResult<PyRefMut<'py, Self>> {
        if language.len() != 3 {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "language must be 3 bytes (ISO 639-2), got {}",
                language.len()
            )));
        }
        let mut lang = [0u8; 3];
        lang.copy_from_slice(language);
        let rust_codec = py_audio_codec(codec)?;
        slf.get_mut()?
            .add_audio_with_language(pid, rust_codec, lang);
        Ok(slf)
    }

    /// Mux-side subtitles need structured per-variant data
    /// (language, page IDs, ...) that the flat Python `SubtitleCodec`
    /// enum doesn't carry. Deferred to a future task.
    pub fn add_subtitle<'py>(
        _slf: PyRefMut<'py, Self>,
        _pid: u16,
        _codec: &Bound<'_, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        Err(pyo3::exceptions::PyNotImplementedError::new_err(
            "add_subtitle is not yet wired in Python; mux-side SubtitleCodec \
             variants carry structured fields (language, page IDs) that the \
             Python SubtitleCodec enum does not yet model. Deferred to a \
             future tst-py task.",
        ))
    }

    pub fn pcr_pid(mut slf: PyRefMut<'_, Self>, pid: u16) -> PyResult<PyRefMut<'_, Self>> {
        slf.get_mut()?.pcr_pid(pid);
        Ok(slf)
    }

    pub fn program_descriptors(
        mut slf: PyRefMut<'_, Self>,
        descs: Vec<Vec<u8>>,
    ) -> PyResult<PyRefMut<'_, Self>> {
        slf.get_mut()?.program_descriptors(descs);
        Ok(slf)
    }

    pub fn stream_descriptors_for_video<'py>(
        mut slf: PyRefMut<'py, Self>,
        py: Python<'py>,
        video_idx: usize,
        descs: Vec<Vec<u8>>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.get_mut()?
            .stream_descriptors_for_video(video_idx, descs)
            .map_err(|e| crate::errors::mux_error_to_pyerr(py, e))?;
        Ok(slf)
    }

    pub fn stream_descriptors_for_klv<'py>(
        mut slf: PyRefMut<'py, Self>,
        py: Python<'py>,
        klv_idx: usize,
        descs: Vec<Vec<u8>>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.get_mut()?
            .stream_descriptors_for_klv(klv_idx, descs)
            .map_err(|e| crate::errors::mux_error_to_pyerr(py, e))?;
        Ok(slf)
    }

    pub fn stream_descriptors_for_audio<'py>(
        mut slf: PyRefMut<'py, Self>,
        py: Python<'py>,
        audio_idx: usize,
        descs: Vec<Vec<u8>>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.get_mut()?
            .stream_descriptors_for_audio(audio_idx, descs)
            .map_err(|e| crate::errors::mux_error_to_pyerr(py, e))?;
        Ok(slf)
    }

    pub fn stream_descriptors_for_subtitle<'py>(
        mut slf: PyRefMut<'py, Self>,
        py: Python<'py>,
        subtitle_idx: usize,
        descs: Vec<Vec<u8>>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.get_mut()?
            .stream_descriptors_for_subtitle(subtitle_idx, descs)
            .map_err(|e| crate::errors::mux_error_to_pyerr(py, e))?;
        Ok(slf)
    }

    pub fn stream_descriptors_for_stream<'py>(
        mut slf: PyRefMut<'py, Self>,
        py: Python<'py>,
        abs_idx: usize,
        descs: Vec<Vec<u8>>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.get_mut()?
            .stream_descriptors_for_stream(abs_idx, descs)
            .map_err(|e| crate::errors::mux_error_to_pyerr(py, e))?;
        Ok(slf)
    }

    /// Finalize and return a [`PyMuxerProgramConfig`]. The Rust
    /// builder uses `&self + clone`, so the same builder can produce
    /// multiple configs.
    pub fn build(slf: PyRef<'_, Self>) -> PyResult<PyMuxerProgramConfig> {
        let b = slf.inner.as_ref().ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err("MuxerProgramConfigBuilder is consumed")
        })?;
        Ok(PyMuxerProgramConfig { inner: b.build() })
    }
}

// ---------------------------------------------------------------------------
// MuxerConfig + MuxerConfigBuilder — Task 5.
// ---------------------------------------------------------------------------
//
// Wraps the outer half of the 4-type Rust config family: the
// top-level `MuxerConfig` (one or more programs + global cadence /
// buffer / AV1 carriage knobs) and its chainable builder. The
// builder's `build()` runs Rust-side `MuxerConfig::validate` — the
// returned `MuxError` is mapped through the 5-variant
// `MuxSenderErrorKind` classifier into a Python `MuxError` carrying
// the right `MuxErrorKind` (Task 1).

/// Frozen view of a built [`MuxerConfig`] — top-level muxer
/// configuration. Holds the program list plus PCR / PSI cadence,
/// the buffered-packet ceiling, and the AV1 PES carriage mode.
/// Construct via [`MuxerConfigBuilder`] or the static
/// [`MuxerConfig.builder()`][PyMuxerConfig::builder] shortcut.
#[pyclass(name = "MuxerConfig", module = "tstrans.mpegts", frozen)]
#[derive(Clone)]
pub struct PyMuxerConfig {
    pub(crate) inner: RustMuxerConfig,
}

#[pymethods]
impl PyMuxerConfig {
    /// Start a new [`MuxerConfigBuilder`]. Equivalent to
    /// `MuxerConfigBuilder()`; mirrors Rust's `MuxerConfig::builder`.
    #[staticmethod]
    pub fn builder() -> PyMuxerConfigBuilder {
        PyMuxerConfigBuilder {
            inner: Some(RustMuxerConfig::builder()),
        }
    }

    /// Tuple of [`MuxerProgramConfig`] entries — one per program in
    /// this multiplex, in add-order.
    #[getter]
    pub fn programs(&self, py: Python<'_>) -> PyResult<PyObject> {
        let items: Vec<PyObject> = self
            .inner
            .programs
            .iter()
            .map(|p| -> PyResult<PyObject> {
                let py_obj = PyMuxerProgramConfig { inner: p.clone() };
                let bound = Py::new(py, py_obj)?;
                Ok(bound.into_any())
            })
            .collect::<PyResult<Vec<_>>>()?;
        Ok(PyTuple::new_bound(py, items).unbind().into())
    }

    /// PCR re-emission interval in milliseconds (per program).
    #[getter]
    pub fn pcr_interval_ms(&self) -> u32 {
        self.inner.pcr_interval_ms
    }

    /// PAT/PMT re-emission interval in milliseconds.
    #[getter]
    pub fn psi_interval_ms(&self) -> u32 {
        self.inner.psi_interval_ms
    }

    /// Maximum buffered TS packets before push returns backpressure.
    #[getter]
    pub fn buffer_packets(&self) -> usize {
        self.inner.buffer_packets
    }

    /// AV1 PES carriage mode — `MPEG2_TS_BINDING` (default,
    /// spec-conformant) or `INTEROP_RAW_OBU` (ffmpeg-style).
    #[getter]
    pub fn av1_carriage<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        av1_carriage_to_py(py, self.inner.av1_carriage)
    }
}

/// Chainable builder for [`PyMuxerConfig`]. Append programs with
/// `add_program(...)` and override the global cadence / buffer /
/// AV1 carriage settings as needed; finalize with `build()`, which
/// runs Rust-side validation. Mirrors
/// `tst_core::mpegts::mux::MuxerConfigBuilder`.
///
/// The inner Rust builder lives in an `Option<...>` so `build()` can
/// take `&self` (the Rust API takes `&self` and clones internally).
#[pyclass(name = "MuxerConfigBuilder", module = "tstrans.mpegts")]
pub struct PyMuxerConfigBuilder {
    pub(crate) inner: Option<RustMuxerConfigBuilder>,
}

impl PyMuxerConfigBuilder {
    fn get_mut(&mut self) -> PyResult<&mut RustMuxerConfigBuilder> {
        self.inner.as_mut().ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err("MuxerConfigBuilder is consumed")
        })
    }
}

#[pymethods]
impl PyMuxerConfigBuilder {
    #[new]
    pub fn new() -> Self {
        Self {
            inner: Some(RustMuxerConfig::builder()),
        }
    }

    pub fn add_program<'py>(
        mut slf: PyRefMut<'py, Self>,
        prog: PyRef<'_, PyMuxerProgramConfig>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        slf.get_mut()?.add_program(prog.inner.clone());
        Ok(slf)
    }

    pub fn pcr_interval_ms(mut slf: PyRefMut<'_, Self>, ms: u32) -> PyResult<PyRefMut<'_, Self>> {
        slf.get_mut()?.pcr_interval_ms(ms);
        Ok(slf)
    }

    pub fn psi_interval_ms(mut slf: PyRefMut<'_, Self>, ms: u32) -> PyResult<PyRefMut<'_, Self>> {
        slf.get_mut()?.psi_interval_ms(ms);
        Ok(slf)
    }

    pub fn buffer_packets(mut slf: PyRefMut<'_, Self>, n: usize) -> PyResult<PyRefMut<'_, Self>> {
        slf.get_mut()?.buffer_packets(n);
        Ok(slf)
    }

    pub fn av1_carriage<'py>(
        mut slf: PyRefMut<'py, Self>,
        mode: &Bound<'_, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let rust_mode = py_av1_carriage(mode)?;
        slf.get_mut()?.av1_carriage(rust_mode);
        Ok(slf)
    }

    /// Finalize. Runs Rust-side `MuxerConfig::validate` and surfaces
    /// the first failed rule as a Python `MuxError` (kind chosen via
    /// `MuxSenderErrorKind` — typically `CONFIG_INVALID`).
    pub fn build(slf: PyRef<'_, Self>) -> PyResult<PyMuxerConfig> {
        let py = slf.py();
        let b = slf.inner.as_ref().ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err("MuxerConfigBuilder is consumed")
        })?;
        b.build()
            .map(|cfg| PyMuxerConfig { inner: cfg })
            .map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }
}

impl Default for PyMuxerConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Muxer — Task 6 (base: init + pull + pending_packets + capacity_packets).
// ---------------------------------------------------------------------------
//
// Wraps `tst_core::mpegts::mux::Muxer` — the stateful TS multiplexer.
// Tasks 7-9 will add the `push_*` and handle-getter surface; this task
// covers construction (which re-runs config validation Rust-side) and
// the drain side (`pull` + back-pressure gauges). `pull` is infallible
// per Rust — the only failure modes surface at `push_*` time.

/// Stateful MPEG-TS multiplexer. Configured at construction with a
/// [`MuxerConfig`]; subsequent `push_*` calls (Tasks 7-8) feed encoded
/// elementary streams, and `pull` drains the assembled TS packets.
///
/// `push_*` may return `MuxError(BACKPRESSURE)` when the internal
/// queue would exceed [`Muxer.capacity_packets`][PyMuxer::capacity_packets];
/// the caller must drain via `pull` before retrying.
#[pyclass(name = "Muxer", module = "tstrans.mpegts")]
pub struct PyMuxer {
    inner: RustMuxer,
}

#[pymethods]
impl PyMuxer {
    /// Construct from a built [`MuxerConfig`]. Re-runs Rust-side
    /// validation; any failure is surfaced as a Python `MuxError`
    /// (typically `CONFIG_INVALID`).
    #[new]
    pub fn new(py: Python<'_>, config: PyRef<'_, PyMuxerConfig>) -> PyResult<Self> {
        RustMuxer::new(config.inner.clone())
            .map(|m| Self { inner: m })
            .map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }

    /// Drain ready TS packets into `out`. Returns the number of bytes
    /// written: either 0 or a positive multiple of 188. A return of 0
    /// indicates either an empty queue or `len(out) < 188`.
    pub fn pull(&mut self, out: &Bound<'_, PyByteArray>) -> PyResult<usize> {
        // SAFETY: `as_bytes_mut` returns a mutable slice into the
        // Python bytearray for the duration of this call. The
        // underlying Rust `Muxer::pull` writes into the slice and
        // does not retain it. The GIL is held for the whole call, so
        // Python cannot mutate or resize the bytearray concurrently.
        let slice = unsafe { out.as_bytes_mut() };
        Ok(self.inner.pull(slice))
    }

    /// Number of 188-byte TS packets currently queued in the muxer's
    /// internal output buffer awaiting [`pull`][PyMuxer::pull].
    pub fn pending_packets(&self) -> u64 {
        self.inner.pending_packets()
    }

    /// Configured queue capacity in 188-byte TS packets — snapshot of
    /// `MuxerConfig.buffer_packets`. Push calls that would exceed this
    /// cap return `MuxError(BACKPRESSURE)`.
    pub fn capacity_packets(&self) -> u64 {
        self.inner.capacity_packets()
    }
}
