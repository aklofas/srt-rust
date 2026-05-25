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

use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PyDict, PyTuple};

use tst_core::mpegts::common::Pts90khz as RustPts90khz;
use tst_core::mpegts::mux::{
    AudioCodec as RustAudioCodec, AudioStreamHandle as RustAudioStreamHandle,
    Av1CarriageMode as RustAv1CarriageMode, KlvStreamHandle as RustKlvStreamHandle,
    KlvStreamType as RustKlvStreamType, Muxer as RustMuxer, MuxerConfig as RustMuxerConfig,
    MuxerConfigBuilder as RustMuxerConfigBuilder, MuxerProgramConfig as RustMuxerProgramConfig,
    MuxerProgramConfigBuilder as RustMuxerProgramConfigBuilder, MuxerStats as RustMuxerStats,
    StreamSpec as RustStreamSpec, SubtitleCodec as RustSubtitleCodec,
    SubtitleStreamHandle as RustSubtitleStreamHandle, VideoCodec as RustVideoCodec,
    VideoStreamHandle as RustVideoStreamHandle,
};
use tst_core::mpegts::stats::StreamCodecStats as RustStreamCodecStats;

/// Translate a Python `Pts90khz` dataclass instance to the Rust
/// `Pts90khz` newtype.
///
/// The Python class is a pure-Python `@dataclass(frozen=True)` with
/// a single `raw: int` field (see `python/tstrans/mpegts.py`), so we
/// extract the attribute via the Python protocol rather than holding
/// a `PyRef` to a PyO3 class. Construction is `Pts90khz::new(i64)`
/// per `tst_core::mpegts::common`.
pub(crate) fn py_pts90khz(v: &Bound<'_, PyAny>) -> PyResult<RustPts90khz> {
    let py = v.py();
    let raw: i64 = v.getattr(intern!(py, "raw"))?.extract()?;
    Ok(RustPts90khz::new(raw))
}

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
    /// Reconstruct a `VideoStreamHandle` from a raw `u32`.
    ///
    /// Validates the canonical bit layout (low 8 bits only) and rejects
    /// any forged value with high bits set — the audit caveat is that a
    /// forged `valid.raw() | 0x100` would otherwise mask down to the
    /// valid low byte and route the push to the wrong elementary stream.
    /// Raises `tstrans.exceptions.MuxError(INVALID_USAGE)` on rejection.
    #[staticmethod]
    pub fn from_raw(py: Python<'_>, raw: u32) -> PyResult<Self> {
        match RustVideoStreamHandle::try_from_raw(raw) {
            Ok(h) => Ok(Self(h)),
            Err(e) => Err(crate::errors::mux_error_to_pyerr(py, e)),
        }
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
    /// Reconstruct an `AudioStreamHandle` from a raw `u32`. Same
    /// validation contract as `VideoStreamHandle.from_raw`: raises
    /// `MuxError(INVALID_USAGE)` if the input has high bits set
    /// outside the canonical 8-bit packed layout.
    #[staticmethod]
    pub fn from_raw(py: Python<'_>, raw: u32) -> PyResult<Self> {
        match RustAudioStreamHandle::try_from_raw(raw) {
            Ok(h) => Ok(Self(h)),
            Err(e) => Err(crate::errors::mux_error_to_pyerr(py, e)),
        }
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
    /// Reconstruct a `KlvStreamHandle` from a raw `u32`. Same
    /// validation contract as `VideoStreamHandle.from_raw`: raises
    /// `MuxError(INVALID_USAGE)` if the input has high bits set
    /// outside the canonical 8-bit packed layout.
    #[staticmethod]
    pub fn from_raw(py: Python<'_>, raw: u32) -> PyResult<Self> {
        match RustKlvStreamHandle::try_from_raw(raw) {
            Ok(h) => Ok(Self(h)),
            Err(e) => Err(crate::errors::mux_error_to_pyerr(py, e)),
        }
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
    /// Reconstruct a `SubtitleStreamHandle` from a raw `u32`. Same
    /// validation contract as `VideoStreamHandle.from_raw`: raises
    /// `MuxError(INVALID_USAGE)` if the input has high bits set
    /// outside the canonical 8-bit packed layout.
    #[staticmethod]
    pub fn from_raw(py: Python<'_>, raw: u32) -> PyResult<Self> {
        match RustSubtitleStreamHandle::try_from_raw(raw) {
            Ok(h) => Ok(Self(h)),
            Err(e) => Err(crate::errors::mux_error_to_pyerr(py, e)),
        }
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
// flat Python `SubtitleCodec` enum (variant tag only) for the streams
// listing. Construction of mux-side subtitles from Python uses the
// `SubtitleCodecConfig` dataclass family in `tstrans.mpegts` and the
// `py_subtitle_codec` converter below (closeout audit finding 3).

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

/// Translate a Python `SubtitleCodecConfig` instance (one of
/// `DvbSubtitlingConfig` / `DvbTeletextConfig` / `Cea708StandaloneConfig` /
/// `WebVttInTsConfig` from `tstrans.mpegts`) to the Rust
/// struct-variant `SubtitleCodec`.
///
/// Dispatches on the Python class name (`type(v).__name__`) rather than
/// importing each class via `isinstance` — same approach as the existing
/// enum converters. Per-field range validation is enforced by the
/// dataclass `__post_init__` on the Python side; this converter only
/// extracts already-validated values.
fn py_subtitle_codec(v: &Bound<'_, PyAny>) -> PyResult<RustSubtitleCodec> {
    let py = v.py();
    let cls_name: String = v.get_type().getattr(intern!(py, "__name__"))?.extract()?;
    match cls_name.as_str() {
        "DvbSubtitlingConfig" => {
            let language_bytes: &[u8] =
                &v.getattr(intern!(py, "language"))?.extract::<Vec<u8>>()?;
            if language_bytes.len() != 3 {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "DvbSubtitlingConfig.language must be 3 bytes; got {}",
                    language_bytes.len()
                )));
            }
            let mut language = [0u8; 3];
            language.copy_from_slice(language_bytes);
            let subtitling_type: u8 = v.getattr(intern!(py, "subtitling_type"))?.extract()?;
            let composition_page_id: u16 =
                v.getattr(intern!(py, "composition_page_id"))?.extract()?;
            let ancillary_page_id: u16 = v.getattr(intern!(py, "ancillary_page_id"))?.extract()?;
            Ok(RustSubtitleCodec::DvbSubtitling {
                language,
                subtitling_type,
                composition_page_id,
                ancillary_page_id,
            })
        }
        "DvbTeletextConfig" => {
            let language_bytes: &[u8] =
                &v.getattr(intern!(py, "language"))?.extract::<Vec<u8>>()?;
            if language_bytes.len() != 3 {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "DvbTeletextConfig.language must be 3 bytes; got {}",
                    language_bytes.len()
                )));
            }
            let mut language = [0u8; 3];
            language.copy_from_slice(language_bytes);
            let teletext_type: u8 = v.getattr(intern!(py, "teletext_type"))?.extract()?;
            let magazine_number: u8 = v.getattr(intern!(py, "magazine_number"))?.extract()?;
            let page_number: u8 = v.getattr(intern!(py, "page_number"))?.extract()?;
            Ok(RustSubtitleCodec::DvbTeletext {
                language,
                teletext_type,
                magazine_number,
                page_number,
            })
        }
        "Cea708StandaloneConfig" => Ok(RustSubtitleCodec::Cea708Standalone),
        "WebVttInTsConfig" => Ok(RustSubtitleCodec::WebVttInTs),
        other => Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "expected one of (DvbSubtitlingConfig, DvbTeletextConfig, \
             Cea708StandaloneConfig, WebVttInTsConfig) from tstrans.mpegts; \
             got {other}"
        ))),
    }
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

    /// Append a subtitle / caption elementary stream to this program.
    ///
    /// `codec_config` must be one of the `SubtitleCodecConfig` dataclasses
    /// from `tstrans.mpegts`: `DvbSubtitlingConfig`, `DvbTeletextConfig`,
    /// `Cea708StandaloneConfig`, or `WebVttInTsConfig`. The Python
    /// dataclass carries the per-codec parameters (language, page IDs,
    /// etc.) — see each class's docstring for the field ranges.
    ///
    /// Returns `self` for fluent chaining. Mirrors Rust
    /// `MuxerProgramConfigBuilder::add_subtitle` (closeout audit
    /// finding 3 — the previously-deferred construction surface).
    pub fn add_subtitle<'py>(
        mut slf: PyRefMut<'py, Self>,
        pid: u16,
        codec_config: &Bound<'_, PyAny>,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let rust_codec = py_subtitle_codec(codec_config)?;
        slf.get_mut()?.add_subtitle(pid, rust_codec);
        Ok(slf)
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
        //
        // Audit #11 / GIL-release decision: `pull` is NOT wrapped in
        // `py.allow_threads` even though the rest of the `push_*`
        // family is. Rationale: `PyByteArray` is mutable + resizable
        // from any Python thread that holds a reference, so releasing
        // the GIL while holding `&mut [u8]` into it would violate
        // Rust's aliasing rules if a racing thread resized or wrote
        // the bytearray. The single-threaded `Muxer.pull(buf)` pattern
        // is the conventional use, but the safety contract here rests
        // on the GIL — unlike `push_*`, which borrows from immutable
        // `Py<PyBytes>`. `pull` is also fast (memcpy-class) so the
        // ergonomic loss is minimal.
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

    /// Push one H.264 / H.265 access unit in Annex-B framing onto the
    /// lone configured video stream.
    ///
    /// Convenience for single-video-stream muxers — raises
    /// `MuxError(INVALID_USAGE)` (mapped from `MuxError::AmbiguousTarget`)
    /// if zero or more than one video stream is configured across all
    /// programs; use [`push_video_to`][PyMuxer::push_video_to] with an
    /// explicit handle in that case.
    ///
    /// `key_frame=True` causes the first TS packet of the resulting
    /// PES to carry an adaptation field with `random_access_indicator`
    /// set; key-frame coincident with the PCR PID also forces a PCR.
    ///
    /// Raises `MuxError(INPUT_MALFORMED)` if `nal` does not begin with
    /// an Annex-B start code; `MuxError(BACKPRESSURE)` if the queue
    /// would exceed `MuxerConfig.buffer_packets`.
    ///
    /// `pts` is keyword-only — pass as `pts=...` (audit #9 normalization
    /// across all `push_*` methods).
    #[pyo3(signature = (nal, *, pts, key_frame = false))]
    pub fn push_video(
        &mut self,
        py: Python<'_>,
        nal: &[u8],
        pts: &Bound<'_, PyAny>,
        key_frame: bool,
    ) -> PyResult<()> {
        let rust_pts = py_pts90khz(pts)?;
        // GIL-release rationale (audit #11): `nal` borrows from a
        // `Py<PyBytes>` held by the caller's frame; safe to access
        // without the GIL because GC cannot collect a referenced
        // object during the call. The Rust `push_video` is pure
        // computation — no Python object construction.
        let res = py.allow_threads(|| self.inner.push_video(nal, rust_pts, key_frame));
        res.map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }

    /// Push one access unit onto a specific video stream identified
    /// by `handle` (obtained from `Muxer.video_handles()` in Task 9).
    ///
    /// Carries the same `key_frame` semantics as
    /// [`push_video`][PyMuxer::push_video]. AV1 streams receive OBU
    /// bitstream input and skip the Annex-B start-code check;
    /// H.264 / H.265 / H.266 require Annex-B framing.
    ///
    /// The PES carries `PTS_DTS_flags = '10'` (PTS only) per
    /// ISO/IEC 13818-1 §2.4.3.6. Streams with B-frame reorder where
    /// `composition_time != decode_time` must use
    /// [`push_video_to_with_dts`][PyMuxer::push_video_to_with_dts]
    /// instead.
    ///
    /// Raises `MuxError(INVALID_USAGE)` on an out-of-range handle,
    /// `MuxError(INPUT_MALFORMED)` on a bad Annex-B payload, or
    /// `MuxError(BACKPRESSURE)` on a full queue.
    ///
    /// `pts` is keyword-only — pass as `pts=...` (audit #9 normalization
    /// across all `push_*` methods).
    #[pyo3(signature = (handle, nal, *, pts, key_frame = false))]
    pub fn push_video_to(
        &mut self,
        py: Python<'_>,
        handle: PyRef<'_, PyVideoStreamHandle>,
        nal: &[u8],
        pts: &Bound<'_, PyAny>,
        key_frame: bool,
    ) -> PyResult<()> {
        let rust_pts = py_pts90khz(pts)?;
        // GIL-release rationale (audit #11): see `push_video`. `handle.0`
        // is a `Copy` u32 newtype; capturing it does not retain the
        // `PyRef`. `nal` is GIL-safe per the `push_video` argument.
        let handle_inner = handle.0;
        let res = py.allow_threads(|| {
            self.inner
                .push_video_to(handle_inner, nal, rust_pts, key_frame)
        });
        res.map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }

    /// Push one access unit with explicit composition (PTS) and decode
    /// (DTS) timestamps. Required for codecs that emit reordered output
    /// (B-frames in H.264 / H.265 / H.266 / AV1).
    ///
    /// Emits PES with `PTS_DTS_flags = '11'` per ISO/IEC 13818-1
    /// §2.4.3.6 — 10 bytes of PES header data carrying both PTS
    /// (composition time) and DTS (decode time). When `pts == dts`,
    /// prefer [`push_video_to`][PyMuxer::push_video_to] for the smaller
    /// 5-byte PTS-only encoding.
    ///
    /// Caller invariant: `dts <= pts` per §2.4.3.6 (decode order
    /// precedes composition order). The muxer does not enforce this —
    /// receivers will reject inverted timestamps.
    ///
    /// Internal cadence (PCR pacing, PSI emission, buffer reservation)
    /// keys off `pts`. DTS does not influence wall-clock scheduling.
    ///
    /// Error mapping matches `push_video_to`.
    #[pyo3(signature = (handle, nal, *, pts, dts, key_frame = false))]
    pub fn push_video_to_with_dts(
        &mut self,
        py: Python<'_>,
        handle: PyRef<'_, PyVideoStreamHandle>,
        nal: &[u8],
        pts: &Bound<'_, PyAny>,
        dts: &Bound<'_, PyAny>,
        key_frame: bool,
    ) -> PyResult<()> {
        let rust_pts = py_pts90khz(pts)?;
        let rust_dts = py_pts90khz(dts)?;
        // GIL-release rationale (audit #11): see `push_video`. `handle.0`
        // copied out before release; `nal` is GIL-safe.
        let handle_inner = handle.0;
        let res = py.allow_threads(|| {
            self.inner
                .push_video_to_with_dts(handle_inner, nal, rust_pts, rust_dts, key_frame)
        });
        res.map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }

    // -----------------------------------------------------------------
    // Task 8 — push_audio + push_klv + push_subtitle (single + handle).
    // -----------------------------------------------------------------
    //
    // Audit #9 (2026-05-24) normalized the Python `push_*` surface:
    // `pts` (and `dts` where applicable) is keyword-only on every
    // method, and `push_audio_to` takes `frames` positionally BEFORE
    // its kw-only `pts` so the `_to` variant mirrors the single-stream
    // variant's `(frames, *, pts)` shape rather than Rust's internally
    // inconsistent `(handle, pts, frames)`. The Rust API is unchanged;
    // the inconsistency was in the Python surface only.

    /// Push one encoded audio frame (codec-native framing — ADTS for
    /// AAC, raw frame for MP2 / AC-3 / AAC-LATM) onto the lone
    /// configured audio stream.
    ///
    /// Convenience for single-audio-stream muxers — raises
    /// `MuxError(INVALID_USAGE)` if zero or more than one audio stream
    /// is configured; use [`push_audio_to`][PyMuxer::push_audio_to] with
    /// an explicit handle in that case.
    ///
    /// Raises `MuxError(INPUT_MALFORMED)` if `frames` does not parse
    /// for the configured codec; `MuxError(BACKPRESSURE)` on a full
    /// queue.
    ///
    /// `pts` is keyword-only — pass as `pts=...` (audit #9 normalization
    /// across all `push_*` methods).
    #[pyo3(signature = (frames, *, pts))]
    pub fn push_audio(
        &mut self,
        py: Python<'_>,
        frames: &[u8],
        pts: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let rust_pts = py_pts90khz(pts)?;
        // GIL-release rationale (audit #11): see `push_video` — `frames`
        // borrows from a `Py<PyBytes>` held by the caller's frame.
        let res = py.allow_threads(|| self.inner.push_audio(frames, rust_pts));
        res.map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }

    /// Push one encoded audio frame onto a specific audio stream
    /// identified by `handle` (obtained from `Muxer.audio_handles()`
    /// in Task 9).
    ///
    /// Argument order: `(handle, frames, *, pts)` — `frames` is
    /// positional (matches the single-stream `push_audio(frames, *, pts)`
    /// shape) and `pts` is keyword-only. This intentionally diverges
    /// from the lower-level Rust `(handle, pts, frames)` order; the
    /// Python surface normalizes to a consistent `(target?, payload, *,
    /// pts)` shape across all `push_*` methods (audit #9).
    ///
    /// Raises `MuxError(INVALID_USAGE)` on an out-of-range handle,
    /// `MuxError(INPUT_MALFORMED)` on a codec parse failure, or
    /// `MuxError(BACKPRESSURE)` on a full queue.
    #[pyo3(signature = (handle, frames, *, pts))]
    pub fn push_audio_to(
        &mut self,
        py: Python<'_>,
        handle: PyRef<'_, PyAudioStreamHandle>,
        frames: &[u8],
        pts: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let rust_pts = py_pts90khz(pts)?;
        // GIL-release rationale (audit #11): see `push_video`.
        let handle_inner = handle.0;
        let res = py.allow_threads(|| self.inner.push_audio_to(handle_inner, rust_pts, frames));
        res.map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }

    /// Push one KLV local-set onto the lone configured KLV stream.
    ///
    /// `klv` is raw KLV LS bytes — for `SynchronousMetadata` streams
    /// the muxer auto-prepends the 5-byte `Metadata_AU_cell` header
    /// per ITU-T H.222.0 V9 §2.12.4.2; callers must NOT pre-wrap.
    /// `PrivateData` streams pass `klv` through as-is.
    ///
    /// `metadata_service_id` selects which service the metadata AU
    /// belongs to (defaults to 0, the common single-service case).
    ///
    /// Convenience for single-KLV-stream muxers — raises
    /// `MuxError(INVALID_USAGE)` if zero or more than one KLV stream is
    /// configured; use [`push_klv_to`][PyMuxer::push_klv_to] with an
    /// explicit handle in that case.
    ///
    /// Raises `MuxError(INPUT_MALFORMED)` if `klv` is too large for a
    /// single PES; `MuxError(BACKPRESSURE)` on a full queue.
    ///
    /// `pts` is keyword-only — pass as `pts=...` (audit #9 normalization
    /// across all `push_*` methods).
    #[pyo3(signature = (klv, *, pts, metadata_service_id = 0))]
    pub fn push_klv(
        &mut self,
        py: Python<'_>,
        klv: &[u8],
        pts: &Bound<'_, PyAny>,
        metadata_service_id: u8,
    ) -> PyResult<()> {
        let rust_pts = py_pts90khz(pts)?;
        // GIL-release rationale (audit #11): see `push_video` — `klv`
        // borrows from a `Py<PyBytes>` held by the caller's frame.
        let res = py.allow_threads(|| self.inner.push_klv(klv, rust_pts, metadata_service_id));
        res.map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }

    /// Push one KLV local-set onto a specific KLV stream identified by
    /// `handle` (obtained from `Muxer.klv_handles()` in Task 9).
    ///
    /// Same `klv` framing rules as [`push_klv`][PyMuxer::push_klv]:
    /// raw LS bytes; muxer auto-wraps the AU cell for synchronous
    /// streams. `metadata_service_id` defaults to 0.
    ///
    /// Raises `MuxError(INVALID_USAGE)` on an out-of-range handle,
    /// `MuxError(INPUT_MALFORMED)` on oversized payload, or
    /// `MuxError(BACKPRESSURE)` on a full queue.
    ///
    /// `pts` is keyword-only — pass as `pts=...` (audit #9 normalization
    /// across all `push_*` methods).
    #[pyo3(signature = (handle, klv, *, pts, metadata_service_id = 0))]
    pub fn push_klv_to(
        &mut self,
        py: Python<'_>,
        handle: PyRef<'_, PyKlvStreamHandle>,
        klv: &[u8],
        pts: &Bound<'_, PyAny>,
        metadata_service_id: u8,
    ) -> PyResult<()> {
        let rust_pts = py_pts90khz(pts)?;
        // GIL-release rationale (audit #11): see `push_video`.
        let handle_inner = handle.0;
        let res = py.allow_threads(|| {
            self.inner
                .push_klv_to(handle_inner, klv, rust_pts, metadata_service_id)
        });
        res.map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }

    /// Push one subtitle payload onto the lone configured subtitle
    /// stream. Argument order follows the Rust API: `(payload, *, pts)`.
    ///
    /// `pts` is keyword-only — pass as `pts=...` (audit #9 normalization
    /// across all `push_*` methods).
    ///
    /// Construct a configured subtitle stream via
    /// `MuxerProgramConfigBuilder.add_subtitle(pid, codec_config)`,
    /// passing one of the `SubtitleCodecConfig` dataclasses
    /// (`DvbSubtitlingConfig`, `DvbTeletextConfig`,
    /// `Cea708StandaloneConfig`, `WebVttInTsConfig`) from
    /// `tstrans.mpegts`.
    ///
    /// Raises `MuxError(INVALID_USAGE)` if zero or more than one
    /// subtitle stream is configured; `MuxError(INPUT_MALFORMED)` for
    /// oversized payloads; `MuxError(BACKPRESSURE)` on a full queue.
    #[pyo3(signature = (payload, *, pts))]
    pub fn push_subtitle(
        &mut self,
        py: Python<'_>,
        payload: &[u8],
        pts: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let rust_pts = py_pts90khz(pts)?;
        // GIL-release rationale (audit #11): see `push_video` — `payload`
        // borrows from a `Py<PyBytes>` held by the caller's frame.
        let res = py.allow_threads(|| self.inner.push_subtitle(rust_pts, payload));
        res.map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }

    /// Push one subtitle payload onto a specific subtitle stream
    /// identified by `handle` (obtained from
    /// `Muxer.subtitle_handles()`).
    ///
    /// Argument order: `(handle, payload, *, pts)` — `payload` is
    /// positional (mirrors `push_subtitle(payload, *, pts)`); `pts` is
    /// keyword-only.
    ///
    /// Raises `MuxError(INVALID_USAGE)` on an out-of-range handle,
    /// `MuxError(INPUT_MALFORMED)` on oversized payload, or
    /// `MuxError(BACKPRESSURE)` on a full queue.
    #[pyo3(signature = (handle, payload, *, pts))]
    pub fn push_subtitle_to(
        &mut self,
        py: Python<'_>,
        handle: PyRef<'_, PySubtitleStreamHandle>,
        payload: &[u8],
        pts: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let rust_pts = py_pts90khz(pts)?;
        // GIL-release rationale (audit #11): see `push_video`.
        let handle_inner = handle.0;
        let res = py.allow_threads(|| self.inner.push_subtitle_to(handle_inner, rust_pts, payload));
        res.map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }

    // -----------------------------------------------------------------
    // Task 9 — handle getters (video/audio/klv/subtitle × list +
    // by_program + by_index).
    // -----------------------------------------------------------------
    //
    // Rust surface (verified against tst-core/src/mpegts/mux/*.rs):
    //   video_*    — list + by_program + by_index
    //   audio_*    — list + by_program             (no by-index getter)
    //   klv_*      — list + by_program + by_index
    //   subtitle_* — list + by_program             (no by-index getter)
    //
    // `_for_program` returns `Result<Vec<_>, MuxError>` in Rust and
    // surfaces `ProgramNotFound` for an unknown program number; the
    // wraps below propagate that error through the standard
    // MuxSenderErrorKind classifier (Task 1) so callers see a Python
    // `MuxError(INVALID_USAGE)`. The list getters and the
    // by-index getters never fail — empty / `None` mean "no match".

    /// All [`VideoStreamHandle`]s across every program, in
    /// `(program_idx, stream_idx)` add-order.
    pub fn video_handles(&self) -> Vec<PyVideoStreamHandle> {
        self.inner
            .video_handles()
            .into_iter()
            .map(PyVideoStreamHandle)
            .collect()
    }

    /// All [`VideoStreamHandle`]s belonging to the given program number.
    /// Raises `MuxError(INVALID_USAGE)` if the program does not exist.
    pub fn video_handles_for_program(
        &self,
        py: Python<'_>,
        program_number: u16,
    ) -> PyResult<Vec<PyVideoStreamHandle>> {
        self.inner
            .video_handles_for_program(program_number)
            .map(|v| v.into_iter().map(PyVideoStreamHandle).collect())
            .map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }

    /// Single-program convenience: the `index`th video stream of the
    /// first program, or `None` if out-of-range. Mirrors Rust's
    /// `Muxer::video_stream_handle` (program 0 only).
    pub fn video_stream_handle(&self, index: usize) -> Option<PyVideoStreamHandle> {
        self.inner
            .video_stream_handle(index)
            .map(PyVideoStreamHandle)
    }

    /// All [`AudioStreamHandle`]s across every program, in
    /// `(program_idx, stream_idx)` add-order.
    pub fn audio_handles(&self) -> Vec<PyAudioStreamHandle> {
        self.inner
            .audio_handles()
            .into_iter()
            .map(PyAudioStreamHandle)
            .collect()
    }

    /// All [`AudioStreamHandle`]s belonging to the given program number.
    /// Raises `MuxError(INVALID_USAGE)` if the program does not exist.
    ///
    /// Note: there is no by-index audio-handle getter Rust-side; use
    /// `audio_handles()[idx]` for the single-program case.
    pub fn audio_handles_for_program(
        &self,
        py: Python<'_>,
        program_number: u16,
    ) -> PyResult<Vec<PyAudioStreamHandle>> {
        self.inner
            .audio_handles_for_program(program_number)
            .map(|v| v.into_iter().map(PyAudioStreamHandle).collect())
            .map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }

    /// All [`KlvStreamHandle`]s across every program, in
    /// `(program_idx, stream_idx)` add-order.
    pub fn klv_handles(&self) -> Vec<PyKlvStreamHandle> {
        self.inner
            .klv_handles()
            .into_iter()
            .map(PyKlvStreamHandle)
            .collect()
    }

    /// All [`KlvStreamHandle`]s belonging to the given program number.
    /// Raises `MuxError(INVALID_USAGE)` if the program does not exist.
    pub fn klv_handles_for_program(
        &self,
        py: Python<'_>,
        program_number: u16,
    ) -> PyResult<Vec<PyKlvStreamHandle>> {
        self.inner
            .klv_handles_for_program(program_number)
            .map(|v| v.into_iter().map(PyKlvStreamHandle).collect())
            .map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }

    /// Single-program convenience: the `index`th KLV stream of the
    /// first program, or `None` if out-of-range. Mirrors Rust's
    /// `Muxer::klv_stream_handle` (program 0 only).
    pub fn klv_stream_handle(&self, index: usize) -> Option<PyKlvStreamHandle> {
        self.inner.klv_stream_handle(index).map(PyKlvStreamHandle)
    }

    /// All [`SubtitleStreamHandle`]s across every program, in
    /// `(program_idx, stream_idx)` add-order.
    pub fn subtitle_handles(&self) -> Vec<PySubtitleStreamHandle> {
        self.inner
            .subtitle_handles()
            .into_iter()
            .map(PySubtitleStreamHandle)
            .collect()
    }

    /// All [`SubtitleStreamHandle`]s belonging to the given program
    /// number. Raises `MuxError(INVALID_USAGE)` if the program does not
    /// exist.
    ///
    /// Note: there is no by-index subtitle-handle getter Rust-side; use
    /// `subtitle_handles()[idx]` for the single-program case.
    pub fn subtitle_handles_for_program(
        &self,
        py: Python<'_>,
        program_number: u16,
    ) -> PyResult<Vec<PySubtitleStreamHandle>> {
        self.inner
            .subtitle_handles_for_program(program_number)
            .map(|v| v.into_iter().map(PySubtitleStreamHandle).collect())
            .map_err(|e| crate::errors::mux_error_to_pyerr(py, e))
    }

    // -----------------------------------------------------------------
    // Task 10 — stats accessors (stats + reset_stats + stream_codec_stats).
    // -----------------------------------------------------------------
    //
    // `stats()` returns a frozen `MuxerStats` snapshot (scalar counters
    // only — `per_stream` BTreeMap not surfaced in v1). `reset_stats`
    // zeros the cumulative counters in place (mirrors Rust contract:
    // per-stream identity preserved, codec counters cleared so a
    // previously-pushed PID reverts to `None` from Python's view).
    // `stream_codec_stats(pid)` constructs the right Python subclass
    // (`VideoStreamCodecStats` / `KlvStreamCodecStats` /
    // `AudioStreamCodecStats`) per the Rust enum variant; the
    // `Some(StreamCodecStats::Unknown)` Rust case (configured but no
    // data yet) collapses to Python `None` for caller simplicity.

    /// Snapshot the current muxer stats counters. Always succeeds.
    pub fn stats(&self) -> PyMuxerStats {
        PyMuxerStats {
            inner: self.inner.stats(),
        }
    }

    /// Zero all cumulative flow counters. Per-stream identity (PID,
    /// stream_type, label) is preserved; codec-specific counters are
    /// cleared, so a previously-pushed PID's `stream_codec_stats(pid)`
    /// returns `None` until the next push re-materializes the typed
    /// variant.
    pub fn reset_stats(&mut self) {
        self.inner.reset_stats();
    }

    /// Per-PID codec-specific counter snapshot. Returns `None` for
    /// PIDs the muxer was not configured with AND for configured PIDs
    /// that have no codec-family counters in v1 (PSI / subtitle / LATM
    /// / AC-3 — Rust returns `Some(StreamCodecStats::Unknown)` for
    /// those, which this wrap collapses to `None` so Python callers
    /// only see `None` vs a typed `*StreamCodecStats` subclass).
    pub fn stream_codec_stats(&self, py: Python<'_>, pid: u16) -> PyResult<Option<PyObject>> {
        let Some(rs) = self.inner.stream_codec_stats(pid) else {
            return Ok(None);
        };
        let mpegts_mod = py.import_bound("tstrans.mpegts")?;
        // Each variant uses `..` because the Rust enum *variants* are
        // `#[non_exhaustive]` (see tst_core::mpegts::stats) — additive
        // counter fields land without a major bump.
        let (cls_name, kwargs) = match rs {
            RustStreamCodecStats::Video {
                nals_or_obus,
                random_access_aus,
                ..
            } => {
                let kw = PyDict::new_bound(py);
                kw.set_item("nals_or_obus", nals_or_obus)?;
                kw.set_item("random_access_aus", random_access_aus)?;
                ("VideoStreamCodecStats", kw)
            }
            RustStreamCodecStats::Klv { records, .. } => {
                let kw = PyDict::new_bound(py);
                kw.set_item("records", records)?;
                ("KlvStreamCodecStats", kw)
            }
            RustStreamCodecStats::Audio { frames, .. } => {
                let kw = PyDict::new_bound(py);
                kw.set_item("frames", frames)?;
                ("AudioStreamCodecStats", kw)
            }
            // `Unknown` (configured PID without a counter family in v1)
            // and any future `#[non_exhaustive]` variant collapse to
            // `None` from Python's view.
            _ => return Ok(None),
        };
        let cls = mpegts_mod.getattr(cls_name)?;
        let obj = cls.call((), Some(&kwargs))?.unbind();
        Ok(Some(obj))
    }
}

// ---------------------------------------------------------------------------
// MuxerStats — Task 10.
// ---------------------------------------------------------------------------
//
// Frozen view of the Rust `MuxerStats` snapshot returned from
// `Muxer::stats`. Only the scalar fields are exposed in v1; the
// `per_stream` BTreeMap of `StreamStats` entries is not yet wrapped
// (no consumer demand — adding it later is additive).

/// Frozen snapshot of [`Muxer`] cumulative counters. Returned by
/// [`Muxer.stats`][PyMuxer::stats]. Reset to zero by
/// [`Muxer.reset_stats`][PyMuxer::reset_stats].
#[pyclass(name = "MuxerStats", module = "tstrans.mpegts", frozen)]
pub struct PyMuxerStats {
    inner: RustMuxerStats,
}

#[pymethods]
impl PyMuxerStats {
    /// Total 188-byte TS packets drained via [`Muxer.pull`][PyMuxer::pull].
    #[getter]
    pub fn ts_packets_emitted(&self) -> u64 {
        self.inner.ts_packets_emitted
    }

    /// Total bytes drained via [`Muxer.pull`][PyMuxer::pull]
    /// (`ts_packets_emitted * 188`).
    #[getter]
    pub fn ts_bytes_emitted(&self) -> u64 {
        self.inner.ts_bytes_emitted
    }

    /// Number of programs (PAT entries) in this muxer's configuration.
    #[getter]
    pub fn programs_configured(&self) -> u32 {
        self.inner.programs_configured
    }

    /// Number of subtitle streams configured across all programs.
    #[getter]
    pub fn subtitle_streams_configured(&self) -> u32 {
        self.inner.subtitle_streams_configured
    }

    fn __repr__(&self) -> String {
        format!(
            "MuxerStats(ts_packets_emitted={}, ts_bytes_emitted={}, programs_configured={}, subtitle_streams_configured={})",
            self.inner.ts_packets_emitted,
            self.inner.ts_bytes_emitted,
            self.inner.programs_configured,
            self.inner.subtitle_streams_configured,
        )
    }
}
