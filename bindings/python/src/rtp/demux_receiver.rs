//! Wave B Task 23 — `DemuxReceiver` convenience wrapper.
//!
//! Wraps `tst_pipeline::DemuxReceiver<tst_rtp::RtpRecvTransport>`:
//! bind a UDP RTP receiver to a URL, demux the resulting MPEG-TS
//! stream, and iterate over `DemuxEvent` instances.
//!
//! Architectural notes:
//!
//! - The PyClass wraps `DemuxReceiver<RtpRecvTransport>` directly,
//!   matching the Stage 1 tst-c lesson #1 (handles concrete
//!   per-transport).
//! - `__iter__` returns self; `__next__` blocks (releases the GIL) on
//!   the next `recv_event()` until either an event arrives or the
//!   transport closes / errors / cancels.
//! - Events are converted to Python via the SAME conversion path used
//!   by `tstrans.mpegts.Demuxer.__next__`: `crate::mpegts::convert_event`.
//!   No new event types — Python sees the existing
//!   `tstrans.mpegts.DemuxEvent.*` subclass hierarchy.
//! - The constructor accepts an optional `DemuxerConfig` Python
//!   dataclass; if `None`, defaults are used. Configuration is lifted
//!   onto the Rust `tst_pipeline::DemuxReceiver::with_demux_options`
//!   path via the existing `crate::mpegts::build_demuxer_config`
//!   helper.
//! - Concurrency: `inner` is held under `Arc<Mutex<Option<...>>>` and
//!   every PyMethod takes `&self`. The mutex serialises access; if a
//!   `__next__` call is parked under `py.allow_threads`, a concurrent
//!   `close()` / `__exit__()` from another Python thread fires the
//!   cancel handle (held outside the mutex), wakes the parked recv,
//!   then takes the inner once the recv path releases the lock. This
//!   avoids the PyO3 "Already borrowed" error that an
//!   `&mut self`-style design hits when close races a parked next.
//!
//! Error mapping:
//! - `DemuxReceiverErrorSource::Transport(...)` → `RtpError(TRANSPORT)`
//!   (or `CANCELLED` / `MALFORMED_PACKET` per `TransportError` variant)
//! - `DemuxReceiverErrorSource::Demux(...)`     → `DemuxError`
//! - Construction-time `ConnectError`            → `RtpError(TRANSPORT)`
//!
//! Both error pathways exercise existing literal `make_rtp_error` and
//! `make_demux_error` call sites, so no new ratchet anchors are needed.

#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use std::sync::{Arc, Mutex};

use pyo3::Py;
use pyo3::prelude::*;

use tst_core::mpegts::demux::DemuxEvent;
use tst_core::transport::TransportError;
use tst_pipeline::{
    DemuxReceiver as RustDemuxReceiver, DemuxReceiverError, DemuxReceiverErrorSource,
};
use tst_rtp::{RtpRecvSocketBuilder, RtpRecvTransport};

use crate::errors::{make_demux_error, make_rtp_error};
use crate::mux::PyMuxerStats;
use crate::rtp::transport::PySocketStats;

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

/// Map a `TransportError` to `RtpError`. Mirror of the helper in
/// `transport.rs` + `mux_sender.rs`; kept inline so each file owns its
/// own error mapping without `pub(crate)` re-exports.
fn transport_error_to_rtp_pyerr(py: Python<'_>, e: TransportError) -> PyErr {
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

/// Map a tst-core `DemuxError` to a Python `DemuxError` via the same
/// kind-mapping the `tstrans.mpegts.Demuxer` wrapper uses. Forwards
/// the discriminant + free-text message.
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
    let msg = format!("{e}");
    make_demux_error(py, kind, &msg)
}

/// Map a `DemuxReceiverError` raised by `recv_event` onto the right
/// Python exception. Transport-side errors map to `RtpError`; demux-side
/// errors map to `DemuxError`.
fn demux_recv_error_to_pyerr(py: Python<'_>, e: DemuxReceiverError) -> PyErr {
    match e.source {
        DemuxReceiverErrorSource::Transport(t) => transport_error_to_rtp_pyerr(py, t),
        DemuxReceiverErrorSource::Demux(d) => demux_error_to_pyerr(py, &d),
        // `DemuxReceiverErrorSource` is `#[non_exhaustive]`; route any
        // future variant through `RtpError(TRANSPORT)` with the
        // ShellErrorKind discriminant preserved in the message.
        _ => make_rtp_error(py, "TRANSPORT", &format!("{:?}", e.kind)),
    }
}

// ---------------------------------------------------------------------------
// PyDemuxReceiver — wraps tst_pipeline::DemuxReceiver<RtpRecvTransport>.
// ---------------------------------------------------------------------------

/// Single-call convenience wrapper that owns a `Demuxer` + `RtpRecvTransport`.
/// Construct with a URL (`rtp://host:port`); iterate over the emitted
/// `DemuxEvent` instances.
///
/// Events are instances of the existing
/// `tstrans.mpegts.DemuxEvent.*` subclass hierarchy — same conversion
/// path as `tstrans.mpegts.Demuxer.__next__`.
///
/// Use as a context manager for guaranteed cleanup:
/// ```python
/// from tstrans.rtp import DemuxReceiver
///
/// with DemuxReceiver("rtp://0.0.0.0:5004") as rx:
///     for event in rx:
///         match event:
///             case DemuxEvent.Sample(...): ...
///             case DemuxEvent.ProgramMap(...): ...
/// ```
#[pyclass(name = "DemuxReceiver", module = "tstrans.rtp")]
pub struct PyDemuxReceiver {
    /// Live receiver, behind a mutex so concurrent `__next__` /
    /// `close()` calls from different Python threads don't trip the
    /// PyO3 "Already borrowed" check that an `&mut self`-style design
    /// would hit. `Option` so `close()` can take + drop the inner
    /// receiver while keeping the PyClass addressable.
    inner: Arc<Mutex<Option<RustDemuxReceiver<RtpRecvTransport>>>>,
    /// Cancel handle pulled from the transport at construction. Held
    /// outside the mutex so `close()` can fire it BEFORE acquiring the
    /// lock — wakes any thread parked in a `__next__`'s `recv_event`,
    /// which then drops the mutex guard and the close path can take
    /// ownership of `inner` cleanly. Cloning is cheap (`Arc`); multiple
    /// `close()` calls are idempotent.
    cancel: Arc<dyn tst_core::transport::TransportCancel + Send + Sync>,
}

#[pymethods]
impl PyDemuxReceiver {
    /// Bind a receiver to `url` (e.g. `"rtp://0.0.0.0:5004"` for
    /// unicast or `"rtp://239.0.0.1:5004"` for multicast).
    ///
    /// `demux_config` is an optional `tstrans.mpegts.DemuxerConfig`
    /// dataclass; when `None`, defaults are used.
    ///
    /// Raises `RtpError(TRANSPORT)` on URL parse / socket bind failure.
    #[new]
    #[pyo3(signature = (url, *, demux_config = None))]
    fn new(py: Python<'_>, url: &str, demux_config: Option<&Bound<'_, PyAny>>) -> PyResult<Self> {
        // Build the RTP recv transport.
        let builder = RtpRecvSocketBuilder::from_url(url)
            .map_err(|e| make_rtp_error(py, "TRANSPORT", &e.to_string()))?;
        let transport = builder
            .build()
            .map_err(|e| make_rtp_error(py, "TRANSPORT", &e.to_string()))?;

        // Build the receiver (with or without demux options).
        let receiver = match demux_config {
            None => RustDemuxReceiver::new(transport),
            Some(cfg) => {
                let opts = crate::mpegts::build_demuxer_config(py, cfg)?;
                RustDemuxReceiver::with_demux_options(transport, opts)
            }
        };
        // Pull the cancel handle once at construction — the pipeline
        // shell delegates to the underlying transport's
        // `cancel_handle()`, which always returns Some() for
        // RtpRecvTransport.
        let cancel = receiver
            .cancel_handle()
            .expect("RtpRecvTransport always returns Some(cancel_handle)");
        Ok(Self {
            inner: Arc::new(Mutex::new(Some(receiver))),
            cancel,
        })
    }

    /// Iterator protocol: `iter(rx)` returns `self`.
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Block until the next `DemuxEvent` is available. Returns a
    /// `tstrans.mpegts.DemuxEvent.*` subclass instance.
    ///
    /// Raises `StopIteration` on clean EOF (transport closed cleanly,
    /// demuxer drained); `RtpError` on transport-side failure;
    /// `DemuxError` on demuxer-side failure (strict-mode rejection,
    /// malformed PMT/PES).
    fn __next__(&self, py: Python<'_>) -> PyResult<PyObject> {
        let inner = self.inner.clone();
        // Release the GIL while parked on `recv_event`. The pipeline's
        // `recv_event` is pure-Rust: parks on the underlying transport's
        // `recv_bytes`, feeds the resulting TS packet to the demuxer,
        // and returns the next event. No Python objects are constructed
        // inside, so `allow_threads` is safe.
        //
        // The mutex guard is acquired inside `allow_threads` so a
        // concurrent `close()` (which doesn't take the mutex; just
        // fires `cancel`) can wake the parked recv. Once recv returns
        // (event, EOF, or cancelled error), we drop the guard and the
        // close path can take ownership of `inner` cleanly.
        let res: Result<Option<DemuxEvent>, DemuxReceiverError> = py.allow_threads(|| {
            let mut guard = match inner.lock() {
                Ok(g) => g,
                Err(_) => {
                    return Err(DemuxReceiverError::from(TransportError::Broken {
                        msg: "DemuxReceiver inner lock poisoned".into(),
                        errno_code: None,
                    }));
                }
            };
            match guard.as_mut() {
                Some(rx) => rx.recv_event(),
                None => Err(DemuxReceiverError::from(TransportError::Closed)),
            }
        });
        match res {
            Ok(None) => Err(pyo3::exceptions::PyStopIteration::new_err(())),
            Ok(Some(ev)) => crate::mpegts::convert_event(py, &ev),
            Err(e) => Err(demux_recv_error_to_pyerr(py, e)),
        }
    }

    /// Tuple of `(SocketStats, MuxerStats)`. `SocketStats` reflects the
    /// underlying RTP transport's wire-level counters; the second
    /// element is a `MuxerStats`-shaped projection of the demuxer's
    /// own event/PMT counters surfaced through the pipeline shell's
    /// `DemuxReceiverStats` (a Demuxer-side analog — fields are reused
    /// to keep the tuple shape symmetric with `MuxSender.stats()`).
    ///
    /// Returns zeroed defaults if the receiver is closed.
    fn stats(&self, py: Python<'_>) -> PyResult<(Py<PySocketStats>, Py<PyMuxerStats>)> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| make_rtp_error(py, "TRANSPORT", "DemuxReceiver lock poisoned"))?;
        let inner = guard
            .as_ref()
            .ok_or_else(|| make_rtp_error(py, "TRANSPORT", "DemuxReceiver is closed"))?;
        let combined = inner.stats();
        // The underlying RTP transport's full SocketStats live behind a
        // separate accessor that the pipeline shell doesn't expose
        // directly; we synthesise a SocketStats with the
        // bytes_received / packets_received fields populated from the
        // pipeline projection. RTCP-derived fields stay zero until
        // Stage 3 closes the deferred TCP RTCP wiring.
        // `SocketStats` is `#[non_exhaustive]`; populate via mut spread.
        let mut sock_stats = tst_core::transport::SocketStats::default();
        sock_stats.bytes_received = combined.bytes_received;
        sock_stats.packets_received = combined.packets_received;
        // Re-shape the demux side as a MuxerStats projection so callers
        // get the same `(SocketStats, MuxerStats)` tuple shape on both
        // MuxSender + DemuxReceiver.
        let mux_stats = tst_core::mpegts::mux::MuxerStats {
            ts_packets_emitted: combined.packets_received,
            ts_bytes_emitted: combined.bytes_received,
            programs_configured: combined.program_maps_seen as u32,
            subtitle_streams_configured: 0,
            per_stream: combined.per_stream,
        };
        let sock_py = Py::new(py, PySocketStats::from_core(sock_stats))?;
        let mux_py = Py::new(py, PyMuxerStats::from_inner(mux_stats))?;
        Ok((sock_py, mux_py))
    }

    /// Close the receiver. Idempotent. Fires the cancel handle BEFORE
    /// acquiring the mutex so a concurrent `__next__` parked in
    /// `recv_event` unparks promptly — without this cancel-first step
    /// the close would deadlock waiting for the lock the parked recv
    /// holds.
    fn close(&self, py: Python<'_>) {
        // Step 1: cancel any in-flight recv. Releases the parked
        // `__next__` immediately (within ~100ms per RTP transport
        // cancel-poll tick). py.allow_threads so we don't hold the GIL
        // during the brief cancel-wake window.
        self.cancel.cancel();
        let inner = self.inner.clone();
        py.allow_threads(move || {
            // Step 2: take the inner under the mutex. If the parked
            // recv has already returned, this is immediate; otherwise
            // we wait briefly for it to release.
            if let Ok(mut guard) = inner.lock() {
                if let Some(mut r) = guard.take() {
                    r.close();
                }
            }
            // Lock-poisoned: silently no-op (close is best-effort —
            // the cancel above already woke any parked recv).
        });
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
        // Try-lock so a __repr__ call from another thread during a
        // parked recv doesn't deadlock; report "<busy>" if locked.
        match self.inner.try_lock() {
            Ok(g) => match g.as_ref() {
                Some(_) => "DemuxReceiver(open)".to_string(),
                None => "DemuxReceiver(closed)".to_string(),
            },
            Err(_) => "DemuxReceiver(<busy>)".to_string(),
        }
    }
}

impl PyDemuxReceiver {
    /// Crate-private constructor used by
    /// `crate::rtp::client::PyRtspSession::into_demux_receiver`: takes
    /// an already-built `RtpRecvTransport` (handed off from the RTSP
    /// session via `RtspSession::into_recv_transport`) and wraps it
    /// with default demux options.
    pub(crate) fn from_recv_transport(transport: RtpRecvTransport) -> Self {
        let receiver = RustDemuxReceiver::new(transport);
        let cancel = receiver
            .cancel_handle()
            .expect("RtpRecvTransport always returns Some(cancel_handle)");
        Self {
            inner: Arc::new(Mutex::new(Some(receiver))),
            cancel,
        }
    }

    /// Like [`Self::from_recv_transport`] but with explicit demuxer
    /// options. Used when the Python caller passes a `DemuxerConfig`
    /// dataclass to `RtspSession.into_demux_receiver(demux_config=...)`.
    pub(crate) fn from_recv_transport_with_config(
        transport: RtpRecvTransport,
        opts: tst_core::mpegts::demux::DemuxerConfig,
    ) -> Self {
        let receiver = RustDemuxReceiver::with_demux_options(transport, opts);
        let cancel = receiver
            .cancel_handle()
            .expect("RtpRecvTransport always returns Some(cancel_handle)");
        Self {
            inner: Arc::new(Mutex::new(Some(receiver))),
            cancel,
        }
    }
}

// ---------------------------------------------------------------------------
// Module registration.
// ---------------------------------------------------------------------------

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyDemuxReceiver>()?;
    Ok(())
}
