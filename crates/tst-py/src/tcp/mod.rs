//! Python bindings for tst-tcp (`tstrans.tcp`). Gated on `feature = "tcp"`.
//!
//! Populated by Plan A5b Wave B (Tasks 6-9). Mirrors the udp/ module
//! structure but wraps a single dual-trait `TcpTransport` that implements
//! both `Transport` (sender) and `RecvTransport` (receiver).
//!
//! Key differences from udp/:
//! - ONE `Transport` PyClass covers both send and recv. The Rust
//!   `TcpTransport` is a bytestream handle — the caller decides whether to
//!   use it as a sender, a receiver, or both. The Python binding does NOT
//!   enforce mutual exclusion (Rust doesn't either).
//! - `Listener` wraps `tst_tcp::TcpListener`; `accept_blocking()` returns
//!   a `Transport`.
//! - TLS (`tcps://`) requires the `tls` cargo feature on tst-tcp. The tst-py
//!   `tcp` feature builds tst-tcp WITHOUT its `tls` sub-feature, so any
//!   `tcps://` URL triggers `TcpError(kind=TLS_DISABLED)` at build() time.
//!   `TlsConfig` and `ClientCert` are exposed as pure-Python dataclasses for
//!   forward-compatibility; the builder accepts them but cannot honour them.
//!
//! GIL boundaries:
//! - `send`, `recv`, `build()` (both builders), `accept_blocking` ->
//!   `py.allow_threads(...)` so concurrent Python threads remain live.
//! - `close`, `stats`, `local_port`, `peer_addr` -> fast read-only ops;
//!   no GIL release needed.
//!
//! Bytes-like extraction in `Transport.send(payload)` follows the abi3-py310
//! two-path pattern from udp/mod.rs: fast zero-copy `&[u8]` for `bytes`,
//! fallback through Python `bytes()` builtin for `bytearray`/`memoryview`.
//!
//! Error mapping: `tst_tcp::error::TcpError` -> `tstrans.exceptions.TcpError`
//! with `.kind` set to one of the eight `TcpErrorKind` variants. The 28th
//! bash ratchet `scripts/check-py-tcp-error-mapping-coverage.sh` enforces
//! that every `TcpErrorKind` variant has at least one literal
//! `make_tcp_error(py, "<VARIANT>", ...)` call site in this crate.

#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use std::sync::Mutex;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use tst_core::transport::{RecvTransport, SocketStats, Transport, TransportError};
use tst_tcp::error::{TcpError, TcpErrorKind};
use tst_tcp::{TcpListener, TcpStats, TcpTransport};

use crate::errors::make_tcp_error;

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/// Map a `tst_tcp::error::TcpError` to a `tstrans.exceptions.TcpError` PyErr.
///
/// `TcpErrorKind` is `#[non_exhaustive]`; the wildcard arm routes any
/// unknown future variant to `IO` so this fn never panics on a Rust-side
/// enum addition. The bash ratchet will surface the omission in CI.
///
/// Each of the eight `TcpErrorKind` variants gets a literal call site below
/// so the `check-py-tcp-error-mapping-coverage.sh` ratchet stays green.
fn map_tcp_error_kind(py: Python<'_>, e: TcpError) -> PyErr {
    let msg = e.to_string();
    match e.kind() {
        TcpErrorKind::Url => make_tcp_error(py, "URL", &msg),
        TcpErrorKind::Io => make_tcp_error(py, "IO", &msg),
        TcpErrorKind::PayloadTooLarge => make_tcp_error(py, "PAYLOAD_TOO_LARGE", &msg),
        TcpErrorKind::Closed => make_tcp_error(py, "CLOSED", &msg),
        TcpErrorKind::ConnectTimeout => make_tcp_error(py, "CONNECT_TIMEOUT", &msg),
        TcpErrorKind::InvalidConfig => make_tcp_error(py, "INVALID_CONFIG", &msg),
        TcpErrorKind::Tls => make_tcp_error(py, "TLS", &msg),
        TcpErrorKind::TlsDisabled => make_tcp_error(py, "TLS_DISABLED", &msg),
        // Wildcard for #[non_exhaustive] additions not yet mapped.
        _ => make_tcp_error(py, "IO", &msg),
    }
}

/// Map a `tst_tcp::url::TcpUrlError` to a `tstrans.exceptions.TcpError`
/// with `kind=URL`.
fn map_tcp_url_error(py: Python<'_>, e: tst_tcp::url::TcpUrlError) -> PyErr {
    make_tcp_error(py, "URL", &e.to_string())
}

/// Classifies a `TransportError` for mapping to TcpErrorKind without
/// requiring a `Python<'_>` token. Returned from inside `allow_threads`
/// closures; the caller converts to `PyErr` after the GIL is re-acquired.
enum TcpTransportErr {
    Closed,
    PayloadTooLarge { len: usize, max: usize },
    Io(String),
    Mutex,
    Closed2,
}

impl From<TransportError> for TcpTransportErr {
    fn from(e: TransportError) -> Self {
        match e {
            TransportError::Closed | TransportError::ExplicitClose => Self::Closed,
            TransportError::TooLarge { len, max } => Self::PayloadTooLarge { len, max },
            other => Self::Io(other.to_string()),
        }
    }
}

impl TcpTransportErr {
    fn into_pyerr(self, py: Python<'_>) -> PyErr {
        match self {
            Self::Closed | Self::Closed2 => {
                make_tcp_error(py, "CLOSED", "transport closed by caller")
            }
            Self::PayloadTooLarge { len, max } => {
                let msg = format!("payload {len} exceeds max {max} bytes per send call");
                make_tcp_error(py, "PAYLOAD_TOO_LARGE", &msg)
            }
            Self::Io(msg) => make_tcp_error(py, "IO", &msg),
            Self::Mutex => PyRuntimeError::new_err("tcp transport mutex poisoned"),
        }
    }
}

// ---------------------------------------------------------------------------
// PyTcpStats — frozen mirror of TcpStats
// ---------------------------------------------------------------------------

/// Cumulative stats snapshot for a TCP transport handle.
///
/// Returned by `Transport.stats()`. Both send and receive counters are
/// populated on the same handle (TCP is full-duplex).
#[pyclass(frozen, get_all, name = "SocketStats", module = "tstrans.tcp")]
pub(crate) struct PyTcpStats {
    /// Bytes successfully sent.
    pub bytes_sent: u64,
    /// Bytes successfully received.
    pub bytes_received: u64,
    /// Number of successful send calls.
    pub send_calls: u64,
    /// Number of successful recv calls.
    pub recv_calls: u64,
    /// Send-side I/O errors.
    pub send_errors: u64,
    /// Receive-side I/O errors.
    pub recv_errors: u64,
}

impl From<TcpStats> for PyTcpStats {
    fn from(s: TcpStats) -> Self {
        Self {
            bytes_sent: s.bytes_sent,
            bytes_received: s.bytes_received,
            send_calls: s.send_calls,
            recv_calls: s.recv_calls,
            send_errors: s.send_errors,
            recv_errors: s.recv_errors,
        }
    }
}

#[allow(dead_code)]
impl PyTcpStats {
    fn from_core(s: SocketStats) -> Self {
        Self {
            bytes_sent: s.bytes_sent,
            bytes_received: s.bytes_received,
            send_calls: s.packets_sent,
            recv_calls: s.packets_received,
            send_errors: s.packets_dropped_send,
            recv_errors: s.packets_dropped_recv,
        }
    }
}

#[pymethods]
impl PyTcpStats {
    fn __repr__(&self) -> String {
        format!(
            "SocketStats(bytes_sent={}, bytes_received={}, \
             send_calls={}, recv_calls={})",
            self.bytes_sent, self.bytes_received, self.send_calls, self.recv_calls,
        )
    }
}

// ---------------------------------------------------------------------------
// PyTcpTransport -- wraps tst_tcp::TcpTransport (dual-trait send+recv)
// ---------------------------------------------------------------------------

/// TCP transport -- wraps `tst_tcp::TcpTransport`.
///
/// Implements BOTH the sender and receiver roles on a single handle.
/// The connection is established by the builder (`Transport.builder()`) or
/// returned by `Listener.accept_blocking()`. Which role the caller uses
/// (send vs recv vs both) is the caller's choice -- TCP is full-duplex.
///
/// Construct via:
/// ```python
/// transport = tcp.Transport.builder().url("tcp://host:port").build()
/// ```
///
/// GIL is released during `send` and `recv` so other Python threads
/// remain live while the kernel I/O blocks.
#[pyclass(name = "Transport", module = "tstrans.tcp")]
pub(crate) struct PyTcpTransport {
    inner: Mutex<Option<TcpTransport>>,
    /// Per-recv scratch buffer. Resized to `max_payload()` bytes at
    /// construction time. Unused by send but allocated so recv doesn't need
    /// a per-call allocation.
    #[allow(dead_code)]
    scratch: Vec<u8>,
}

#[pymethods]
impl PyTcpTransport {
    /// Return a builder for configuring and constructing a `Transport`.
    #[staticmethod]
    fn builder() -> PyTcpTransportBuilder {
        PyTcpTransportBuilder::default()
    }

    /// Send a payload over the TCP connection. Accepts any bytes-like object:
    /// `bytes`, `bytearray`, `memoryview`, or any buffer-protocol object.
    ///
    /// Raises `TcpError(kind=PAYLOAD_TOO_LARGE)` if `len(payload)` exceeds
    /// the configured `pkt_size` (default 64 KiB).
    ///
    /// Releases the GIL during the kernel send.
    fn send(&self, py: Python<'_>, payload: &Bound<'_, PyAny>) -> PyResult<()> {
        // Coerce to owned bytes before crossing the allow_threads boundary.
        // `Python<'_>` is `!Send` so nothing involving it can enter the closure.
        let owned: Vec<u8> = if let Ok(slice) = payload.extract::<&[u8]>() {
            slice.to_vec()
        } else {
            // Fallback: bytearray / memoryview / etc. -- coerce through Python
            // `bytes()` builtin (one C copy). Required under abi3-py310 since
            // PyBuffer is gated on not(Py_LIMITED_API) in PyO3 0.22.
            let coerced: Bound<'_, PyBytes> = py
                .import_bound("builtins")?
                .getattr(intern!(py, "bytes"))?
                .call1((payload,))?
                .downcast_into::<PyBytes>()?;
            coerced.as_bytes().to_vec()
        };
        // Two-step error handling: inside allow_threads we return a Send-safe
        // TcpTransportErr; after the GIL is re-acquired we convert to PyErr.
        let result: Result<(), TcpTransportErr> = py.allow_threads(|| {
            let mut guard = self.inner.lock().map_err(|_| TcpTransportErr::Mutex)?;
            let inner = guard.as_mut().ok_or(TcpTransportErr::Closed2)?;
            inner.send_bytes(&owned).map_err(TcpTransportErr::from)
        });
        result.map_err(|e| e.into_pyerr(py))
    }

    /// Receive bytes from the TCP connection into a pre-allocated `bytearray`.
    ///
    /// Returns the number of bytes written into `buf`. The caller is
    /// responsible for sizing `buf` to at least the expected chunk size
    /// (`Transport.builder().pkt_size(N)` controls the sender cap, which
    /// defaults to 64 KiB).
    ///
    /// Raises `TcpError(kind=CLOSED)` if the transport has been closed.
    /// Raises `TcpError(kind=IO)` on connection errors (including peer close).
    ///
    /// Releases the GIL while blocking on kernel recv.
    fn recv(&self, py: Python<'_>, buf: &Bound<'_, pyo3::types::PyByteArray>) -> PyResult<usize> {
        // We need an owned buffer to cross the allow_threads boundary --
        // `PyByteArray` is a Python object and is !Send.
        let buf_len = buf.len();
        let mut owned = vec![0u8; buf_len];
        // Two-step: compute inside allow_threads, map error after.
        let result: Result<usize, TcpTransportErr> = py.allow_threads(|| {
            let mut guard = self.inner.lock().map_err(|_| TcpTransportErr::Mutex)?;
            let inner = guard.as_mut().ok_or(TcpTransportErr::Closed2)?;
            inner
                .recv_bytes(owned.as_mut_slice())
                .map_err(TcpTransportErr::from)
        });
        let n = result.map_err(|e| e.into_pyerr(py))?;
        // Copy the received bytes back into the Python bytearray.
        // Safety: we hold the GIL; no other Python thread can resize or alias
        // this bytearray concurrently (PyO3 0.22 requires `unsafe` for the
        // raw bytes_mut accessor since PyByteArray is a Python-managed object).
        let dest = unsafe { buf.as_bytes_mut() };
        dest[..n].copy_from_slice(&owned[..n]);
        Ok(n)
    }

    /// Peer address as a `"host:port"` string. Returns `""` if the transport
    /// has been closed.
    fn peer_addr(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match guard.as_ref() {
            Some(t) => t.peer().to_string(),
            None => String::new(),
        }
    }

    /// Close the transport. Idempotent -- further `.send()` / `.recv()` calls
    /// raise `TcpError(kind=CLOSED)`.
    fn close(&self) {
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(mut t) = guard.take() {
            Transport::close(&mut t);
        }
    }

    /// Snapshot of wire-level statistics. Counters are cumulative and never
    /// wrap (saturating add).
    fn stats(&self, py: Python<'_>) -> PyResult<Py<PyTcpStats>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("tcp transport mutex poisoned"))?;
        let inner = guard
            .as_ref()
            .ok_or_else(|| make_tcp_error(py, "CLOSED", "transport closed"))?;
        Py::new(py, PyTcpStats::from(inner.stats()))
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __exit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        self.close();
        false
    }

    fn __repr__(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match guard.as_ref() {
            Some(t) => format!("Transport(peer={})", t.peer()),
            None => "Transport(closed)".to_string(),
        }
    }
}

/// Construct a `PyTcpTransport` from an already-connected `TcpTransport`.
/// Used internally by `PyTcpListenerBuilder::build()` / `accept_blocking`.
fn make_py_tcp_transport(t: TcpTransport) -> PyTcpTransport {
    let scratch_len = Transport::max_payload(&t).max(65_536);
    PyTcpTransport {
        inner: Mutex::new(Some(t)),
        scratch: vec![0u8; scratch_len],
    }
}

// ---------------------------------------------------------------------------
// PyTcpTransportBuilder -- builder for PyTcpTransport
// ---------------------------------------------------------------------------

/// Builder for `Transport`. Chain setter calls, then call `.build()`.
///
/// Example:
/// ```python
/// transport = tcp.Transport.builder() \
///     .url("tcp://192.168.1.100:5001") \
///     .nodelay(True) \
///     .connect_timeout_ms(5000) \
///     .build()
/// ```
#[pyclass(name = "TransportBuilder", module = "tstrans.tcp")]
#[derive(Default)]
pub(crate) struct PyTcpTransportBuilder {
    url: Option<String>,
    nodelay: Option<bool>,
    keepalive_ms: Option<u64>,
    rcvbuf: Option<usize>,
    sndbuf: Option<usize>,
    pkt_size: Option<usize>,
    connect_timeout_ms: Option<u64>,
}

#[pymethods]
impl PyTcpTransportBuilder {
    /// Set the destination URL. Required. Must be `tcp://host:port` or
    /// `tcps://host:port` (TLS -- returns `TcpError(kind=TLS_DISABLED)` at
    /// build time unless tst-tcp was compiled with `--features tls`).
    fn url<'py>(mut slf: PyRefMut<'py, Self>, s: &str) -> PyRefMut<'py, Self> {
        slf.url = Some(s.to_string());
        slf
    }

    /// Enable or disable TCP_NODELAY (Nagle's algorithm).
    ///
    /// `True` is typically preferred for low-latency streaming.
    fn nodelay(mut slf: PyRefMut<'_, Self>, v: bool) -> PyRefMut<'_, Self> {
        slf.nodelay = Some(v);
        slf
    }

    /// Set SO_KEEPALIVE idle timeout in milliseconds.
    fn keepalive_ms(mut slf: PyRefMut<'_, Self>, v: u64) -> PyRefMut<'_, Self> {
        slf.keepalive_ms = Some(v);
        slf
    }

    /// `SO_RCVBUF` size in bytes.
    fn rcvbuf(mut slf: PyRefMut<'_, Self>, v: usize) -> PyRefMut<'_, Self> {
        slf.rcvbuf = Some(v);
        slf
    }

    /// `SO_SNDBUF` size in bytes.
    fn sndbuf(mut slf: PyRefMut<'_, Self>, v: usize) -> PyRefMut<'_, Self> {
        slf.sndbuf = Some(v);
        slf
    }

    /// Maximum payload chunk size per `send()` call (default 64 KiB).
    fn pkt_size(mut slf: PyRefMut<'_, Self>, v: usize) -> PyRefMut<'_, Self> {
        slf.pkt_size = Some(v);
        slf
    }

    /// Connection timeout in milliseconds (default 10 000 ms).
    fn connect_timeout_ms(mut slf: PyRefMut<'_, Self>, v: u64) -> PyRefMut<'_, Self> {
        slf.connect_timeout_ms = Some(v);
        slf
    }

    /// Build the `Transport` by establishing a TCP connection.
    ///
    /// Raises `TcpError(kind=URL)` for a malformed URL.
    /// Raises `TcpError(kind=CONNECT_TIMEOUT)` or `TcpError(kind=IO)` on
    /// connection failures.
    /// Raises `TcpError(kind=TLS_DISABLED)` if `tcps://` was used but tst-tcp
    /// was not built with the `tls` feature.
    fn build(&self, py: Python<'_>) -> PyResult<PyTcpTransport> {
        let url_str = self
            .url
            .as_ref()
            .ok_or_else(|| PyValueError::new_err("url(...) is required before build()"))?
            .clone();

        let nodelay = self.nodelay;
        let keepalive_ms = self.keepalive_ms;
        let rcvbuf = self.rcvbuf;
        let sndbuf = self.sndbuf;
        let pkt_size = self.pkt_size;
        let connect_timeout_ms = self.connect_timeout_ms;

        let t = py.allow_threads(|| -> Result<TcpTransport, TcpError> {
            let mut b = tst_tcp::TcpTransportBuilder::from_url(&url_str).map_err(TcpError::Url)?;
            if let Some(v) = nodelay {
                b.nodelay(v);
            }
            if let Some(ms) = keepalive_ms {
                b.keepalive(std::time::Duration::from_millis(ms));
            }
            if let Some(v) = rcvbuf {
                b.rcvbuf(v);
            }
            if let Some(v) = sndbuf {
                b.sndbuf(v);
            }
            if let Some(v) = pkt_size {
                b.pkt_size(v);
            }
            if let Some(ms) = connect_timeout_ms {
                b.connect_timeout(std::time::Duration::from_millis(ms));
            }
            b.build()
        });

        match t {
            Ok(transport) => Ok(make_py_tcp_transport(transport)),
            Err(TcpError::Url(e)) => Err(map_tcp_url_error(py, e)),
            Err(e) => Err(map_tcp_error_kind(py, e)),
        }
    }

    fn __repr__(&self) -> String {
        format!("TransportBuilder(url={:?})", self.url)
    }
}

// ---------------------------------------------------------------------------
// PyTcpListener -- wraps tst_tcp::TcpListener
// ---------------------------------------------------------------------------

/// TCP listener -- wraps `tst_tcp::TcpListener`.
///
/// Construct via `Listener.builder().bind("host:port").build()`, then call
/// `accept_blocking()` to receive a `Transport` per inbound connection.
///
/// Binding to port 0 lets the kernel pick a free ephemeral port; read it
/// back via `local_port()` before accepting.
///
/// GIL is released during `accept_blocking` so other Python threads
/// remain live while waiting for a connection.
#[pyclass(name = "Listener", module = "tstrans.tcp")]
pub(crate) struct PyTcpListener {
    inner: Mutex<Option<TcpListener>>,
}

#[pymethods]
impl PyTcpListener {
    /// Return a builder for configuring and constructing a `Listener`.
    #[staticmethod]
    fn builder() -> PyTcpListenerBuilder {
        PyTcpListenerBuilder::default()
    }

    /// Block until a new inbound connection arrives. Returns a `Transport`
    /// wrapping the accepted connection.
    ///
    /// Raises `TcpError(kind=IO)` on accept failure.
    /// Raises `TcpError(kind=CLOSED)` if the listener has been closed.
    ///
    /// Releases the GIL while waiting.
    fn accept_blocking(&self, py: Python<'_>) -> PyResult<PyTcpTransport> {
        // Two-step: accept inside allow_threads (returns Result<TcpTransport, TcpError>
        // where TcpError is Send), then map to PyErr after re-acquiring the GIL.
        let result: Result<TcpTransport, TcpError> = py.allow_threads(|| {
            let guard = self
                .inner
                .lock()
                .map_err(|_| TcpError::InvalidConfig("listener mutex poisoned".into()))?;
            let listener = guard.as_ref().ok_or(TcpError::Closed)?;
            listener.accept_blocking()
        });
        let t = result.map_err(|e| map_tcp_error_kind(py, e))?;
        Ok(make_py_tcp_transport(t))
    }

    /// Local bound port. Non-zero after successful `build()`.
    ///
    /// Use this to discover the ephemeral port when `.bind("127.0.0.1:0")`
    /// was used.
    fn local_port(&self, py: Python<'_>) -> PyResult<u16> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("tcp listener mutex poisoned"))?;
        let listener = guard
            .as_ref()
            .ok_or_else(|| make_tcp_error(py, "CLOSED", "listener closed"))?;
        listener
            .local_addr()
            .map(|a| a.port())
            .map_err(|e| make_tcp_error(py, "IO", &e.to_string()))
    }

    /// Close the listener. Idempotent -- further `accept_blocking()` calls
    /// raise `TcpError(kind=CLOSED)`.
    fn close(&self) {
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        guard.take(); // drop the std::net::TcpListener
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __exit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> bool {
        self.close();
        false
    }

    fn __repr__(&self) -> String {
        let guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match guard.as_ref() {
            Some(_) => "Listener(open)".to_string(),
            None => "Listener(closed)".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// PyTcpListenerBuilder -- builder for PyTcpListener
// ---------------------------------------------------------------------------

/// Builder for `Listener`. Chain setter calls, then call `.build()`.
///
/// Example:
/// ```python
/// listener = tcp.Listener.builder() \
///     .bind("127.0.0.1:0") \
///     .nodelay(True) \
///     .build()
/// port = listener.local_port()
/// ```
#[pyclass(name = "ListenerBuilder", module = "tstrans.tcp")]
#[derive(Default)]
pub(crate) struct PyTcpListenerBuilder {
    bind_addr: Option<String>,
    nodelay: Option<bool>,
    rcvbuf: Option<usize>,
    sndbuf: Option<usize>,
    pkt_size: Option<usize>,
}

#[pymethods]
impl PyTcpListenerBuilder {
    /// Set the bind address as `"host:port"` (e.g. `"127.0.0.1:0"` or
    /// `"0.0.0.0:5001"`). Required. Port 0 requests an ephemeral port.
    fn bind<'py>(mut slf: PyRefMut<'py, Self>, s: &str) -> PyRefMut<'py, Self> {
        slf.bind_addr = Some(s.to_string());
        slf
    }

    /// Enable or disable TCP_NODELAY for accepted connections.
    fn nodelay(mut slf: PyRefMut<'_, Self>, v: bool) -> PyRefMut<'_, Self> {
        slf.nodelay = Some(v);
        slf
    }

    /// `SO_RCVBUF` size in bytes for accepted connections.
    fn rcvbuf(mut slf: PyRefMut<'_, Self>, v: usize) -> PyRefMut<'_, Self> {
        slf.rcvbuf = Some(v);
        slf
    }

    /// `SO_SNDBUF` size in bytes for accepted connections.
    fn sndbuf(mut slf: PyRefMut<'_, Self>, v: usize) -> PyRefMut<'_, Self> {
        slf.sndbuf = Some(v);
        slf
    }

    /// Maximum payload chunk size for accepted connections (default 64 KiB).
    fn pkt_size(mut slf: PyRefMut<'_, Self>, v: usize) -> PyRefMut<'_, Self> {
        slf.pkt_size = Some(v);
        slf
    }

    /// Bind the listener socket.
    ///
    /// Raises `ValueError` if `bind(...)` was not called.
    /// Raises `TcpError(kind=IO)` if the port is in use or permissions are
    /// insufficient.
    fn build(&self, py: Python<'_>) -> PyResult<PyTcpListener> {
        let addr_str = self
            .bind_addr
            .as_ref()
            .ok_or_else(|| PyValueError::new_err("bind(...) is required before build()"))?
            .clone();

        let nodelay = self.nodelay;
        let rcvbuf = self.rcvbuf;
        let sndbuf = self.sndbuf;
        let pkt_size = self.pkt_size;

        let listener = py.allow_threads(|| -> Result<TcpListener, TcpError> {
            // Build a listener URL: tcp://addr:port?listen=1
            let listen_url = format!("tcp://{}?listen=1", addr_str);
            let mut b =
                tst_tcp::TcpListenerBuilder::from_url(&listen_url).map_err(TcpError::Url)?;
            if let Some(v) = nodelay {
                b.nodelay(v);
            }
            if let Some(v) = rcvbuf {
                b.rcvbuf(v);
            }
            if let Some(v) = sndbuf {
                b.sndbuf(v);
            }
            if let Some(v) = pkt_size {
                b.pkt_size(v);
            }
            b.build()
        });

        match listener {
            Ok(l) => Ok(PyTcpListener {
                inner: Mutex::new(Some(l)),
            }),
            Err(TcpError::Url(e)) => Err(map_tcp_url_error(py, e)),
            Err(e) => Err(map_tcp_error_kind(py, e)),
        }
    }

    fn __repr__(&self) -> String {
        format!("ListenerBuilder(bind={:?})", self.bind_addr)
    }
}

// ---------------------------------------------------------------------------
// TlsConfig / ClientCert -- forward-compat dataclasses for tcps:// callers
// ---------------------------------------------------------------------------

/// TLS configuration dataclass for `tcps://` transports.
///
/// **Note:** The `tcp` cargo feature builds tst-tcp WITHOUT its `tls`
/// sub-feature. Passing a `TlsConfig` to a builder is accepted but any
/// `tcps://` URL will still raise `TcpError(kind=TLS_DISABLED)` at
/// `build()` time. These classes exist for forward compatibility and for
/// code written against a full-TLS wheel.
#[pyclass(name = "TlsConfig", module = "tstrans.tcp", frozen)]
#[derive(Clone)]
pub(crate) struct PyTlsConfig {
    /// PEM-encoded CA certificate bundle. Used for server certificate
    /// verification when connecting to `tcps://` endpoints.
    pub ca_pem: Vec<u8>,
    /// If `True` (default), the server hostname is verified against the
    /// certificate CN / SAN fields.
    pub verify_hostname: bool,
    /// Optional client certificate for mutual TLS authentication.
    pub client_cert: Option<PyClientCert>,
}

#[pymethods]
impl PyTlsConfig {
    #[new]
    #[pyo3(signature = (ca_pem = None, *, verify_hostname = true, client_cert = None))]
    fn new(
        ca_pem: Option<&[u8]>,
        verify_hostname: bool,
        client_cert: Option<PyClientCert>,
    ) -> Self {
        Self {
            ca_pem: ca_pem.unwrap_or_default().to_vec(),
            verify_hostname,
            client_cert,
        }
    }

    // Explicit getters — `ca_pem` returns `bytes` (a `get_all` auto-getter
    // would expose the `Vec<u8>` as `list[int]`, mismatching the stub + tests).
    #[getter]
    fn ca_pem<'py>(&self, py: Python<'py>) -> pyo3::Bound<'py, pyo3::types::PyBytes> {
        pyo3::types::PyBytes::new_bound(py, &self.ca_pem)
    }
    #[getter]
    fn verify_hostname(&self) -> bool {
        self.verify_hostname
    }
    #[getter]
    fn client_cert(&self) -> Option<PyClientCert> {
        self.client_cert.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "TlsConfig(ca_pem=<{} bytes>, verify_hostname={})",
            self.ca_pem.len(),
            self.verify_hostname
        )
    }
}

/// Client certificate for mutual TLS authentication.
#[pyclass(name = "ClientCert", module = "tstrans.tcp", frozen)]
#[derive(Clone)]
pub(crate) struct PyClientCert {
    /// PEM-encoded client certificate.
    pub cert_pem: Vec<u8>,
    /// PEM-encoded private key. Treat as sensitive; avoid logging.
    pub key_pem: Vec<u8>,
}

#[pymethods]
impl PyClientCert {
    #[new]
    fn new(cert_pem: &[u8], key_pem: &[u8]) -> Self {
        Self {
            cert_pem: cert_pem.to_vec(),
            key_pem: key_pem.to_vec(),
        }
    }

    // Explicit getters returning `bytes` (not the `list[int]` a `get_all`
    // auto-getter would expose for a `Vec<u8>` field).
    #[getter]
    fn cert_pem<'py>(&self, py: Python<'py>) -> pyo3::Bound<'py, pyo3::types::PyBytes> {
        pyo3::types::PyBytes::new_bound(py, &self.cert_pem)
    }
    #[getter]
    fn key_pem<'py>(&self, py: Python<'py>) -> pyo3::Bound<'py, pyo3::types::PyBytes> {
        pyo3::types::PyBytes::new_bound(py, &self.key_pem)
    }

    fn __repr__(&self) -> String {
        format!(
            "ClientCert(cert_pem=<{} bytes>, key_pem=<redacted {}>)",
            self.cert_pem.len(),
            self.key_pem.len()
        )
    }
}

// ---------------------------------------------------------------------------
// Module registration
// ---------------------------------------------------------------------------

pub(crate) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(parent.py(), "tcp")?;
    m.add_class::<PyTcpStats>()?;
    m.add_class::<PyTcpTransport>()?;
    m.add_class::<PyTcpTransportBuilder>()?;
    m.add_class::<PyTcpListener>()?;
    m.add_class::<PyTcpListenerBuilder>()?;
    m.add_class::<PyTlsConfig>()?;
    m.add_class::<PyClientCert>()?;
    parent.add_submodule(&m)?;
    Ok(())
}
