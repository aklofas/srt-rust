//! `ManagedMuxSender` + `ManagedDemuxReceiver` for SRT.
//!
//! Auto-reconnect convenience wrappers paralleling
//! [`crate::srt::mux_sender::PyMuxSender`] +
//! [`crate::srt::demux_receiver::PyDemuxReceiver`] one-for-one, but with
//! a [`tst_pipeline::reconnect::ManagedTransport`] (sender) or a
//! [`tst_pipeline::ManagedRecvTransport`] (receiver) underneath. URL +
//! socket config are captured at construction and replayed by the
//! reconnect factory on each `Broken`/`Closed` event from the inner SRT
//! socket.
//!
//! ## Why a new file rather than extending T5
//!
//! The inner type of the wrapped pipeline shell changes
//! (`SrtTransport` → `ManagedTransport<SrtTransport>` /
//! `ManagedRecvTransport<SrtTransport>`), which cascades into the field
//! type, every accessor, and the cancel-handle wiring. Sharing T5's code
//! via generics would force the PyClass methods to be generic too —
//! pyo3 doesn't support generic `#[pymethods]`. Copy + adjust is the
//! ergonomic shape.
//!
//! ## Reconnect-attempt counter
//!
//! Both wrappers expose `reconnect_attempts() -> int`. The receiver side
//! could in principle reuse `ManagedRecvTransport::reconnects_count` (a
//! SUCCESS counter); the sender side has no equivalent public accessor on
//! `ManagedTransport`. To keep the surface symmetric we instrument the
//! factory closure on BOTH sides with an `Arc<AtomicU64>` that bumps on
//! every factory CALL (attempt), then expose that counter unconditionally.
//! That gives the same semantics on both shells: a non-zero value means
//! the inner transport has been rebuilt (or attempted to rebuild) at
//! least once since construction.

#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use pyo3::Py;
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use tst_core::mpegts::demux::DemuxEvent;
use tst_core::transport::{TransportCancel, TransportError};
use tst_pipeline::{
    BoxedMuxSender, ManagedDemuxReceiver as RustManagedDemuxReceiver, ManagedDemuxReceiverConfig,
    ManagedRecvTransport, ManagedTransport, MuxSender as RustMuxSender, MuxSenderError,
    MuxSenderErrorSource,
};
use tst_srt::{Listener, ListenerConfig, Socket, SocketConfig, SrtTransport, SrtUrl, url::Mode};

use crate::errors::{make_demux_error, make_srt_error, mux_error_to_pyerr};
use crate::mux::{
    PyAudioStreamHandle, PyDataStreamHandle, PyKlvStreamHandle, PyMuxerProgramConfig, PyMuxerStats,
    PySubtitleStreamHandle, PyVideoStreamHandle, py_pts90khz,
};
use crate::srt::errors::{
    accept_error_to_pyerr, bind_error_to_pyerr, connect_error_to_pyerr, transport_error_to_pyerr,
    url_error_to_pyerr,
};
use crate::srt::policy::PyReconnectPolicy;
use crate::srt::transport::{PyCancelHandle, PySocketStats};

// ---------------------------------------------------------------------------
// Shared helpers (parallel to T5's `mux_sender.rs` / `demux_receiver.rs`)
// ---------------------------------------------------------------------------

/// Coerce a Python bytes-like argument to a `Bound<PyBytes>` whose
/// `.as_bytes()` borrow lives across a subsequent `py.allow_threads()`
/// call. Identical to T5's helper.
fn coerce_bytes_like<'py>(
    py: Python<'py>,
    arg: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyBytes>> {
    if let Ok(b) = arg.downcast::<PyBytes>() {
        return Ok(b.clone());
    }
    py.import_bound("builtins")?
        .getattr(intern!(py, "bytes"))?
        .call1((arg,))?
        .downcast_into::<PyBytes>()
        .map_err(|e| e.into())
}

/// Map a `MuxSenderError` to a Python exception. Mirror of T5's
/// `mux_sender_error_to_pyerr` — kept local rather than re-exported so
/// the call site can pick the SRT-specific transport-error helper.
fn mux_sender_error_to_pyerr(py: Python<'_>, e: MuxSenderError) -> PyErr {
    match e.source {
        MuxSenderErrorSource::Mux(mux_err) => mux_error_to_pyerr(py, mux_err),
        MuxSenderErrorSource::Transport(t) => transport_error_to_pyerr(py, t),
        _ => make_srt_error(py, "IO", &format!("{:?}", e.kind)),
    }
}

/// Map a `DemuxReceiverError` to the right Python exception. Mirror of
/// T5's helper in `demux_receiver.rs`.
fn demux_recv_error_to_pyerr(py: Python<'_>, e: tst_pipeline::DemuxReceiverError) -> PyErr {
    use tst_pipeline::DemuxReceiverErrorSource;
    match e.source {
        DemuxReceiverErrorSource::Transport(t) => transport_error_to_pyerr(py, t),
        DemuxReceiverErrorSource::Demux(d) => demux_error_to_pyerr(py, &d),
        _ => make_srt_error(py, "IO", &format!("{:?}", e.kind)),
    }
}

fn demux_error_to_pyerr(py: Python<'_>, e: &tst_core::error::DemuxError) -> PyErr {
    use tst_core::error::DemuxError;
    let kind = match e {
        DemuxError::Unrecoverable { .. } => "INTERNAL",
        DemuxError::StrictRejection(_) => "STRICT_REJECTION",
        DemuxError::MalformedPsi { .. } => "BAD_PMT",
        DemuxError::MalformedPes { .. } => "BAD_PES",
        DemuxError::SyncBufExhausted { .. } => "SYNC_LOSS",
        _ => "INTERNAL",
    };
    make_demux_error(py, kind, &format!("{e}"))
}

/// Brackets an IPv6 literal so it parses through `SocketAddr`. Mirror of
/// T5's helper.
fn join_host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Build a fresh `SrtTransport` connected as a caller. Mirror of
/// `tst-c`'s `crate::sender::connect::connect_srt` — re-implemented here
/// (`tst-c` is downstream of `tst-py`).
fn connect_srt(host: &str, port: u16, cfg: &SocketConfig) -> Result<SrtTransport, TransportError> {
    let mut cfg = cfg.clone();
    cfg.merge_sender_defaults();
    let addr = join_host_port(host, port);
    let socket = Socket::connect_with(&cfg, addr.as_str()).map_err(|e| TransportError::Broken {
        msg: format!("connect: {e}"),
        errno_code: None,
    })?;
    Ok(SrtTransport::new(socket))
}

/// Bind a listener + accept one peer; return the accepted `SrtTransport`.
/// Mirror of `tst-c`'s `crate::receiver::listen::listen_srt`.
fn listen_srt(host: &str, port: u16, cfg: &ListenerConfig) -> Result<SrtTransport, TransportError> {
    let bind_host = if host.is_empty() { "0.0.0.0" } else { host };
    let addr = if host.contains(':') && !host.starts_with('[') {
        format!("[{bind_host}]:{port}")
    } else {
        format!("{bind_host}:{port}")
    };
    let mut listener =
        Listener::bind_with(cfg, addr.as_str()).map_err(|e| TransportError::Broken {
            msg: format!("bind: {e}"),
            errno_code: None,
        })?;
    let (socket, _peer) = listener.accept().map_err(|e| TransportError::Broken {
        msg: format!("accept: {e}"),
        errno_code: None,
    })?;
    Ok(SrtTransport::new(socket))
}

// ---------------------------------------------------------------------------
// PyManagedMuxSender — wraps MuxSender<ManagedTransport<SrtTransport>>.
// ---------------------------------------------------------------------------

/// Single-call convenience wrapper that owns a `Muxer` plus a managed
/// SRT transport (auto-reconnect on `Broken`/`Closed`).
///
/// Construct via `ManagedMuxSender.from_url(url, program_config,
/// policy=ReconnectPolicy(...))`. URL must specify `?mode=caller` (the
/// default). When the underlying SRT socket drops, the wrapper rebuilds
/// it using the captured (URL, SocketConfig) — bytes accumulated during
/// the outage land in the gap buffer (sized by `ReconnectPolicy`).
///
/// All push methods accept any bytes-like input and release the GIL
/// while the muxer + transport work proceeds.
///
/// Use as a context manager for guaranteed cleanup:
/// ```python
/// from tstrans.srt import ManagedMuxSender, ReconnectPolicy
/// from tstrans.mpegts import MuxerProgramConfigBuilder, VideoCodec, Pts90khz
///
/// program = (
///     MuxerProgramConfigBuilder(1, 0x100)
///     .add_video(0x101, VideoCodec.H264)
///     .build()
/// )
/// with ManagedMuxSender.from_url(
///     "srt://127.0.0.1:7000?mode=caller", program, policy=ReconnectPolicy()
/// ) as s:
///     s.push_video(b"\x00\x00\x00\x01\x09\xf0", pts=Pts90khz.from_raw(0))
/// ```
#[pyclass(name = "ManagedMuxSender", module = "tstrans.srt")]
pub(crate) struct PyManagedMuxSender {
    /// `Option` so `close()` / `__exit__` can drop the inner shell while
    /// keeping the PyClass instance addressable for idempotent closes.
    inner: Option<RustMuxSender<ManagedTransport<SrtTransport>>>,
    /// Counts factory invocations (reconnect attempts). Bumped from
    /// inside the captured `Fn() -> Result<...>` closure by every
    /// `ManagedTransport::reconnect_and_drain` retry tick.
    factory_attempts: Arc<AtomicU64>,
}

#[pymethods]
impl PyManagedMuxSender {
    /// Build a `ManagedMuxSender` targeting `url` for the single-program
    /// configuration `program_config`. URL must specify `?mode=caller`.
    ///
    /// `policy` defaults to `ReconnectPolicy()` (matches Rust default —
    /// 10 attempts, exponential backoff 100ms..=10_000ms, gap buffer
    /// of 256 messages, drop-oldest overflow).
    ///
    /// Releases the GIL during the libsrt handshake. Raises
    /// `SrtError(CONFIG_INVALID)` on URL parse / bad-mode failure;
    /// `SrtError(CONNECT_FAILED)` / `SrtError(TIMEOUT)` on handshake
    /// failure; `MuxError(CONFIG_INVALID)` if the muxer construction
    /// rejects the program config.
    #[staticmethod]
    #[pyo3(signature = (url, program_config, *, policy = None))]
    fn from_url(
        py: Python<'_>,
        url: &str,
        program_config: PyRef<'_, PyMuxerProgramConfig>,
        policy: Option<PyReconnectPolicy>,
    ) -> PyResult<Self> {
        // 1. Build the muxer config from the single program.
        let mut cfg_builder = tst_core::mpegts::mux::MuxerConfig::builder();
        cfg_builder.add_program(program_config.inner.clone());
        let muxer_cfg = cfg_builder.build().map_err(|e| mux_error_to_pyerr(py, e))?;

        // 2. Parse the URL and enforce caller mode.
        let parsed = SrtUrl::parse(url).map_err(|e| url_error_to_pyerr(py, e))?;
        if parsed.mode != Mode::Caller {
            let msg = format!(
                "ManagedMuxSender.from_url requires ?mode=caller (default); got mode={:?}",
                parsed.mode
            );
            return Err(make_srt_error(py, "CONFIG_INVALID", &msg));
        }
        let mut sock_cfg = SocketConfig::default();
        parsed.overlay.apply_to_socket(&mut sock_cfg);

        // 3. Initial connect (GIL released for the libsrt handshake).
        let host = parsed.host.clone();
        let port = parsed.port;
        let initial = py
            .allow_threads(|| connect_srt(&host, port, &sock_cfg))
            .map_err(|e| {
                // Initial connect failure: convert TransportError::Broken
                // into a CONNECT_FAILED SrtError so callers can distinguish
                // it from runtime reconnect failures.
                let msg = match &e {
                    TransportError::Broken { msg, .. } => msg.clone(),
                    _ => format!("{e:?}"),
                };
                make_srt_error(py, "CONNECT_FAILED", &msg)
            })?;

        // 4. Build the reconnect factory. Capture host+port+cfg by value
        // so the closure outlives this scope. The factory is `Fn + Sync`
        // for `ManagedTransport::new` (send-side requires Sync).
        let attempts = Arc::new(AtomicU64::new(0));
        let attempts_for_factory = attempts.clone();
        let host_for_factory = parsed.host.clone();
        let port_for_factory = parsed.port;
        let cfg_for_factory = sock_cfg.clone();
        let factory = move || -> Result<SrtTransport, TransportError> {
            attempts_for_factory.fetch_add(1, Ordering::Release);
            connect_srt(&host_for_factory, port_for_factory, &cfg_for_factory)
        };

        // 5. Wrap initial in a ManagedTransport, then hand to MuxSender.
        let policy_inner = policy.map(|p| p.inner).unwrap_or_default();
        let managed = ManagedTransport::new(initial, factory, policy_inner);
        let sender =
            RustMuxSender::new(managed, muxer_cfg).map_err(|e| mux_error_to_pyerr(py, e))?;
        Ok(Self {
            inner: Some(sender),
            factory_attempts: attempts,
        })
    }

    // ── Push family — single-stream variants ──────────────────────────────

    /// Push one video access unit onto the lone configured video stream.
    /// Annex-B framing for H.264/H.265/H.266; raw OBU stream for AV1.
    #[pyo3(signature = (nal, *, pts, key_frame = false))]
    fn push_video(
        &self,
        py: Python<'_>,
        nal: &Bound<'_, PyAny>,
        pts: &Bound<'_, PyAny>,
        key_frame: bool,
    ) -> PyResult<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_srt_error(py, "CLOSED", "ManagedMuxSender is closed"))?;
        let rust_pts = py_pts90khz(pts)?;
        let coerced = coerce_bytes_like(py, nal)?;
        let slice = coerced.as_bytes();
        let res = py.allow_threads(|| inner.send_video(slice, rust_pts, key_frame));
        res.map_err(|e| mux_sender_error_to_pyerr(py, e))
    }

    /// Push one KLV blob onto the lone configured KLV stream.
    #[pyo3(signature = (klv, *, pts, metadata_service_id = 0))]
    fn push_klv(
        &self,
        py: Python<'_>,
        klv: &Bound<'_, PyAny>,
        pts: &Bound<'_, PyAny>,
        metadata_service_id: u8,
    ) -> PyResult<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_srt_error(py, "CLOSED", "ManagedMuxSender is closed"))?;
        let rust_pts = py_pts90khz(pts)?;
        let coerced = coerce_bytes_like(py, klv)?;
        let slice = coerced.as_bytes();
        let res = py.allow_threads(|| inner.send_klv(slice, rust_pts, metadata_service_id));
        res.map_err(|e| mux_sender_error_to_pyerr(py, e))
    }

    /// Push one encoded audio frame onto the lone configured audio stream.
    #[pyo3(signature = (adts, *, pts))]
    fn push_audio(
        &self,
        py: Python<'_>,
        adts: &Bound<'_, PyAny>,
        pts: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_srt_error(py, "CLOSED", "ManagedMuxSender is closed"))?;
        let rust_pts = py_pts90khz(pts)?;
        let coerced = coerce_bytes_like(py, adts)?;
        let slice = coerced.as_bytes();
        let res = py.allow_threads(|| inner.send_audio(slice, rust_pts));
        res.map_err(|e| mux_sender_error_to_pyerr(py, e))
    }

    /// Push one subtitle payload onto the lone configured subtitle stream.
    #[pyo3(signature = (payload, *, pts))]
    fn push_subtitle(
        &self,
        py: Python<'_>,
        payload: &Bound<'_, PyAny>,
        pts: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_srt_error(py, "CLOSED", "ManagedMuxSender is closed"))?;
        let rust_pts = py_pts90khz(pts)?;
        let coerced = coerce_bytes_like(py, payload)?;
        let slice = coerced.as_bytes();
        let res = py.allow_threads(|| inner.send_subtitle(slice, rust_pts));
        res.map_err(|e| mux_sender_error_to_pyerr(py, e))
    }

    /// Push one data payload onto the lone configured data stream.
    /// Pass-through: lands verbatim as one PES packet on stream_id 0xBD.
    #[pyo3(signature = (data, *, pts))]
    fn push_data(
        &self,
        py: Python<'_>,
        data: &Bound<'_, PyAny>,
        pts: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_srt_error(py, "CLOSED", "ManagedMuxSender is closed"))?;
        let rust_pts = py_pts90khz(pts)?;
        let coerced = coerce_bytes_like(py, data)?;
        let slice = coerced.as_bytes();
        let res = py.allow_threads(|| inner.send_data(slice, rust_pts));
        res.map_err(|e| mux_sender_error_to_pyerr(py, e))
    }

    // ── Push family — handle-targeted variants ────────────────────────────

    #[pyo3(signature = (handle, nal, *, pts, key_frame = false))]
    fn push_video_to(
        &self,
        py: Python<'_>,
        handle: PyRef<'_, PyVideoStreamHandle>,
        nal: &Bound<'_, PyAny>,
        pts: &Bound<'_, PyAny>,
        key_frame: bool,
    ) -> PyResult<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_srt_error(py, "CLOSED", "ManagedMuxSender is closed"))?;
        let rust_pts = py_pts90khz(pts)?;
        let handle_inner = handle.0;
        let coerced = coerce_bytes_like(py, nal)?;
        let slice = coerced.as_bytes();
        let res =
            py.allow_threads(|| inner.send_video_to(handle_inner, slice, rust_pts, key_frame));
        res.map_err(|e| mux_sender_error_to_pyerr(py, e))
    }

    #[pyo3(signature = (handle, klv, *, pts, metadata_service_id = 0))]
    fn push_klv_to(
        &self,
        py: Python<'_>,
        handle: PyRef<'_, PyKlvStreamHandle>,
        klv: &Bound<'_, PyAny>,
        pts: &Bound<'_, PyAny>,
        metadata_service_id: u8,
    ) -> PyResult<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_srt_error(py, "CLOSED", "ManagedMuxSender is closed"))?;
        let rust_pts = py_pts90khz(pts)?;
        let handle_inner = handle.0;
        let coerced = coerce_bytes_like(py, klv)?;
        let slice = coerced.as_bytes();
        let res = py.allow_threads(|| {
            inner.send_klv_to(handle_inner, slice, rust_pts, metadata_service_id)
        });
        res.map_err(|e| mux_sender_error_to_pyerr(py, e))
    }

    #[pyo3(signature = (handle, adts, *, pts))]
    fn push_audio_to(
        &self,
        py: Python<'_>,
        handle: PyRef<'_, PyAudioStreamHandle>,
        adts: &Bound<'_, PyAny>,
        pts: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_srt_error(py, "CLOSED", "ManagedMuxSender is closed"))?;
        let rust_pts = py_pts90khz(pts)?;
        let handle_inner = handle.0;
        let coerced = coerce_bytes_like(py, adts)?;
        let slice = coerced.as_bytes();
        let res = py.allow_threads(|| inner.send_audio_to(handle_inner, slice, rust_pts));
        res.map_err(|e| mux_sender_error_to_pyerr(py, e))
    }

    #[pyo3(signature = (handle, payload, *, pts))]
    fn push_subtitle_to(
        &self,
        py: Python<'_>,
        handle: PyRef<'_, PySubtitleStreamHandle>,
        payload: &Bound<'_, PyAny>,
        pts: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_srt_error(py, "CLOSED", "ManagedMuxSender is closed"))?;
        let rust_pts = py_pts90khz(pts)?;
        let handle_inner = handle.0;
        let coerced = coerce_bytes_like(py, payload)?;
        let slice = coerced.as_bytes();
        let res = py.allow_threads(|| inner.send_subtitle_to(handle_inner, slice, rust_pts));
        res.map_err(|e| mux_sender_error_to_pyerr(py, e))
    }

    #[pyo3(signature = (handle, data, *, pts))]
    fn push_data_to(
        &self,
        py: Python<'_>,
        handle: PyRef<'_, PyDataStreamHandle>,
        data: &Bound<'_, PyAny>,
        pts: &Bound<'_, PyAny>,
    ) -> PyResult<()> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_srt_error(py, "CLOSED", "ManagedMuxSender is closed"))?;
        let rust_pts = py_pts90khz(pts)?;
        let handle_inner = handle.0;
        let coerced = coerce_bytes_like(py, data)?;
        let slice = coerced.as_bytes();
        let res = py.allow_threads(|| inner.send_data_to(handle_inner, slice, rust_pts));
        res.map_err(|e| mux_sender_error_to_pyerr(py, e))
    }

    // ── Handle getters ────────────────────────────────────────────────────

    fn video_handle(&self) -> Option<PyVideoStreamHandle> {
        let inner = self.inner.as_ref()?;
        inner
            .video_handles()
            .into_iter()
            .next()
            .map(PyVideoStreamHandle)
    }

    fn klv_handle(&self) -> Option<PyKlvStreamHandle> {
        let inner = self.inner.as_ref()?;
        inner
            .klv_handles()
            .into_iter()
            .next()
            .map(PyKlvStreamHandle)
    }

    fn audio_handle(&self) -> Option<PyAudioStreamHandle> {
        let inner = self.inner.as_ref()?;
        inner
            .audio_handles()
            .into_iter()
            .next()
            .map(PyAudioStreamHandle)
    }

    fn subtitle_handle(&self) -> Option<PySubtitleStreamHandle> {
        let inner = self.inner.as_ref()?;
        inner
            .subtitle_handles()
            .into_iter()
            .next()
            .map(PySubtitleStreamHandle)
    }

    fn data_handle(&self) -> Option<PyDataStreamHandle> {
        let inner = self.inner.as_ref()?;
        inner
            .data_handles()
            .into_iter()
            .next()
            .map(PyDataStreamHandle)
    }

    // ── Stats ──────────────────────────────────────────────────────────────

    /// `(SocketStats, MuxerStats)` snapshot. Same shape as T5's
    /// `MuxSender.stats()`. `SocketStats` may report zeros while the
    /// transport is mid-reconnect (the inner socket is `None`).
    fn stats(&self, py: Python<'_>) -> PyResult<(Py<PySocketStats>, Py<PyMuxerStats>)> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_srt_error(py, "CLOSED", "ManagedMuxSender is closed"))?;
        let sock = inner.socket_stats().unwrap_or_default();
        let pipe = inner.stats();
        let mux_stats = tst_core::mpegts::mux::MuxerStats {
            ts_packets_emitted: pipe.packets_sent,
            ts_bytes_emitted: pipe.bytes_sent,
            programs_configured: pipe.programs_configured,
            subtitle_streams_configured: 0,
            per_stream: pipe.per_stream,
        };
        let sock_py = Py::new(py, PySocketStats::from_core(sock))?;
        let mux_py = Py::new(py, PyMuxerStats::from_inner(mux_stats))?;
        Ok((sock_py, mux_py))
    }

    /// Total number of times the reconnect factory has been invoked
    /// since construction. 0 means the initial connect is still live;
    /// rising values mean the inner SRT socket has been rebuilt (or a
    /// rebuild attempt failed and was retried).
    fn reconnect_attempts(&self) -> u64 {
        self.factory_attempts.load(Ordering::Acquire)
    }

    // ── Lifecycle ──────────────────────────────────────────────────────────

    /// Close the sender. Idempotent. Drops the underlying managed
    /// transport which in turn closes the inner SRT socket.
    fn close(&mut self) {
        if let Some(s) = self.inner.take() {
            s.close();
        }
    }

    fn is_alive(&self) -> bool {
        self.inner.as_ref().is_some_and(|s| s.is_alive())
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __exit__(
        &mut self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        self.close();
        false
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            Some(_) => format!(
                "ManagedMuxSender(open, reconnect_attempts={})",
                self.reconnect_attempts()
            ),
            None => "ManagedMuxSender(closed)".to_string(),
        }
    }
}

// `BoxedMuxSender` import kept for future cross-task helpers (e.g. when
// the lowlevel `Builder` learns to promote a managed transport). Unused
// today; silence the warning.
#[allow(dead_code)]
type _BoxedMuxSenderAlias = BoxedMuxSender;

// ---------------------------------------------------------------------------
// PyManagedDemuxReceiver — wraps ManagedDemuxReceiver<SrtTransport>.
// ---------------------------------------------------------------------------

/// Single-call convenience wrapper that owns a `ManagedDemuxReceiver` +
/// SRT transport with auto-reconnect.
///
/// Construct via `ManagedDemuxReceiver.from_url(url,
/// demux_config=DemuxerConfig(...), policy=ReconnectPolicy(...))`. URL
/// may specify `?mode=listener` (default for the receiver side) or
/// `?mode=caller`; in caller mode the wrapper will dial the configured
/// peer on each reconnect, in listener mode it re-binds and re-accepts.
///
/// On reconnect, the inner [`tst_pipeline::ManagedDemuxReceiver`] emits
/// a [`tstrans.mpegts.DemuxEvent.ReconnectDiscontinuity`] event before
/// any post-reconnect events. Consumers should drop per-stream caches on
/// receipt and rebuild from the next `ProgramMap` event.
///
/// Use as a context manager for guaranteed cleanup:
/// ```python
/// from tstrans.srt import ManagedDemuxReceiver, ReconnectPolicy
///
/// with ManagedDemuxReceiver.from_url(
///     "srt://:7000?mode=listener", policy=ReconnectPolicy()
/// ) as rx:
///     for event in rx:
///         match event:
///             case DemuxEvent.ReconnectDiscontinuity():
///                 cache = {}  # rebuild on next ProgramMap
///             case DemuxEvent.Sample(...): ...
/// ```
#[pyclass(name = "ManagedDemuxReceiver", module = "tstrans.srt")]
pub(crate) struct PyManagedDemuxReceiver {
    /// Live receiver behind a mutex so a concurrent `__next__` /
    /// `close()` from different Python threads serialise cleanly.
    /// `Option` so `close()` can take + drop the inner shell.
    inner: Arc<Mutex<Option<RustManagedDemuxReceiver<SrtTransport>>>>,
    /// Cancel handle pulled from the receiver at construction. Held
    /// outside the mutex so `close()` can fire it BEFORE acquiring the
    /// lock — wakes any thread parked in `__next__`'s `recv_event`,
    /// which then drops the mutex guard and the close path can take
    /// ownership of `inner` cleanly.
    cancel: Arc<dyn TransportCancel + Send + Sync>,
    /// Reconnect-attempt counter — bumped from inside the factory closure
    /// on every invocation. Symmetric with `PyManagedMuxSender`.
    factory_attempts: Arc<AtomicU64>,
}

#[pymethods]
impl PyManagedDemuxReceiver {
    /// Bind (or connect) a managed receiver to `url`.
    ///
    /// `demux_config` is an optional `tstrans.mpegts.DemuxerConfig`
    /// dataclass; defaults are used when `None`. `policy` defaults to
    /// `ReconnectPolicy()` (matches Rust default).
    ///
    /// Raises `SrtError(CONFIG_INVALID)` on URL parse failure;
    /// `SrtError(CONNECT_FAILED)` on bind / connect failure;
    /// `SrtError(ACCEPT_FAILED)` / `SrtError(TIMEOUT)` on accept failure.
    #[staticmethod]
    #[pyo3(signature = (url, *, demux_config = None, policy = None))]
    fn from_url(
        py: Python<'_>,
        url: &str,
        demux_config: Option<&Bound<'_, PyAny>>,
        policy: Option<PyReconnectPolicy>,
    ) -> PyResult<Self> {
        // 1. Parse URL.
        let parsed = SrtUrl::parse(url).map_err(|e| url_error_to_pyerr(py, e))?;
        let is_listener = parsed.mode == Mode::Listener;
        let mut listener_cfg = ListenerConfig::default();
        let mut sock_cfg = SocketConfig::default();
        if is_listener {
            parsed.overlay.apply_to_listener(&mut listener_cfg);
        } else {
            parsed.overlay.apply_to_socket(&mut sock_cfg);
        }
        let host = parsed.host.clone();
        let port = parsed.port;

        // 2. Translate DemuxerConfig dataclass (must happen with GIL held).
        let demux_opts = match demux_config {
            None => None,
            Some(cfg_obj) => Some(crate::mpegts::build_demuxer_config(py, cfg_obj)?),
        };

        // 3. Initial inner transport (GIL released).
        let host_for_initial = host.clone();
        let listener_cfg_for_initial = listener_cfg.clone();
        let sock_cfg_for_initial = sock_cfg.clone();
        let initial = py.allow_threads(move || -> Result<SrtTransport, BindOrConnect> {
            if is_listener {
                listen_srt(&host_for_initial, port, &listener_cfg_for_initial)
                    .map_err(BindOrConnect::ListenInitial)
            } else {
                connect_srt(&host_for_initial, port, &sock_cfg_for_initial)
                    .map_err(BindOrConnect::ConnectInitial)
            }
        });
        let initial = match initial {
            Ok(t) => t,
            Err(BindOrConnect::ListenInitial(e)) => {
                let msg = match &e {
                    TransportError::Broken { msg, .. } => msg.clone(),
                    _ => format!("{e:?}"),
                };
                return Err(make_srt_error(py, "CONNECT_FAILED", &msg));
            }
            Err(BindOrConnect::ConnectInitial(e)) => {
                let msg = match &e {
                    TransportError::Broken { msg, .. } => msg.clone(),
                    _ => format!("{e:?}"),
                };
                return Err(make_srt_error(py, "CONNECT_FAILED", &msg));
            }
        };

        // 4. Build the reconnect factory. `ManagedRecvTransport::new`
        // takes `Box<dyn FnMut() -> Result<R, TransportError> + Send>` —
        // no `Sync` bound (receive-side single-thread access pattern).
        let attempts = Arc::new(AtomicU64::new(0));
        let attempts_for_factory = attempts.clone();
        let host_for_factory = host.clone();
        let listener_cfg_for_factory = listener_cfg.clone();
        let sock_cfg_for_factory = sock_cfg.clone();
        let factory: Box<dyn FnMut() -> Result<SrtTransport, TransportError> + Send> =
            Box::new(move || {
                attempts_for_factory.fetch_add(1, Ordering::Release);
                if is_listener {
                    listen_srt(&host_for_factory, port, &listener_cfg_for_factory)
                } else {
                    connect_srt(&host_for_factory, port, &sock_cfg_for_factory)
                }
            });

        // 5. Wrap.
        let policy_inner = policy.map(|p| p.inner).unwrap_or_default();
        let managed = ManagedRecvTransport::new(initial, factory, policy_inner);
        let receiver = match demux_opts {
            None => RustManagedDemuxReceiver::new(managed, ManagedDemuxReceiverConfig::default()),
            Some(opts) => RustManagedDemuxReceiver::with_demux_options(
                managed,
                opts,
                ManagedDemuxReceiverConfig::default(),
            ),
        };
        // `ManagedDemuxReceiver::cancel_handle` may legitimately return
        // None if the inner is mid-reconnect at construction. With a
        // freshly-built inner that's not the case, but defend with a
        // typed error rather than `.expect`.
        let cancel = receiver.cancel_handle().ok_or_else(|| {
            make_srt_error(
                py,
                "IO",
                "ManagedDemuxReceiver constructed without a live cancel handle",
            )
        })?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Some(receiver))),
            cancel,
            factory_attempts: attempts,
        })
    }

    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Block until the next `DemuxEvent` is available. Emits
    /// `DemuxEvent.ReconnectDiscontinuity` once after each transport
    /// reconnect; consumers should drop per-stream caches on receipt
    /// and rebuild from the next `ProgramMap`.
    ///
    /// Raises `StopIteration` on clean EOF; `SrtError` on transport-side
    /// failure (reconnect budget exhausted); `DemuxError` on demuxer
    /// failure.
    fn __next__(&self, py: Python<'_>) -> PyResult<PyObject> {
        let inner = self.inner.clone();
        let res: Result<Option<DemuxEvent>, tst_pipeline::DemuxReceiverError> =
            py.allow_threads(|| {
                let mut guard = match inner.lock() {
                    Ok(g) => g,
                    Err(_) => {
                        return Err(tst_pipeline::DemuxReceiverError::from(
                            TransportError::Broken {
                                msg: "ManagedDemuxReceiver inner lock poisoned".into(),
                                errno_code: None,
                            },
                        ));
                    }
                };
                match guard.as_mut() {
                    Some(rx) => rx.recv_event(),
                    None => Err(tst_pipeline::DemuxReceiverError::from(
                        TransportError::Closed,
                    )),
                }
            });
        match res {
            Ok(None) => Err(pyo3::exceptions::PyStopIteration::new_err(())),
            Ok(Some(ev)) => crate::mpegts::convert_event(py, &ev),
            Err(e) => Err(demux_recv_error_to_pyerr(py, e)),
        }
    }

    fn cancel_handle(&self, py: Python<'_>) -> PyResult<Py<PyCancelHandle>> {
        Py::new(py, PyCancelHandle::from_arc(self.cancel.clone()))
    }

    /// Wire-level transport stats (RTT, bytes received, etc.) sourced
    /// from the underlying `ManagedRecvTransport::socket_stats`. Returns
    /// `SrtError(CLOSED)` if the receiver has been closed, or all-zero
    /// stats if the wrapper is mid-reconnect.
    fn socket_stats(&self, py: Python<'_>) -> PyResult<Py<PySocketStats>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| make_srt_error(py, "IO", "ManagedDemuxReceiver lock poisoned"))?;
        let inner = guard
            .as_ref()
            .ok_or_else(|| make_srt_error(py, "CLOSED", "ManagedDemuxReceiver is closed"))?;
        let core = inner.socket_stats().unwrap_or_default();
        Py::new(py, PySocketStats::from_core(core))
    }

    /// SRT-specific stats. Same access pattern as `socket_stats` — peers
    /// the inner managed transport for its `SocketStats`, projects out
    /// the SRT-only fields. Today this returns the same `SocketStats`
    /// view as `socket_stats` because `ManagedRecvTransport` doesn't
    /// expose a separate SRT stats accessor — they're already in the
    /// `SocketStats` shape.
    fn srt_stats(&self, py: Python<'_>) -> PyResult<Py<PySocketStats>> {
        // Same as socket_stats today; reserved for future projection.
        self.socket_stats(py)
    }

    /// Total number of times the reconnect factory has been invoked
    /// since construction. Mirror of `ManagedMuxSender.reconnect_attempts`.
    fn reconnect_attempts(&self) -> u64 {
        self.factory_attempts.load(Ordering::Acquire)
    }

    /// Close the receiver. Fires the cancel handle BEFORE acquiring the
    /// mutex so a concurrent `__next__` parked in `recv_event` unparks
    /// promptly. Idempotent.
    fn close(&self, py: Python<'_>) {
        self.cancel.cancel();
        let inner = self.inner.clone();
        py.allow_threads(move || {
            if let Ok(mut guard) = inner.lock() {
                if let Some(mut r) = guard.take() {
                    r.close();
                }
            }
        });
    }

    fn is_alive(&self) -> bool {
        match self.inner.try_lock() {
            Ok(g) => g.as_ref().is_some_and(|r| r.is_alive()),
            // Lock currently held by a parked __next__ — optimistic.
            Err(_) => true,
        }
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __exit__(
        &self,
        py: Python<'_>,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        self.close(py);
        false
    }

    fn __repr__(&self) -> String {
        match self.inner.try_lock() {
            Ok(g) => match g.as_ref() {
                Some(_) => format!(
                    "ManagedDemuxReceiver(open, reconnect_attempts={})",
                    self.reconnect_attempts()
                ),
                None => "ManagedDemuxReceiver(closed)".to_string(),
            },
            Err(_) => "ManagedDemuxReceiver(<busy>)".to_string(),
        }
    }
}

/// Internal helper for `ManagedDemuxReceiver::from_url` — combines the
/// listen-vs-connect failure paths inside one `allow_threads` block.
enum BindOrConnect {
    ListenInitial(TransportError),
    ConnectInitial(TransportError),
}

// Silence "unused" lints for the BindError/AcceptError helpers — they're
// only used by the non-Managed receiver. We import them transitively for
// future symmetry but don't use them here (the managed path collapses
// listen+accept into one helper that returns `TransportError`).
#[allow(dead_code)]
fn _suppress_warns(py: Python<'_>) {
    let _ = bind_error_to_pyerr;
    let _ = accept_error_to_pyerr;
    let _ = connect_error_to_pyerr;
    let _ = py;
}

// ---------------------------------------------------------------------------
// Module registration.
// ---------------------------------------------------------------------------

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyManagedMuxSender>()?;
    m.add_class::<PyManagedDemuxReceiver>()?;
    Ok(())
}
