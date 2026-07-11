//! `Sender`, `Receiver`, `SocketStats`, `CancelHandle` for RTP.
//!
//! PyO3 wrappers for `tst_rtp::RtpTransport` (send) and
//! `tst_rtp::RtpRecvTransport` (recv).  Each PyClass wraps a single
//! concrete transport — NOT generic over `T: Transport` — matching the
//! Stage 1 tst-c lesson #1 (handles concrete per-transport).
//!
//! GIL boundaries (per `docs/specs/2026-05-26-tst-rtp-phase-4-binding-exposure-design.md`):
//! - `send`, `recv` → wrapped in `py.allow_threads(|| ...)` so concurrent
//!   Python threads can keep working while UDP I/O blocks on the kernel.
//! - `stats`, `cancel_handle`, `cancel`, `__enter__`, `__exit__` → fast
//!   read-only / atomic operations; no GIL release.
//!
//! Bytes-like extraction in `.send(ts_bytes)` follows the audit-backlog
//! #10 two-path pattern: fast `&[u8]` extract (zero-copy for `bytes`),
//! fallback through Python's `bytes()` builtin for `bytearray` /
//! `memoryview` (one C copy). Required under PyO3's abi3-py310 feature
//! since `PyBuffer` is gated behind `not(Py_LIMITED_API)`.
//!
//! Error mapping: `tst_core::transport::TransportError` →
//! `tstrans.exceptions.RtpError` with `.kind` set to one of the three
//! `RtpErrorKind` variants. Mapping table:
//! - `ExplicitClose` → `CANCELLED`
//! - `TooLarge`      → `MALFORMED_PACKET`
//! - all others (`Broken`, `Backpressure`, `Closed`) → `TRANSPORT`
//!
//! The 25th bash ratchet `scripts/check-py-rtp-error-mapping-coverage.sh`
//! enforces that every `RtpErrorKind` variant has at least one literal
//! `make_rtp_error(py, "<VARIANT>", ...)` call site in this crate.

#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use std::sync::Arc;

use pyo3::Py;
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use tst_core::transport::{RecvTransport, SocketStats, Transport, TransportCancel, TransportError};
use tst_rtp::builder::RtpRecvSocketBuilder;
use tst_rtp::{ConnectError, RtpRecvTransport, RtpSocketBuilder, RtpTransport};

use crate::errors::make_rtp_error;

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/// Map a `TransportError` raised by `RtpTransport::send_bytes` or
/// `RtpRecvTransport::recv_bytes` into a `tstrans.exceptions.RtpError`
/// instance carrying the right `RtpErrorKind`.
///
/// Each of the three `RtpErrorKind` variants gets a literal call site
/// below so the `check-py-rtp-error-mapping-coverage.sh` ratchet stays
/// green.
fn transport_error_to_pyerr(py: Python<'_>, e: TransportError) -> PyErr {
    match e {
        TransportError::ExplicitClose => {
            make_rtp_error(py, "CANCELLED", "transport cancelled by caller")
        }
        TransportError::TooLarge { len, max } => {
            let msg = format!("payload too large: {len} bytes exceeds {max}-byte cap");
            make_rtp_error(py, "MALFORMED_PACKET", &msg)
        }
        // Backpressure + Broken + Closed all surface as TRANSPORT — the
        // free-text message carries the specific Rust variant.
        other => make_rtp_error(py, "TRANSPORT", &other.to_string()),
    }
}

/// Map a `ConnectError` raised by `RtpSocketBuilder::build` /
/// `RtpRecvSocketBuilder::build` into an `RtpError`.
fn connect_error_to_pyerr(py: Python<'_>, e: ConnectError) -> PyErr {
    // All connect-time failures are reported as TRANSPORT — URL parse,
    // bind / connect / setsockopt errors, multicast-iface-unsupported.
    // The free-text message carries the specific Rust variant.
    make_rtp_error(py, "TRANSPORT", &e.to_string())
}

// ---------------------------------------------------------------------------
// PySocketStats — frozen mirror of tst_core::transport::SocketStats
// ---------------------------------------------------------------------------

/// Mirror of `tst_core::transport::SocketStats` exposed to Python as
/// a frozen, get_all-decorated PyClass. Fields match the Rust struct
/// 1:1 — `RtpTransport` populates `bytes_sent` / `packets_sent` only
/// in Phase 1; `RtpRecvTransport` populates the receive-side counters.
/// The RTCP-derived fields (`rtt_us`, `packets_lost_*`) stay zero
/// until RTCP RR/SR ingest is wired.
#[pyclass(frozen, get_all, name = "SocketStats", module = "tstrans.rtp")]
pub(crate) struct PySocketStats {
    pub rtt_us: u32,
    pub send_bandwidth_bps: u64,
    pub recv_bandwidth_bps: u64,
    pub link_bandwidth_bps: u64,
    pub bytes_sent: u64,
    pub packets_sent: u64,
    pub bytes_received: u64,
    pub packets_received: u64,
    pub bytes_lost_recv: u64,
    pub packets_lost_recv: u64,
    pub packets_lost_send: u64,
    pub packets_retransmitted: u64,
    pub packets_dropped_send: u64,
    pub packets_dropped_recv: u64,
    pub send_buffer_packets: u32,
    pub recv_buffer_packets: u32,
}

impl PySocketStats {
    pub(crate) fn from_core(s: SocketStats) -> Self {
        Self {
            rtt_us: s.rtt_us,
            send_bandwidth_bps: s.send_bandwidth_bps,
            recv_bandwidth_bps: s.recv_bandwidth_bps,
            link_bandwidth_bps: s.link_bandwidth_bps,
            bytes_sent: s.bytes_sent,
            packets_sent: s.packets_sent,
            bytes_received: s.bytes_received,
            packets_received: s.packets_received,
            bytes_lost_recv: s.bytes_lost_recv,
            packets_lost_recv: s.packets_lost_recv,
            packets_lost_send: s.packets_lost_send,
            packets_retransmitted: s.packets_retransmitted,
            packets_dropped_send: s.packets_dropped_send,
            packets_dropped_recv: s.packets_dropped_recv,
            send_buffer_packets: s.send_buffer_packets,
            recv_buffer_packets: s.recv_buffer_packets,
        }
    }
}

#[pymethods]
impl PySocketStats {
    fn __repr__(&self) -> String {
        format!(
            "SocketStats(bytes_sent={}, packets_sent={}, bytes_received={}, packets_received={}, rtt_us={})",
            self.bytes_sent,
            self.packets_sent,
            self.bytes_received,
            self.packets_received,
            self.rtt_us,
        )
    }
}

// ---------------------------------------------------------------------------
// PyCancelHandle — Arc-shared so multiple Python refs share one target
// ---------------------------------------------------------------------------

/// Python-side cancel handle. Wraps an `Arc<dyn TransportCancel>` so
/// multiple Python references (e.g., one held by the Sender, one
/// stashed in a worker thread) share a single cancellation target.
/// Calling `.cancel()` on any reference wakes any thread parked in
/// the paired transport's send/recv loop within ~100 ms.
///
/// The trait-erased Arc comes from `Transport::cancel_handle()` /
/// `RecvTransport::cancel_handle()` — the transport's own internal
/// `Arc<RtpCancelHandle>` shared with the cancel-poll loop. Cancelling
/// here flips the same atomic the transport polls.
#[pyclass(name = "CancelHandle", module = "tstrans.rtp")]
pub(crate) struct PyCancelHandle {
    pub(crate) inner: Arc<dyn TransportCancel + Send + Sync>,
}

#[pymethods]
impl PyCancelHandle {
    /// Signal cancellation. Idempotent — repeated calls are a no-op.
    /// Wakes a thread parked in `Sender.send` / `Receiver.recv` at the
    /// next 100 ms cancel-poll tick; that call returns an `RtpError`
    /// with `.kind == RtpErrorKind.CANCELLED`.
    fn cancel(&self) {
        self.inner.cancel();
    }

    fn __repr__(&self) -> String {
        "CancelHandle()".to_string()
    }
}

// ---------------------------------------------------------------------------
// PySender — wraps tst_rtp::RtpTransport
// ---------------------------------------------------------------------------

/// Python RTP sender — wraps `tst_rtp::RtpTransport`.
///
/// Constructed from an `rtp://host:port` URL plus optional `pkt_size`
/// (188-multiple, default 1316) and `ssrc` (random when omitted)
/// keyword arguments. Other URL query parameters (`ttl=`, `iface=`)
/// can be embedded in the URL itself.
#[pyclass(name = "Sender", module = "tstrans.rtp")]
pub(crate) struct PySender {
    inner: Option<RtpTransport>,
    /// Trait-erased cancel handle pulled from `Transport::cancel_handle()`
    /// at construction. Shared with any Python-side `CancelHandle` clones;
    /// calling `.cancel()` here flips the same atomic the transport's
    /// send loop polls every 100 ms.
    cancel: Arc<dyn TransportCancel + Send + Sync>,
}

#[pymethods]
impl PySender {
    /// Construct a sender connected to `url` (e.g.
    /// `"rtp://127.0.0.1:5004"`).
    ///
    /// `pkt_size` overrides the UDP datagram size (RTP header + TS
    /// payload). `ssrc` pins the RTP synchronization source identifier;
    /// when omitted the transport picks a random one.
    #[new]
    #[pyo3(signature = (url, *, pkt_size = 1316, ssrc = None))]
    fn new(py: Python<'_>, url: &str, pkt_size: usize, ssrc: Option<u32>) -> PyResult<Self> {
        let mut builder = RtpSocketBuilder::from_url(url)
            .map_err(|e| make_rtp_error(py, "TRANSPORT", &e.to_string()))?;
        builder.pkt_size(pkt_size);
        if let Some(s) = ssrc {
            builder.ssrc(s);
        }
        let inner = builder.build().map_err(|e| connect_error_to_pyerr(py, e))?;
        // Pull the cancel handle BEFORE we own `inner` mutably so the
        // borrow is short-lived. The Arc returned is the same one the
        // transport's send-loop holds — flipping it here wakes a parked
        // send on the next 100 ms cancel-poll tick.
        let cancel = inner
            .cancel_handle()
            .expect("RtpTransport always returns Some(cancel_handle)");
        Ok(Self {
            inner: Some(inner),
            cancel,
        })
    }

    /// Send one MPEG-TS payload chunk over RTP. Accepts any bytes-like
    /// input: `bytes`, `bytearray`, `memoryview` (over either), and
    /// any object implementing the buffer protocol.
    ///
    /// Releases the GIL during the underlying `sendto` call so other
    /// Python threads can run while this thread blocks on the kernel.
    fn send(&mut self, py: Python<'_>, ts_bytes: &Bound<'_, PyAny>) -> PyResult<()> {
        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| make_rtp_error(py, "TRANSPORT", "sender is closed"))?;
        // Fast path: real `bytes` extracts to a borrowed &[u8] (zero-copy).
        if let Ok(slice) = ts_bytes.extract::<&[u8]>() {
            let res = py.allow_threads(|| inner.send_bytes(slice));
            return res.map_err(|e| transport_error_to_pyerr(py, e));
        }
        // Fallback for bytearray / memoryview / numpy uint8 / etc.
        // Coerce through Python's `bytes()` builtin — one C copy into a
        // fresh immutable PyBytes. PyBuffer would skip this copy but
        // it's gated on not(Py_LIMITED_API) in PyO3 0.22 and we build
        // with abi3-py310 for one-wheel coverage of 3.10+.
        let coerced: Bound<'_, PyBytes> = py
            .import_bound("builtins")?
            .getattr(intern!(py, "bytes"))?
            .call1((ts_bytes,))?
            .downcast_into::<PyBytes>()?;
        let slice: &[u8] = coerced.as_bytes();
        let res = py.allow_threads(|| inner.send_bytes(slice));
        res.map_err(|e| transport_error_to_pyerr(py, e))
    }

    /// Snapshot of wire-level statistics. Returns a frozen `SocketStats`
    /// dataclass; the `bytes_sent` / `packets_sent` counters tick on
    /// each successful `.send()`.
    fn stats(&self, py: Python<'_>) -> PyResult<Py<PySocketStats>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_rtp_error(py, "TRANSPORT", "sender is closed"))?;
        let core_stats = inner.socket_stats().unwrap_or_default();
        Py::new(py, PySocketStats::from_core(core_stats))
    }

    /// Return a shareable cancel handle. Calling `.cancel()` on the
    /// returned handle wakes any thread currently parked in `.send()`;
    /// that call returns `RtpError(kind=CANCELLED)`.
    fn cancel_handle(&self, py: Python<'_>) -> PyResult<Py<PyCancelHandle>> {
        Py::new(
            py,
            PyCancelHandle {
                inner: self.cancel.clone(),
            },
        )
    }

    /// Close the sender. After close, further `.send()` calls raise
    /// `RtpError(kind=TRANSPORT)`. Idempotent.
    fn close(&mut self) {
        if let Some(mut t) = self.inner.take() {
            t.close();
        }
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
        false // do not suppress exceptions
    }

    fn __repr__(&self) -> String {
        match &self.inner {
            Some(_) => "Sender(open)".to_string(),
            None => "Sender(closed)".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// PyReceiver — wraps tst_rtp::RtpRecvTransport
// ---------------------------------------------------------------------------

/// Python RTP receiver — wraps `tst_rtp::RtpRecvTransport`.
///
/// Binds to `url` (literal IP:port). For multicast URLs, joins the
/// group automatically. `pkt_size` sizes the recv scratch buffer; the
/// 12-byte RTP header is stripped internally so `.recv()` returns just
/// the TS payload bytes.
#[pyclass(name = "Receiver", module = "tstrans.rtp")]
pub(crate) struct PyReceiver {
    inner: Option<RtpRecvTransport>,
    /// Trait-erased cancel handle pulled from
    /// `RecvTransport::cancel_handle()` at construction. Shared with any
    /// Python-side `CancelHandle` clones — flipping it wakes a parked
    /// recv on the next 100 ms cancel-poll tick.
    cancel: Arc<dyn TransportCancel + Send + Sync>,
    /// Per-recv scratch buffer sized to the underlying transport's
    /// `max_payload()`. Reused across calls to avoid a per-recv malloc.
    scratch: Vec<u8>,
}

#[pymethods]
impl PyReceiver {
    /// Bind a receiver to `url` (e.g. `"rtp://127.0.0.1:5004"` for
    /// unicast or `"rtp://239.0.0.1:5004"` for multicast).
    ///
    /// `pkt_size` overrides the recv scratch buffer size.
    #[new]
    #[pyo3(signature = (url, *, pkt_size = 1316))]
    fn new(py: Python<'_>, url: &str, pkt_size: usize) -> PyResult<Self> {
        let mut builder = RtpRecvSocketBuilder::from_url(url)
            .map_err(|e| make_rtp_error(py, "TRANSPORT", &e.to_string()))?;
        builder.pkt_size(pkt_size);
        let inner = builder.build().map_err(|e| connect_error_to_pyerr(py, e))?;
        let scratch_len = inner.max_payload();
        let cancel = inner
            .cancel_handle()
            .expect("RtpRecvTransport always returns Some(cancel_handle)");
        Ok(Self {
            inner: Some(inner),
            cancel,
            scratch: vec![0u8; scratch_len],
        })
    }

    /// Receive one MPEG-TS payload chunk. Blocks until a packet arrives
    /// (releases the GIL while parked) or the cancel handle fires.
    ///
    /// Returns a fresh `bytes` object containing the TS bundle (RTP
    /// header already stripped).
    fn recv(&mut self, py: Python<'_>) -> PyResult<Py<PyBytes>> {
        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| make_rtp_error(py, "TRANSPORT", "receiver is closed"))?;
        // SAFETY: scratch is owned by this PyClass instance and not
        // shared with Python objects; the &mut borrow is exclusive for
        // the duration of recv_bytes. py.allow_threads is safe because
        // we touch no Python objects inside.
        let scratch: &mut [u8] = self.scratch.as_mut_slice();
        let res = py.allow_threads(|| inner.recv_bytes(scratch));
        match res {
            Ok(n) => Ok(PyBytes::new_bound(py, &self.scratch[..n]).unbind()),
            Err(e) => Err(transport_error_to_pyerr(py, e)),
        }
    }

    /// Snapshot of wire-level statistics. Returns a frozen `SocketStats`
    /// dataclass; the `bytes_received` / `packets_received` counters
    /// tick on each successful `.recv()`.
    fn stats(&self, py: Python<'_>) -> PyResult<Py<PySocketStats>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| make_rtp_error(py, "TRANSPORT", "receiver is closed"))?;
        let core_stats = inner.socket_stats().unwrap_or_default();
        Py::new(py, PySocketStats::from_core(core_stats))
    }

    /// Return a shareable cancel handle. Calling `.cancel()` on the
    /// returned handle wakes any thread currently parked in `.recv()`;
    /// that call returns `RtpError(kind=CANCELLED)`.
    fn cancel_handle(&self, py: Python<'_>) -> PyResult<Py<PyCancelHandle>> {
        Py::new(
            py,
            PyCancelHandle {
                inner: self.cancel.clone(),
            },
        )
    }

    /// Close the receiver. After close, further `.recv()` calls raise
    /// `RtpError(kind=TRANSPORT)`. Idempotent.
    fn close(&mut self) {
        // Also flip the cancel so any parked .recv on a different
        // thread unparks promptly.
        self.cancel.cancel();
        if let Some(mut t) = self.inner.take() {
            t.close();
        }
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
            Some(_) => "Receiver(open)".to_string(),
            None => "Receiver(closed)".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Module registration
// ---------------------------------------------------------------------------

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySocketStats>()?;
    m.add_class::<PyCancelHandle>()?;
    m.add_class::<PySender>()?;
    m.add_class::<PyReceiver>()?;
    Ok(())
}
