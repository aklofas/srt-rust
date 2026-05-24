//! PyO3 wrappers for `tst_core::mpegts::Demuxer` + `DemuxEvent`.
//!
//! Translation strategy: each Rust `DemuxEvent` variant is converted
//! to an instance of a Python-side subclass under
//! `tstrans.mpegts.DemuxEvent.*` via `convert_event(py, ...)`. Support
//! types (`StreamId`, `StreamInfo`, `ProgramMap`) are built from
//! Python-side dataclasses defined in `tstrans/mpegts.py`.
//!
//! Phase 2 ships: `Demuxer` PyClass + event conversion for all 6
//! Rust DemuxEvent variants. Sample.payload exposed as raw `bytes`;
//! typed NAL/OBU access lands in Phase 5.
//!
//! `#![allow(...)]` mirrors the pattern in `errors.rs` — PyO3 0.22 +
//! Rust 2024 macro expansions trip these lints. Hand-written code in
//! this module has no unsafe blocks.
#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use pyo3::Py;
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyTuple};

use tst_core::error::DemuxError;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::demux::event::{AudioCodec, MultiCellAuReason};
use tst_core::mpegts::demux::{
    DemuxEvent, Demuxer, DemuxerBuilder, DiscontinuityKind, LinkSource, MetadataKind,
    NonConformantIssue, ProgramMap, SamplePayload, StreamId, StreamInfo, StreamKind, StrictMode,
    SubtitleCodec, VideoCodec, VideoPayload,
};

use crate::errors::{codec_parse_error_to_pyerr, make_demux_error};

// ---------------------------------------------------------------------------
// PyDemuxer — the main wrapper
// ---------------------------------------------------------------------------

/// Python `Demuxer` — wraps `tst_core::mpegts::demux::Demuxer`.
///
/// Phase 2 surface: `feed(bytes)`, `flush()`, `next_event()`, iterator,
/// `stats()`, `reset_stats()`. Advanced knobs are exposed via
/// `DemuxerConfig` (Python-side dataclass) translated to Rust at
/// construction.
#[pyclass(name = "Demuxer", module = "tstrans.mpegts")]
pub struct PyDemuxer {
    inner: Demuxer,
}

#[pymethods]
impl PyDemuxer {
    /// Construct a Demuxer. `config` is a Python `DemuxerConfig`
    /// dataclass or `None` (defaults).
    #[new]
    #[pyo3(signature = (config = None))]
    fn new(py: Python<'_>, config: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        let inner = build_demuxer(py, config)?;
        Ok(Self { inner })
    }

    /// Feed a buffer of bytes. Accepts any bytes-like input that
    /// Python's `bytes()` constructor accepts: `bytes`, `bytearray`,
    /// `memoryview` (over either), and NumPy `uint8` arrays.
    ///
    /// Fast path: a `bytes` argument is borrowed via PyO3's `&[u8]`
    /// extractor with no extra copy. Fallback: any other bytes-like
    /// is coerced through the Python `bytes()` builtin (a single C
    /// copy into a fresh immutable `bytes` object) and then borrowed
    /// the same way. The copy is the price of accepting writable
    /// (`bytearray`) or non-contiguous (`memoryview` slice) producers
    /// safely under the GIL; the demuxer itself then parses without
    /// further copying.
    ///
    /// `PyBuffer` would let us skip the Python-side coercion, but it
    /// is gated behind `not(Py_LIMITED_API)` in PyO3 0.22, and
    /// `tst-py` builds with the `abi3-py310` stable-ABI feature so
    /// one wheel covers Python 3.10+.
    ///
    /// May produce events available via `next_event` / `__iter__`.
    /// Raises `tstrans.exceptions.DemuxError` in strict mode on
    /// non-conformance. Raises `TypeError` if the argument is not
    /// bytes-like (i.e. cannot be passed to `bytes()`).
    fn feed(&mut self, py: Python<'_>, bytes: &Bound<'_, PyAny>) -> PyResult<()> {
        // Fast path: real `bytes` extracts to a borrowed &[u8].
        //
        // GIL-release rationale (audit #11): the `&[u8]` borrows from a
        // `Py<PyBytes>` whose strong reference is held by the calling
        // Python frame for the duration of this call. Python's GC cannot
        // collect a referenced object, so the slice remains valid without
        // the GIL held. `feed` does pure-Rust parsing (no Python object
        // construction inside), so it is safe to wrap in `allow_threads`.
        if let Ok(slice) = bytes.extract::<&[u8]>() {
            let res = py.allow_threads(|| self.inner.feed(slice));
            return res.map_err(|e| demux_error_to_pyerr(py, e));
        }
        // Fallback: coerce via the Python `bytes()` builtin. Accepts
        // `bytearray`, `memoryview`, and any object exposing the
        // buffer protocol; raises `TypeError` if not bytes-like.
        let coerced: Bound<'_, PyBytes> = py
            .import_bound("builtins")?
            .getattr(intern!(py, "bytes"))?
            .call1((bytes,))?
            .downcast_into::<PyBytes>()?;
        // `coerced` is `!Ungil` (contains `Python<'_>`), but the
        // underlying `&[u8]` is — extract the slice first and let
        // `coerced` (the strong `Py<PyBytes>` reference) keep it alive
        // on the stack across the GIL drop. Python's GC cannot collect
        // a referenced object.
        let slice: &[u8] = coerced.as_bytes();
        let res = py.allow_threads(|| self.inner.feed(slice));
        res.map_err(|e| demux_error_to_pyerr(py, e))
    }

    /// Flush any in-flight PES reassembly. Call once at EOF before
    /// draining the iterator.
    fn flush(&mut self) {
        self.inner.flush();
    }

    /// Return the next available event, or `None` when the queue is
    /// empty.
    fn next_event(&mut self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        match self.inner.next_event() {
            None => Ok(None),
            Some(rust_event) => Ok(Some(convert_event(py, &rust_event)?)),
        }
    }

    /// Iterator protocol. `iter(demuxer)` returns `self`; `next()`
    /// either yields the next event or raises `StopIteration`.
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<PyObject> {
        match self.inner.next_event() {
            None => Err(pyo3::exceptions::PyStopIteration::new_err(())),
            Some(rust_event) => convert_event(py, &rust_event),
        }
    }

    /// Stats snapshot as a dict. Returns a flat `field_name: value`
    /// view of `DemuxerStats`.
    fn stats(&self, py: Python<'_>) -> PyResult<PyObject> {
        let s = self.inner.stats();
        let d = PyDict::new_bound(py);
        d.set_item("program_maps_seen", s.program_maps_seen)?;
        d.set_item("pmt_versions_seen", s.pmt_versions_seen)?;
        d.set_item("discontinuities", s.discontinuities)?;
        d.set_item("nonconformant", s.nonconformant)?;
        d.set_item("programs_seen", s.programs_seen)?;
        d.set_item("subtitle_streams_seen", s.subtitle_streams_seen)?;
        Ok(d.into())
    }

    /// Reset cumulative counters without dropping per-PID state.
    fn reset_stats(&mut self) {
        self.inner.reset_stats();
    }
}

// ---------------------------------------------------------------------------
// Config translation — Python DemuxerConfig dataclass → Rust DemuxerBuilder
// ---------------------------------------------------------------------------

/// Build a `Demuxer` from an optional Python `DemuxerConfig` dataclass.
fn build_demuxer(py: Python<'_>, config: Option<&Bound<'_, PyAny>>) -> PyResult<Demuxer> {
    let mut b = DemuxerBuilder::new();
    if let Some(cfg) = config {
        let strict_attr = cfg.getattr(intern!(py, "strict_mode"))?;
        let strict_name: String = strict_attr.getattr(intern!(py, "name"))?.extract()?;
        let strict = match strict_name.as_str() {
            "OFF" => StrictMode::Off,
            "TIMING_ONLY" => StrictMode::TimingOnly,
            // Python side uses "PSI_ONLY"; Rust renamed to DescriptorsOnly
            // (same semantics: hard-fail on descriptor/stream-type issues).
            "PSI_ONLY" => StrictMode::DescriptorsOnly,
            "FULL" => StrictMode::Full,
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown StrictMode variant: {other}"
                )));
            }
        };
        let cap_per_pid: usize = cfg.getattr(intern!(py, "pes_cap_per_pid"))?.extract()?;
        let cap_total: usize = cfg.getattr(intern!(py, "pes_cap_total"))?.extract()?;
        let cfi_tolerance: bool = cfg
            .getattr(intern!(py, "malformed_au_cell_cfi_tolerance"))?
            .extract()?;

        b = b.strict(strict);
        b = b.pes_cap_per_pid(cap_per_pid);
        b = b.pes_cap_total(cap_total);
        b = b.malformed_au_cell_cfi_tolerance(cfi_tolerance);
    }
    Ok(b.build())
}

// ---------------------------------------------------------------------------
// Event conversion: Rust DemuxEvent → Python DemuxEvent.* instance
// ---------------------------------------------------------------------------

fn convert_event(py: Python<'_>, ev: &DemuxEvent) -> PyResult<PyObject> {
    let mpegts = py.import_bound("tstrans.mpegts")?;
    match ev {
        DemuxEvent::ProgramMap(pm) => convert_program_map_event(py, &mpegts, pm),
        DemuxEvent::Sample {
            stream,
            pts,
            dts,
            payload,
        } => convert_sample_event(py, &mpegts, stream, *pts, *dts, payload),
        DemuxEvent::Metadata {
            stream,
            pts,
            kind,
            payload,
        } => convert_metadata_event(py, &mpegts, stream, *pts, kind, payload),
        DemuxEvent::Discontinuity { stream, kind } => {
            convert_discontinuity_event(py, &mpegts, stream, kind)
        }
        DemuxEvent::NonConformant { stream, issue } => {
            convert_non_conformant_event(py, &mpegts, stream, issue)
        }
        DemuxEvent::ReconnectDiscontinuity => {
            let cls = mpegts
                .getattr(intern!(py, "DemuxEvent"))?
                .getattr(intern!(py, "ReconnectDiscontinuity"))?;
            Ok(cls.call0()?.into())
        }
    }
}

fn convert_program_map_event(
    py: Python<'_>,
    mpegts: &Bound<'_, PyModule>,
    pm: &ProgramMap,
) -> PyResult<PyObject> {
    let pm_py = build_program_map(py, mpegts, pm)?;
    let cls = mpegts
        .getattr(intern!(py, "DemuxEvent"))?
        .getattr(intern!(py, "ProgramMap"))?;
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("programs", PyTuple::new_bound(py, &[pm_py]))?;
    Ok(cls.call((), Some(&kwargs))?.into())
}

fn build_program_map(
    py: Python<'_>,
    mpegts: &Bound<'_, PyModule>,
    pm: &ProgramMap,
) -> PyResult<PyObject> {
    let streams = PyList::empty_bound(py);
    for s in &pm.streams {
        streams.append(build_stream_info(py, mpegts, s)?)?;
    }
    let links = PyList::empty_bound(py);
    for l in &pm.klv_links {
        let link_cls = mpegts.getattr(intern!(py, "KlvLink"))?;
        let kwargs = PyDict::new_bound(py);
        kwargs.set_item("klv_pid", l.klv_pid)?;
        kwargs.set_item("video_pid", l.video_pid)?;
        kwargs.set_item("source", link_source_to_py(py, mpegts, &l.source)?)?;
        links.append(link_cls.call((), Some(&kwargs))?)?;
    }
    let cls = mpegts.getattr(intern!(py, "ProgramMap"))?;
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("program_number", pm.program_number)?;
    kwargs.set_item("pcr_pid", pm.pcr_pid)?;
    let streams_vec: Vec<_> = streams.iter().collect();
    let links_vec: Vec<_> = links.iter().collect();
    kwargs.set_item("streams", PyTuple::new_bound(py, &streams_vec))?;
    kwargs.set_item("klv_links", PyTuple::new_bound(py, &links_vec))?;
    Ok(cls.call((), Some(&kwargs))?.into())
}

fn build_stream_info(
    py: Python<'_>,
    mpegts: &Bound<'_, PyModule>,
    s: &StreamInfo,
) -> PyResult<PyObject> {
    let (kind_tag, codec_py) = stream_kind_to_py(py, mpegts, &s.kind)?;
    let cls = mpegts.getattr(intern!(py, "StreamInfo"))?;
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("pid", s.pid)?;
    kwargs.set_item("stream_type", s.stream_type.as_byte())?;
    kwargs.set_item("kind", kind_tag)?;
    kwargs.set_item("codec", codec_py)?;
    kwargs.set_item("program_number", s.program_number)?;
    Ok(cls.call((), Some(&kwargs))?.into())
}

fn build_stream_id(
    py: Python<'_>,
    mpegts: &Bound<'_, PyModule>,
    s: &StreamId,
) -> PyResult<PyObject> {
    let (kind_tag, codec_py) = stream_kind_to_py(py, mpegts, &s.kind)?;
    let cls = mpegts.getattr(intern!(py, "StreamId"))?;
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("pid", s.pid)?;
    kwargs.set_item("kind", kind_tag)?;
    kwargs.set_item("codec", codec_py)?;
    kwargs.set_item("program_number", s.program_number)?;
    Ok(cls.call((), Some(&kwargs))?.into())
}

fn stream_kind_to_py(
    py: Python<'_>,
    mpegts: &Bound<'_, PyModule>,
    kind: &StreamKind,
) -> PyResult<(PyObject, PyObject)> {
    let kind_enum = mpegts.getattr(intern!(py, "StreamKindTag"))?;
    let none = py.None();
    match kind {
        StreamKind::Video(c) => Ok((
            kind_enum.getattr(intern!(py, "VIDEO"))?.into(),
            video_codec_to_py(py, mpegts, c)?,
        )),
        StreamKind::Audio(c) => Ok((
            kind_enum.getattr(intern!(py, "AUDIO"))?.into(),
            audio_codec_to_py(py, mpegts, c)?,
        )),
        StreamKind::Subtitle(c) => Ok((
            kind_enum.getattr(intern!(py, "SUBTITLE"))?.into(),
            subtitle_codec_to_py(py, mpegts, c)?,
        )),
        StreamKind::KlvSync { .. } => {
            Ok((kind_enum.getattr(intern!(py, "KLV_SYNC"))?.into(), none))
        }
        StreamKind::KlvAsync => Ok((kind_enum.getattr(intern!(py, "KLV_ASYNC"))?.into(), none)),
        StreamKind::Unknown(_) => Ok((kind_enum.getattr(intern!(py, "UNKNOWN"))?.into(), none)),
    }
}

fn video_codec_to_py(
    py: Python<'_>,
    mpegts: &Bound<'_, PyModule>,
    c: &VideoCodec,
) -> PyResult<PyObject> {
    let e = mpegts.getattr(intern!(py, "VideoCodec"))?;
    let name = match c {
        VideoCodec::H264 => "H264",
        VideoCodec::H265 => "H265",
        VideoCodec::H266 => "H266",
        VideoCodec::Av1 => "AV1",
    };
    Ok(e.getattr(name)?.into())
}

fn audio_codec_to_py(
    py: Python<'_>,
    mpegts: &Bound<'_, PyModule>,
    c: &AudioCodec,
) -> PyResult<PyObject> {
    let e = mpegts.getattr(intern!(py, "AudioCodec"))?;
    let name = match c {
        AudioCodec::Mp2 => "MP2",
        AudioCodec::Aac => "AAC",
        AudioCodec::AacLatm => "AAC_LATM",
        AudioCodec::Ac3 => "AC3",
    };
    Ok(e.getattr(name)?.into())
}

fn subtitle_codec_to_py(
    py: Python<'_>,
    mpegts: &Bound<'_, PyModule>,
    c: &SubtitleCodec,
) -> PyResult<PyObject> {
    let e = mpegts.getattr(intern!(py, "SubtitleCodec"))?;
    let name = match c {
        SubtitleCodec::DvbSubtitling => "DVB_SUBTITLING",
        SubtitleCodec::DvbTeletext => "DVB_TELETEXT",
        SubtitleCodec::Cea708Standalone => "CEA708_STANDALONE",
        SubtitleCodec::WebVttInTs => "WEBVTT_IN_TS",
    };
    Ok(e.getattr(name)?.into())
}

fn link_source_to_py(
    py: Python<'_>,
    mpegts: &Bound<'_, PyModule>,
    src: &LinkSource,
) -> PyResult<PyObject> {
    let e = mpegts.getattr(intern!(py, "LinkSource"))?;
    let name = match src {
        LinkSource::Declared => "DECLARED",
        LinkSource::Inferred => "INFERRED",
        LinkSource::Override => "OVERRIDE",
    };
    Ok(e.getattr(name)?.into())
}

fn pts_to_py(py: Python<'_>, mpegts: &Bound<'_, PyModule>, p: Pts90khz) -> PyResult<PyObject> {
    let cls = mpegts.getattr(intern!(py, "Pts90khz"))?;
    Ok(cls
        .call_method1(intern!(py, "from_raw"), (p.as_ticks(),))?
        .into())
}

fn opt_pts_to_py(
    py: Python<'_>,
    mpegts: &Bound<'_, PyModule>,
    p: Option<Pts90khz>,
) -> PyResult<PyObject> {
    match p {
        Some(p) => pts_to_py(py, mpegts, p),
        None => Ok(py.None()),
    }
}

fn convert_sample_event(
    py: Python<'_>,
    mpegts: &Bound<'_, PyModule>,
    stream: &StreamId,
    pts: Pts90khz,
    dts: Option<Pts90khz>,
    payload: &SamplePayload,
) -> PyResult<PyObject> {
    let stream_py = build_stream_id(py, mpegts, stream)?;
    let pts_py = pts_to_py(py, mpegts, pts)?;
    let dts_py = opt_pts_to_py(py, mpegts, dts)?;
    let de = mpegts.getattr(intern!(py, "DemuxEvent"))?;
    match payload {
        SamplePayload::Video {
            codec,
            payload,
            random_access_indicator,
        } => {
            // Phase 5: emit typed list[NalUnit] | list[Obu] instead of raw bytes.
            let payload_py: PyObject = match payload {
                VideoPayload::Nals(nals) => {
                    let list = pyo3::types::PyList::empty_bound(py);
                    for nal in nals {
                        let nal_py = match nal {
                            tst_core::mpegts::demux::NalUnit::H264 {
                                nal_type,
                                ref_idc,
                                payload,
                            } => Py::new(
                                py,
                                crate::codec::NalUnitPy::make_h264(
                                    *nal_type,
                                    *ref_idc,
                                    payload.clone(),
                                ),
                            )?,
                            tst_core::mpegts::demux::NalUnit::H265 {
                                nal_type,
                                layer_id,
                                temporal_id_plus1,
                                payload,
                            } => Py::new(
                                py,
                                crate::codec::NalUnitPy::make_h265(
                                    *nal_type,
                                    *layer_id,
                                    *temporal_id_plus1,
                                    payload.clone(),
                                ),
                            )?,
                            tst_core::mpegts::demux::NalUnit::H266 {
                                nal_type,
                                layer_id,
                                temporal_id_plus1,
                                payload,
                            } => Py::new(
                                py,
                                crate::codec::NalUnitPy::make_h266(
                                    *nal_type,
                                    *layer_id,
                                    *temporal_id_plus1,
                                    payload.clone(),
                                ),
                            )?,
                        };
                        list.append(nal_py)?;
                    }
                    list.into_py(py)
                }
                VideoPayload::Obus(obus) => {
                    let list = pyo3::types::PyList::empty_bound(py);
                    for obu in obus {
                        let ext = obu.extension.map(|e| crate::codec::ObuExtensionPy {
                            temporal_id: e.temporal_id,
                            spatial_id: e.spatial_id,
                        });
                        let obu_py = Py::new(
                            py,
                            crate::codec::ObuPy::make(obu.obu_type, ext, obu.payload.clone()),
                        )?;
                        list.append(obu_py)?;
                    }
                    list.into_py(py)
                }
            };
            let cls = de.getattr(intern!(py, "Video"))?;
            let kwargs = PyDict::new_bound(py);
            kwargs.set_item("stream", stream_py)?;
            kwargs.set_item("pts", pts_py)?;
            kwargs.set_item("dts", dts_py)?;
            kwargs.set_item("codec", video_codec_to_py(py, mpegts, codec)?)?;
            kwargs.set_item("payload", payload_py)?;
            kwargs.set_item("random_access_indicator", *random_access_indicator)?;
            // codec_parse_error: None — video typed-parse cannot fail at this layer.
            kwargs.set_item("codec_parse_error", py.None())?;
            Ok(cls.call((), Some(&kwargs))?.into())
        }
        SamplePayload::Audio { codec, frames } => {
            // Phase 5: emit typed list[AdtsFrame] | list[Mpeg2AudioFrame],
            // or bytes-fallback + codec_parse_error on mid-stream parse failure (option c).
            use tst_core::codec::aac::frames_with_resync as aac_frames;
            use tst_core::codec::mpegaudio::frames_with_resync as mpegaudio_frames;
            // `frames` is a Vec<u8>; we need a &[u8] slice.
            let payload: &[u8] = frames;
            let (payload_py, parse_err): (PyObject, Option<PyErr>) = match codec {
                AudioCodec::Aac => {
                    let mut parsed: Vec<Py<crate::codec::AdtsFramePy>> = Vec::new();
                    let mut last_err = None;
                    for res in aac_frames(payload) {
                        match res {
                            Ok(f) => parsed.push(Py::new(
                                py,
                                crate::codec::AdtsFramePy {
                                    inner: f.to_owned(),
                                },
                            )?),
                            Err(e) => {
                                last_err = Some(e);
                                break;
                            }
                        }
                    }
                    if let Some(e) = last_err {
                        let err = codec_parse_error_to_pyerr(py, &e, "aac");
                        let bytes_py = PyBytes::new_bound(py, payload).into_py(py);
                        (bytes_py, Some(err))
                    } else {
                        let list = pyo3::types::PyList::empty_bound(py);
                        for f in parsed {
                            list.append(f)?;
                        }
                        (list.into_py(py), None)
                    }
                }
                AudioCodec::Mp2 => {
                    let mut parsed: Vec<Py<crate::codec::Mpeg2AudioFramePy>> = Vec::new();
                    let mut last_err = None;
                    for res in mpegaudio_frames(payload) {
                        match res {
                            Ok(f) => parsed.push(Py::new(
                                py,
                                crate::codec::Mpeg2AudioFramePy {
                                    inner: f.to_owned(),
                                },
                            )?),
                            Err(e) => {
                                last_err = Some(e);
                                break;
                            }
                        }
                    }
                    if let Some(e) = last_err {
                        let err = codec_parse_error_to_pyerr(py, &e, "mp2");
                        let bytes_py = PyBytes::new_bound(py, payload).into_py(py);
                        (bytes_py, Some(err))
                    } else {
                        let list = pyo3::types::PyList::empty_bound(py);
                        for f in parsed {
                            list.append(f)?;
                        }
                        (list.into_py(py), None)
                    }
                }
                // AAC-LATM typed parsing deferred — fall back to bytes silently.
                // AC-3 is not yet parsed — fall back to bytes silently.
                _ => {
                    let bytes_py = PyBytes::new_bound(py, payload).into_py(py);
                    (bytes_py, None)
                }
            };
            let cls = de.getattr(intern!(py, "Audio"))?;
            let kwargs = PyDict::new_bound(py);
            kwargs.set_item("stream", stream_py)?;
            kwargs.set_item("pts", pts_py)?;
            kwargs.set_item("dts", dts_py)?;
            kwargs.set_item("codec", audio_codec_to_py(py, mpegts, codec)?)?;
            kwargs.set_item("payload", payload_py)?;
            match parse_err {
                Some(err) => {
                    let val = err.value_bound(py).clone().into_any();
                    kwargs.set_item("codec_parse_error", val)?;
                }
                None => kwargs.set_item("codec_parse_error", py.None())?,
            }
            Ok(cls.call((), Some(&kwargs))?.into())
        }
        SamplePayload::Subtitle { codec, payload } => {
            let cls = de.getattr(intern!(py, "Subtitle"))?;
            let kwargs = PyDict::new_bound(py);
            kwargs.set_item("stream", stream_py)?;
            kwargs.set_item("pts", pts_py)?;
            kwargs.set_item("dts", dts_py)?;
            kwargs.set_item("codec", subtitle_codec_to_py(py, mpegts, codec)?)?;
            kwargs.set_item("payload", PyBytes::new_bound(py, payload))?;
            Ok(cls.call((), Some(&kwargs))?.into())
        }
        SamplePayload::Unknown {
            stream_type: _,
            raw,
        } => {
            // No typed Video/Audio/Subtitle subclass fits; surface as
            // NonConformant with an OTHER kind. Phase 5 (codec wrap)
            // may add typed Unknown support if needed.
            let nc_cls = de.getattr(intern!(py, "NonConformant"))?;
            let kind_enum = mpegts.getattr(intern!(py, "NonConformantKind"))?;
            let kwargs = PyDict::new_bound(py);
            kwargs.set_item("stream", stream_py)?;
            kwargs.set_item(
                "issue",
                format!(
                    "unknown stream_type sample (len={}) — Phase 5 codec wrap will add typed support",
                    raw.len()
                ),
            )?;
            kwargs.set_item("kind", kind_enum.getattr(intern!(py, "OTHER"))?)?;
            Ok(nc_cls.call((), Some(&kwargs))?.into())
        }
    }
}

fn convert_metadata_event(
    py: Python<'_>,
    mpegts: &Bound<'_, PyModule>,
    stream: &StreamId,
    pts: Pts90khz,
    kind: &MetadataKind,
    payload: &[u8],
) -> PyResult<PyObject> {
    let stream_py = build_stream_id(py, mpegts, stream)?;
    let pts_py = pts_to_py(py, mpegts, pts)?;
    let kind_enum = mpegts.getattr(intern!(py, "MetadataKindTag"))?;
    // Extract the new multi-cell reassembly fields when present.
    // Single-cell + non-KlvSyncAuCell paths default to (false, 1).
    let (kind_py, was_reassembled, cell_count) = match kind {
        MetadataKind::KlvSyncAuCell {
            was_reassembled,
            cell_count,
            ..
        } => (
            kind_enum.getattr(intern!(py, "KLV_SYNC_AU_CELL"))?,
            *was_reassembled,
            *cell_count,
        ),
        MetadataKind::KlvAsync => (kind_enum.getattr(intern!(py, "KLV_ASYNC"))?, false, 1u32),
        MetadataKind::Unknown(_) => (kind_enum.getattr(intern!(py, "UNKNOWN"))?, false, 1u32),
    };
    let cls = mpegts
        .getattr(intern!(py, "DemuxEvent"))?
        .getattr(intern!(py, "Klv"))?;
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("stream", stream_py)?;
    kwargs.set_item("pts", pts_py)?;
    kwargs.set_item("kind", kind_py)?;
    kwargs.set_item("payload", PyBytes::new_bound(py, payload))?;
    kwargs.set_item("was_reassembled", was_reassembled)?;
    kwargs.set_item("cell_count", cell_count)?;
    Ok(cls.call((), Some(&kwargs))?.into())
}

fn convert_discontinuity_event(
    py: Python<'_>,
    mpegts: &Bound<'_, PyModule>,
    stream: &StreamId,
    kind: &DiscontinuityKind,
) -> PyResult<PyObject> {
    let stream_py = build_stream_id(py, mpegts, stream)?;
    let kind_enum = mpegts.getattr(intern!(py, "DiscontinuityKindTag"))?;
    let kind_name = match kind {
        DiscontinuityKind::ContinuityJump { .. } => "CONTINUITY_JUMP",
        DiscontinuityKind::PesOversize { .. } => "PES_OVERSIZE",
        DiscontinuityKind::PesTotalOversize => "PES_TOTAL_OVERSIZE",
        DiscontinuityKind::AdaptationFieldFlag => "ADAPTATION_FIELD_FLAG",
    };
    let cls = mpegts
        .getattr(intern!(py, "DemuxEvent"))?
        .getattr(intern!(py, "Discontinuity"))?;
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("stream", stream_py)?;
    kwargs.set_item("kind", kind_enum.getattr(kind_name)?)?;
    Ok(cls.call((), Some(&kwargs))?.into())
}

fn convert_non_conformant_event(
    py: Python<'_>,
    mpegts: &Bound<'_, PyModule>,
    stream: &StreamId,
    issue: &NonConformantIssue,
) -> PyResult<PyObject> {
    let stream_py = build_stream_id(py, mpegts, stream)?;
    let kind_name = non_conformant_kind_name(issue);
    let issue_str = format!("{issue}");
    let kind_enum = mpegts.getattr(intern!(py, "NonConformantKind"))?;
    let cls = mpegts
        .getattr(intern!(py, "DemuxEvent"))?
        .getattr(intern!(py, "NonConformant"))?;
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("stream", stream_py)?;
    kwargs.set_item("issue", issue_str)?;
    kwargs.set_item("kind", kind_enum.getattr(kind_name)?)?;
    // Surface the typed multi-cell reason only on MultiCellAu issues; all
    // other issue kinds get None (Python-side default).
    let reason_py: PyObject = match issue {
        NonConformantIssue::MultiCellAu { reason, .. } => {
            Py::new(py, PyMultiCellAuReason::from(*reason))?.into_py(py)
        }
        _ => py.None(),
    };
    kwargs.set_item("multi_cell_au_reason", reason_py)?;

    // Surface the typed CFI bits only on MalformedAuCellCfiTolerated;
    // None on every other issue kind (Python-side default).
    let (observed_py, treated_py): (PyObject, PyObject) = match issue {
        NonConformantIssue::MalformedAuCellCfiTolerated {
            observed_cfi,
            treated_as,
            ..
        } => (
            Py::new(py, PyCellFragmentIndication::from(*observed_cfi))?.into_py(py),
            Py::new(py, PyCellFragmentIndication::from(*treated_as))?.into_py(py),
        ),
        _ => (py.None(), py.None()),
    };
    kwargs.set_item("observed_cfi", observed_py)?;
    kwargs.set_item("treated_as", treated_py)?;

    Ok(cls.call((), Some(&kwargs))?.into())
}

fn non_conformant_kind_name(issue: &NonConformantIssue) -> &'static str {
    use NonConformantIssue::*;
    match issue {
        StreamTypeMismatchSyncOnAsyncPid | StreamTypeMismatchAsyncOnSyncPid => {
            "STREAM_TYPE_MISMATCH"
        }
        MissingMetadataDescriptor => "MISSING_METADATA_DESCRIPTOR",
        PcrAnomaly { .. } => "PCR_ANOMALY",
        PsiChecksumMismatch { .. } => "PSI_CHECKSUM_MISMATCH",
        PusiMidPes => "PUSI_MID_PES",
        MalformedPes { .. } => "MALFORMED_PES",
        PidReusedAcrossPrograms { .. } => "PID_REUSED_ACROSS_PROGRAMS",
        SubtitleMissingDescriptor { .. } => "SUBTITLE_MISSING_DESCRIPTOR",
        SubtitleDescriptorAmbiguous { .. } => "SUBTITLE_DESCRIPTOR_AMBIGUOUS",
        SubtitleDescriptorMalformed { .. } => "SUBTITLE_DESCRIPTOR_MALFORMED",
        Av1RegistrationMalformed { .. } => "AV1_REGISTRATION_MALFORMED",
        Av1ObuMissingSizeField { .. } => "AV1_OBU_MISSING_SIZE_FIELD",
        Av1TileListNotAllowed { .. } => "AV1_TILE_LIST_NOT_ALLOWED",
        PsiOverlongSection { .. } => "PSI_OVERLONG_SECTION",
        TransportErrorPacket { .. } => "TRANSPORT_ERROR_PACKET",
        DvbSubDataIdentifier { .. } => "DVB_SUB_DATA_IDENTIFIER",
        PtsAnomaly { .. } => "PTS_ANOMALY",
        MissingRequiredPts { .. } => "MISSING_REQUIRED_PTS",
        PesHeaderMalformed { .. } => "PES_HEADER_MALFORMED",
        SubtitleAlignmentMissing { .. } => "SUBTITLE_ALIGNMENT_MISSING",
        PcrMalformed { .. } => "PCR_MALFORMED",
        NalHeader { .. } => "NAL_HEADER",
        Av1ObuHeader { .. } => "AV1_OBU_HEADER",
        LatmFraming { .. } => "LATM_FRAMING",
        PsiCcDiscontinuity { .. } => "PSI_CC_DISCONTINUITY",
        MultiCellAu { .. } => "MULTI_CELL_AU",
        MalformedAuCellCfiTolerated { .. } => "MALFORMED_AU_CELL_CFI_TOLERATED",
        PsiMultiSectionUnsupported { .. } => "PSI_MULTI_SECTION_UNSUPPORTED",
        Ac3SyncMissing { .. } => "AC3_SYNC_MISSING",
        Av1WrongStreamId { .. } => "AV1_WRONG_STREAM_ID",
        Av1MissingTsObuFraming { .. } => "AV1_MISSING_TS_OBU_FRAMING",
        Other(_) => "OTHER",
    }
}

// ---------------------------------------------------------------------------
// DemuxError → PyErr
// ---------------------------------------------------------------------------

fn demux_error_to_pyerr(py: Python<'_>, e: DemuxError) -> PyErr {
    // Map Rust DemuxError variants to Python DemuxErrorKind.
    let kind = match &e {
        DemuxError::Unrecoverable { .. } => "INTERNAL",
        DemuxError::StrictRejection(_) => "INTERNAL",
        DemuxError::MalformedPsi { .. } => "BAD_PMT",
        DemuxError::MalformedPes { .. } => "BAD_PES",
        DemuxError::SyncBufExhausted { .. } => "SYNC_LOSS",
        // DemuxError carries the non-exhaustive attribute; forward-compat catch-all.
        _ => "INTERNAL",
    };
    let msg = format!("{e}");
    make_demux_error(py, kind, &msg)
}

// ---------------------------------------------------------------------------
// MultiCellAuReason — Python eq_int enum mirroring Rust
// ---------------------------------------------------------------------------

/// Why a multi-cell AU reassembly attempt did not produce a `Sample`.
///
/// Mirrors `tst_core::mpegts::demux::event::MultiCellAuReason`. Surfaced on
/// `_NonConformantEvent.multi_cell_au_reason` when the underlying issue is
/// `MULTI_CELL_AU`. PyO3 `eq_int` enum — compare with `==`.
#[pyclass(eq, eq_int, name = "MultiCellAuReason", module = "tstrans.mpegts")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyMultiCellAuReason {
    /// Continuation cell (Middle/Last) arrived without a prior First.
    #[pyo3(name = "ORPHAN")]
    Orphan,
    /// Continuation cell's sequence_number did not match expected mod-256.
    #[pyo3(name = "SEQUENCE_GAP")]
    SequenceGap,
    /// A new First arrived while the previous AU was still buffering.
    #[pyo3(name = "CONCURRENT_FIRST")]
    ConcurrentFirst,
    /// Buffered AU exceeded `au_cell_cap_per_pid`.
    #[pyo3(name = "OVERFLOW")]
    Overflow,
}

impl From<MultiCellAuReason> for PyMultiCellAuReason {
    fn from(r: MultiCellAuReason) -> Self {
        match r {
            MultiCellAuReason::Orphan => Self::Orphan,
            MultiCellAuReason::SequenceGap => Self::SequenceGap,
            MultiCellAuReason::ConcurrentFirst => Self::ConcurrentFirst,
            MultiCellAuReason::Overflow => Self::Overflow,
            // Forward-compat for #[non_exhaustive] additions on the Rust side.
            // Map any future variant to Orphan as a safe-but-imprecise default;
            // future Python releases should extend the enum.
            _ => Self::Orphan,
        }
    }
}

// ---------------------------------------------------------------------------
// CellFragmentIndication — Python eq_int enum mirroring Rust
// ---------------------------------------------------------------------------

/// H.222.0 V9 §2.12.4.2 Table 2-157 `cell_fragment_indication` bits.
///
/// Mirrors `tst_core::mpegts::au_cell::CellFragmentIndication`. Surfaced on
/// `_NonConformantEvent.observed_cfi` and `_NonConformantEvent.treated_as`
/// when the underlying issue is `MALFORMED_AU_CELL_CFI_TOLERATED`. PyO3
/// `eq_int` enum — compare with `==`.
///
/// Discriminant values match the wire bits exactly: `MIDDLE=0`, `LAST=1`,
/// `FIRST=2`, `COMPLETE=3`.
#[pyclass(eq, eq_int, name = "CellFragmentIndication", module = "tstrans.mpegts")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PyCellFragmentIndication {
    /// `0b00` (0): middle cell of a multi-cell AU.
    #[pyo3(name = "MIDDLE")]
    Middle = 0,
    /// `0b01` (1): last cell of a multi-cell AU.
    #[pyo3(name = "LAST")]
    Last = 1,
    /// `0b10` (2): first cell of a multi-cell AU.
    #[pyo3(name = "FIRST")]
    First = 2,
    /// `0b11` (3): single cell carrying a complete AU.
    #[pyo3(name = "COMPLETE")]
    Complete = 3,
}

impl From<tst_core::mpegts::au_cell::CellFragmentIndication> for PyCellFragmentIndication {
    fn from(c: tst_core::mpegts::au_cell::CellFragmentIndication) -> Self {
        use tst_core::mpegts::au_cell::CellFragmentIndication;
        match c {
            CellFragmentIndication::Middle => Self::Middle,
            CellFragmentIndication::Last => Self::Last,
            CellFragmentIndication::First => Self::First,
            CellFragmentIndication::Complete => Self::Complete,
        }
    }
}

// ---------------------------------------------------------------------------
// PyModule registration
// ---------------------------------------------------------------------------

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyDemuxer>()?;
    m.add_class::<PyMultiCellAuReason>()?;
    m.add_class::<PyCellFragmentIndication>()?;
    Ok(())
}
