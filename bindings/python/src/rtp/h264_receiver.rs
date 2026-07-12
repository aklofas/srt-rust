//! Python mirror of the RFC 6184 H.264 receive surface.
//!
//! Exports:
//! - `ParameterSetInjection` — IntEnum-shaped pyclass (NONE / BEFORE_IDR)
//! - `H264DepayConfig`       — frozen pyclass with kwargs ctor
//! - `H264AccessUnit`        — frozen pyclass, get_all fields
//! - `H264DepayStats`        — frozen pyclass, get_all 9 counters
//! - `RtpStats`              — frozen pyclass, get_all (malformed_packets)
//! - `H264Receiver`          — blocking receiver; GIL released in recv_au
//!
//! # GIL release discipline
//!
//! `recv_au` releases the GIL via `py.allow_threads(|| ...)` (the DA-PY-1
//! lesson): the blocking Rust call parks the calling thread on a kernel
//! read timeout and must not hold the GIL, or all other Python threads
//! freeze.  All other methods are fast (stat reads, flag flips) and do
//! not release the GIL.
//!
//! # Handle lifetime
//!
//! `H264Receiver` wraps `Option<tst_rtp::H264Receiver>`.  All methods
//! that access the inner value check `Option::as_mut`/`as_ref` and raise
//! `RtpError(TRANSPORT, "receiver is closed")` when the option is
//! `None`. `close()` takes the inner value (dropping it) and fires the
//! cancel handle so any parked `recv_au` on another thread unparks promptly.
//!
//! # Error mapping
//!
//! - `TransportError::ExplicitClose` → `RtpError(CANCELLED, ...)`
//! - `TransportError::Broken`        → `RtpError(TRANSPORT, ...)`
//! - closed-handle calls             → `RtpError(TRANSPORT, "receiver is closed")`
//! - `ConnectError` (URL / bind)     → `RtpError(TRANSPORT, ...)`

#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use std::sync::Arc;

use pyo3::prelude::*;
use pyo3::types::PyBytes;

use tst_core::transport::TransportError;
use tst_rtp::cancel::RtpCancelHandle;
use tst_rtp::transport::RtpStats;
use tst_rtp::{
    ConnectError, H264Au, H264DepayConfig, H264DepayStats, H264Receiver, ParameterSetInjection,
};

use crate::errors::make_rtp_error;
use crate::rtp::transport::{PyCancelHandle, PySocketStats};

// ---------------------------------------------------------------------------
// Error mapping helpers
// ---------------------------------------------------------------------------

fn transport_error_to_pyerr(py: Python<'_>, e: TransportError) -> PyErr {
    match e {
        TransportError::ExplicitClose => {
            make_rtp_error(py, "CANCELLED", "transport cancelled by caller")
        }
        TransportError::TooLarge { len, max } => {
            let msg = format!("payload too large: {len} bytes exceeds {max}-byte cap");
            make_rtp_error(py, "MALFORMED_PACKET", &msg)
        }
        other => make_rtp_error(py, "TRANSPORT", &other.to_string()),
    }
}

fn connect_error_to_pyerr(py: Python<'_>, e: ConnectError) -> PyErr {
    make_rtp_error(py, "TRANSPORT", &e.to_string())
}

// ---------------------------------------------------------------------------
// ParameterSetInjection — IntEnum-shaped PyClass
// ---------------------------------------------------------------------------

/// Controls whether out-of-band SPS/PPS are injected before IDR frames.
///
/// Mirrors `tst_rtp::ParameterSetInjection`.
///
/// `NONE`       — pass NALUs through as-is.
/// `BEFORE_IDR` — inject cached SPS/PPS before every IDR frame (the default).
#[pyclass(eq, eq_int, name = "ParameterSetInjection", module = "tstrans.rtp")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms, non_camel_case_types)]
pub enum PyParameterSetInjection {
    /// No injection — pass NALUs through exactly as received.
    NONE = 0,
    /// Inject cached SPS and PPS NALUs before every IDR frame.
    BEFORE_IDR = 1,
}

impl PyParameterSetInjection {
    fn to_rust(self) -> ParameterSetInjection {
        match self {
            PyParameterSetInjection::NONE => ParameterSetInjection::None,
            PyParameterSetInjection::BEFORE_IDR => ParameterSetInjection::BeforeIdr,
        }
    }
    fn from_rust(r: ParameterSetInjection) -> Self {
        match r {
            ParameterSetInjection::BeforeIdr => PyParameterSetInjection::BEFORE_IDR,
            _ => PyParameterSetInjection::NONE,
        }
    }
}

// ---------------------------------------------------------------------------
// H264DepayConfig — frozen pyclass with kwargs ctor
// ---------------------------------------------------------------------------

/// Depacketizer configuration. Frozen — construct once and pass to
/// `H264Receiver.listen(url, config=...)` or `RtspClient.connect_h264`.
///
/// All defaults are derived from `tst_rtp::H264DepayConfig::default()`,
/// not re-stated as literals in Python, so they stay in sync with the
/// Rust implementation automatically.
///
/// `initial_parameter_sets` accepts a list of raw NALU bytes (type 7 for
/// SPS, type 8 for PPS). Last SPS and PPS each win when multiple are
/// provided; seeding does NOT tick `parameter_set_updates`.
#[pyclass(name = "H264DepayConfig", module = "tstrans.rtp", frozen)]
#[derive(Debug, Clone)]
pub struct PyH264DepayConfig {
    inner: H264DepayConfig,
}

#[pymethods]
impl PyH264DepayConfig {
    /// Construct with optional keyword arguments. Defaults mirror
    /// `tst_rtp::H264DepayConfig::default()` (payload_type=96,
    /// parameter_set_injection=BEFORE_IDR, initial_parameter_sets=[],
    /// max_au_bytes=8388608).
    #[new]
    #[pyo3(signature = (
        *,
        payload_type = None,
        parameter_set_injection = None,
        initial_parameter_sets = None,
        max_au_bytes = None,
    ))]
    fn new(
        payload_type: Option<u8>,
        parameter_set_injection: Option<PyParameterSetInjection>,
        initial_parameter_sets: Option<Vec<Vec<u8>>>,
        max_au_bytes: Option<usize>,
    ) -> PyResult<Self> {
        // Start from Rust defaults so we never re-state literal values here.
        let mut inner = H264DepayConfig::default();
        if let Some(pt) = payload_type {
            inner.payload_type = pt;
        }
        if let Some(psi) = parameter_set_injection {
            inner.parameter_set_injection = psi.to_rust();
        }
        if let Some(ps) = initial_parameter_sets {
            inner.initial_parameter_sets = ps;
        }
        if let Some(m) = max_au_bytes {
            // Reject a zero cap (matches the JVM builder): a zero-byte AU cap
            // drops every AU and is never useful for real depacketization.
            if m == 0 {
                return Err(pyo3::exceptions::PyValueError::new_err(
                    "max_au_bytes must be positive",
                ));
            }
            inner.max_au_bytes = m;
        }
        Ok(Self { inner })
    }

    #[getter]
    fn payload_type(&self) -> u8 {
        self.inner.payload_type
    }

    #[getter]
    fn parameter_set_injection(&self) -> PyParameterSetInjection {
        PyParameterSetInjection::from_rust(self.inner.parameter_set_injection)
    }

    #[getter]
    fn initial_parameter_sets<'py>(&self, py: Python<'py>) -> Vec<Bound<'py, PyBytes>> {
        self.inner
            .initial_parameter_sets
            .iter()
            .map(|b| PyBytes::new_bound(py, b))
            .collect()
    }

    #[getter]
    fn max_au_bytes(&self) -> usize {
        self.inner.max_au_bytes
    }

    fn __repr__(&self) -> String {
        format!(
            "H264DepayConfig(payload_type={}, parameter_set_injection={:?}, \
             initial_parameter_sets=[{} item(s)], max_au_bytes={})",
            self.inner.payload_type,
            self.inner.parameter_set_injection,
            self.inner.initial_parameter_sets.len(),
            self.inner.max_au_bytes,
        )
    }
}

// ---------------------------------------------------------------------------
// H264AccessUnit — frozen, get_all
// ---------------------------------------------------------------------------

/// A fully reassembled H.264 Access Unit. Frozen.
///
/// `annexb`       — Annex B–framed NALU bytes (start codes prepended).
/// `pts`          — 90 kHz decode-order timestamp (i64 ticks from Pts90khz).
/// `key_frame`    — `True` if the AU contains an IDR slice (NALU type 5).
/// `rtp_timestamp`— raw 32-bit RTP timestamp from the packet header.
#[pyclass(name = "H264AccessUnit", module = "tstrans.rtp", frozen)]
pub struct PyH264AccessUnit {
    /// Annex B framed bytes (heap copy — new `bytes` object for Python).
    pub annexb: Py<PyBytes>,
    /// Byte length of `annexb`, captured at construction so `__repr__`
    /// can render it without needing a Python handle.
    annexb_len: usize,
    /// Decode-order 90 kHz timestamp, as i64 ticks.
    pub pts: i64,
    /// True when the AU contains at least one IDR slice (NALU type 5).
    pub key_frame: bool,
    /// Raw 32-bit RTP timestamp from the packet header.
    pub rtp_timestamp: u32,
}

#[pymethods]
impl PyH264AccessUnit {
    #[getter]
    fn annexb(&self, py: Python<'_>) -> Py<PyBytes> {
        self.annexb.clone_ref(py)
    }

    #[getter]
    fn pts(&self) -> i64 {
        self.pts
    }

    #[getter]
    fn key_frame(&self) -> bool {
        self.key_frame
    }

    #[getter]
    fn rtp_timestamp(&self) -> u32 {
        self.rtp_timestamp
    }

    fn __repr__(&self) -> String {
        format!(
            "H264AccessUnit(pts={}, key_frame={}, rtp_timestamp={}, \
             annexb=<{} bytes>)",
            self.pts, self.key_frame, self.rtp_timestamp, self.annexb_len,
        )
    }
}

impl PyH264AccessUnit {
    /// Construct from a Rust `H264Au`, copying the annexb bytes into a
    /// new Python `bytes` object.
    pub(crate) fn from_rust(py: Python<'_>, au: H264Au) -> PyResult<Self> {
        Ok(Self {
            annexb: PyBytes::new_bound(py, &au.annexb).unbind(),
            annexb_len: au.annexb.len(),
            pts: au.pts.as_ticks(),
            key_frame: au.key_frame,
            rtp_timestamp: au.rtp_timestamp,
        })
    }
}

// ---------------------------------------------------------------------------
// H264DepayStats — frozen, get_all (9 counters)
// ---------------------------------------------------------------------------

/// Depacketizer counters. Frozen snapshot returned by
/// `H264Receiver.depay_stats()`.
#[pyclass(name = "H264DepayStats", module = "tstrans.rtp", frozen, get_all)]
pub struct PyH264DepayStats {
    /// Number of complete, unpoisoned AUs emitted.
    pub aus_emitted: u64,
    /// AUs discarded due to poisoning (seq gaps, F-bit, etc.).
    /// Includes oversize drops (also counted in `aus_dropped_oversize`).
    pub aus_dropped: u64,
    /// AUs dropped specifically for exceeding `max_au_bytes`.
    pub aus_dropped_oversize: u64,
    /// RTP packets discarded (empty, reserved, interleaved types).
    pub packets_discarded: u64,
    /// NALUs discarded (F-bit set, open FU at AU completion, etc.).
    pub nalus_discarded: u64,
    /// Sequence-number gaps detected.
    pub seq_gaps: u64,
    /// Duplicate sequence numbers detected.
    pub duplicate_packets: u64,
    /// Times cached parameter sets were updated (in-band SPS/PPS changed).
    pub parameter_set_updates: u64,
    /// SSRC changes (source restarts) detected.
    pub ssrc_changes: u64,
}

impl PyH264DepayStats {
    fn from_rust(s: H264DepayStats) -> Self {
        Self {
            aus_emitted: s.aus_emitted,
            aus_dropped: s.aus_dropped,
            aus_dropped_oversize: s.aus_dropped_oversize,
            packets_discarded: s.packets_discarded,
            nalus_discarded: s.nalus_discarded,
            seq_gaps: s.seq_gaps,
            duplicate_packets: s.duplicate_packets,
            parameter_set_updates: s.parameter_set_updates,
            ssrc_changes: s.ssrc_changes,
        }
    }
}

#[pymethods]
impl PyH264DepayStats {
    fn __repr__(&self) -> String {
        format!(
            "H264DepayStats(aus_emitted={}, aus_dropped={}, seq_gaps={}, \
             ssrc_changes={}, parameter_set_updates={})",
            self.aus_emitted,
            self.aus_dropped,
            self.seq_gaps,
            self.ssrc_changes,
            self.parameter_set_updates,
        )
    }
}

// ---------------------------------------------------------------------------
// PyRtpStats — frozen mirror of tst_rtp::transport::RtpStats
// ---------------------------------------------------------------------------

/// RTP protocol–level statistics snapshot. Frozen.
///
/// `malformed_packets` — number of received datagrams with an invalid RTP
/// header, wrong payload type, or empty payload. Cumulative since
/// `listen()`.
#[pyclass(name = "RtpStats", module = "tstrans.rtp", frozen, get_all)]
pub struct PyRtpStats {
    pub malformed_packets: u64,
}

impl PyRtpStats {
    fn from_rust(s: RtpStats) -> Self {
        Self {
            malformed_packets: s.malformed_packets,
        }
    }
}

#[pymethods]
impl PyRtpStats {
    fn __repr__(&self) -> String {
        format!("RtpStats(malformed_packets={})", self.malformed_packets)
    }
}

// ---------------------------------------------------------------------------
// H264Receiver — blocking recv shell
// ---------------------------------------------------------------------------

/// Blocking H.264-over-RTP receiver. Wraps `tst_rtp::H264Receiver`.
///
/// # Constructing
///
/// `H264Receiver.listen(url, config=None)` — parse an `rtp://host:port?pt=N`
/// URL, bind a UDP socket, and return a ready receiver. The `?pt=` query
/// parameter is required (range 1..=127, value 33 rejected).
///
/// The crate-private `PyH264Receiver::from_h264_receiver` constructor is
/// used by `PyRtspSession::into_h264_receiver` to wrap an already-built
/// `H264Receiver` from the RTSP session bridge.
///
/// # Receiving
///
/// `recv_au()` blocks until a complete Access Unit is reassembled or EOS.
/// Returns `H264AccessUnit` on success, `None` at EOS (clean close or RTSP
/// teardown), or raises `RtpError` on transport failure.
///
/// The GIL is released via `py.allow_threads()` during the blocking recv so
/// other Python threads continue to run while this thread waits for packets.
///
/// # Iterator protocol
///
/// `H264Receiver` is its own iterator. `__next__` calls `recv_au()` and
/// raises `StopIteration` at EOS (when `recv_au()` returns `None`).
///
/// # Lifecycle
///
/// `close()` fires the cancel handle and drops the underlying source;
/// subsequent `recv_au()` calls raise `RtpError(TRANSPORT)`. Idempotent.
#[pyclass(name = "H264Receiver", module = "tstrans.rtp")]
pub struct PyH264Receiver {
    /// Live receiver. `None` once `close()` has been called.
    inner: Option<H264Receiver>,
    /// Cancel handle pulled from the receiver at construction. Held
    /// separately so `close()` can fire it BEFORE taking `inner`, waking
    /// any thread parked in `recv_au` within ~100ms.
    cancel: Arc<RtpCancelHandle>,
}

impl PyH264Receiver {
    /// Crate-private constructor used by `PyRtspSession::into_h264_receiver`.
    pub(crate) fn from_h264_receiver(receiver: H264Receiver) -> Self {
        let cancel = receiver.cancel_handle();
        Self {
            inner: Some(receiver),
            cancel,
        }
    }
}

#[pymethods]
impl PyH264Receiver {
    /// Bind to `url` and return a ready `H264Receiver`.
    ///
    /// `url` must have the form `rtp://host:port?pt=N` where `N` is the
    /// dynamic payload type (range 1..=127; 33 is rejected — use
    /// `tstrans.rtp.DemuxReceiver` for MPEG-TS streams).
    ///
    /// `config` overrides the depacketizer configuration. When `None`,
    /// `H264DepayConfig()` defaults are used (payload type is overridden
    /// from the URL's `?pt=` parameter regardless of `config.payload_type`).
    ///
    /// Raises `RtpError(TRANSPORT)` on URL parse failure, missing `?pt=`,
    /// or socket bind error.
    #[staticmethod]
    #[pyo3(signature = (url, config = None))]
    fn listen(py: Python<'_>, url: &str, config: Option<&PyH264DepayConfig>) -> PyResult<Self> {
        let receiver = match config {
            None => H264Receiver::listen(url).map_err(|e| connect_error_to_pyerr(py, e))?,
            Some(cfg) => {
                // listen_with parses the URL, picks the pt from the URL
                // (overriding config.inner.payload_type), and binds the socket.
                let parsed = tst_rtp::url::RtpUrl::parse(url)
                    .map_err(|e| make_rtp_error(py, "TRANSPORT", &e.to_string()))?;
                H264Receiver::listen_with(&parsed, cfg.inner.clone())
                    .map_err(|e| connect_error_to_pyerr(py, e))?
            }
        };
        let cancel = receiver.cancel_handle();
        Ok(Self {
            inner: Some(receiver),
            cancel,
        })
    }

    /// Receive the next reassembled H.264 Access Unit.
    ///
    /// Blocks until a packet arrives (releasing the GIL) or EOS / error.
    ///
    /// Returns:
    /// - `H264AccessUnit` when a complete AU is available.
    /// - `None` at EOS (clean close, cancel, or RTSP teardown).
    ///
    /// Raises:
    /// - `RtpError(CANCELLED)` if the cancel handle was fired explicitly.
    /// - `RtpError(TRANSPORT)` on a hard I/O error or if the receiver is
    ///   already closed.
    fn recv_au(&mut self, py: Python<'_>) -> PyResult<Option<Py<PyH264AccessUnit>>> {
        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| make_rtp_error(py, "TRANSPORT", "receiver is closed"))?;
        // Release the GIL while parked on the blocking recv_au call.
        // The inner receiver is exclusively owned here (no Arc/Mutex) so
        // py.allow_threads is safe: no Python objects are accessed inside.
        let result = py.allow_threads(|| inner.recv_au());
        match result {
            Ok(None) => Ok(None),
            Ok(Some(au)) => {
                let py_au = PyH264AccessUnit::from_rust(py, au)?;
                Ok(Some(Py::new(py, py_au)?))
            }
            Err(e) => Err(transport_error_to_pyerr(py, e)),
        }
    }

    /// Iterator protocol: `iter(rx)` returns self.
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Advance the iterator. Returns the next `H264AccessUnit` or raises
    /// `StopIteration` at EOS.
    fn __next__(&mut self, py: Python<'_>) -> PyResult<Py<PyH264AccessUnit>> {
        match self.recv_au(py)? {
            Some(au) => Ok(au),
            None => Err(pyo3::exceptions::PyStopIteration::new_err(())),
        }
    }

    /// RFC 6184 depacketizer counters (AU counts, seq gaps, etc.).
    fn depay_stats(&self, py: Python<'_>) -> PyResult<Py<PyH264DepayStats>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_rtp_error(py, "TRANSPORT", "receiver is closed"))?;
        Py::new(py, PyH264DepayStats::from_rust(inner.depay_stats()))
    }

    /// RTP protocol–level counters (malformed packet counter).
    fn rtp_stats(&self, py: Python<'_>) -> PyResult<Py<PyRtpStats>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_rtp_error(py, "TRANSPORT", "receiver is closed"))?;
        Py::new(py, PyRtpStats::from_rust(inner.rtp_stats()))
    }

    /// Throughput wire-level stats (bytes/packets received).
    fn socket_stats(&self, py: Python<'_>) -> PyResult<Py<PySocketStats>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_rtp_error(py, "TRANSPORT", "receiver is closed"))?;
        Py::new(py, PySocketStats::from_core(inner.socket_stats()))
    }

    /// Local address the UDP socket is bound to, as `"host:port"` string.
    /// Returns `None` only for a live TCP-interleaved (RTSP) receiver
    /// where no UDP socket exists.
    ///
    /// Raises `RtpError(TRANSPORT, "receiver is closed")` on a closed
    /// handle — matching the module's closed-handle contract — so `None`
    /// is never ambiguous between "closed" and "no UDP socket".
    fn local_addr(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_rtp_error(py, "TRANSPORT", "receiver is closed"))?;
        Ok(inner.local_addr().map(|a| a.to_string()))
    }

    /// Return a shareable cancel handle. Calling `.cancel()` on the
    /// returned handle wakes any thread parked in `recv_au()` within
    /// ~100ms; that call returns `None` (EOS) rather than raising.
    fn cancel_handle(&self, py: Python<'_>) -> PyResult<Py<PyCancelHandle>> {
        Py::new(
            py,
            PyCancelHandle {
                inner: self.cancel.clone(),
            },
        )
    }

    /// Close the receiver. Idempotent. Fires the cancel handle so any
    /// thread parked in `recv_au` unparks at the next cancel-poll tick
    /// (~100ms), then drops the underlying source.
    fn close(&mut self) {
        self.cancel.cancel();
        self.inner.take(); // drops the H264Receiver + its socket
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
            Some(_) => "H264Receiver(open)".to_string(),
            None => "H264Receiver(closed)".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Module registration
// ---------------------------------------------------------------------------

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyParameterSetInjection>()?;
    m.add_class::<PyH264DepayConfig>()?;
    m.add_class::<PyH264AccessUnit>()?;
    m.add_class::<PyH264DepayStats>()?;
    m.add_class::<PyRtpStats>()?;
    m.add_class::<PyH264Receiver>()?;
    Ok(())
}
