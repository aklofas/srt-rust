//! Plan A5b Wave C T12 — `HlsPublisher` + `HlsPublisherBuilder`.
//!
//! `tstrans.hls.HlsPublisher` wraps `tst_tcp::hls::HlsPublisher`: an
//! outbound-only, segment-aware MPEG-TS sink that writes `.ts` segments +
//! a `playlist.m3u8` to disk and serves them over a built-in HTTP(S)
//! server. It is a concrete `impl Publisher` (NOT bridged through
//! Python) and is registered (in `tstrans/hls.py`) as a virtual subclass
//! of the pure-Python `Publisher` ABC so `isinstance(pub, Publisher)`
//! holds.
//!
//! Lifecycle:
//! - `finish()` consumes the inner publisher (writes the terminal
//!   playlist, tears down the HTTP server). Stored as `Option<...>` +
//!   `take()` on finish; subsequent ops raise `HlsError(FINISHED)`.
//! - `MuxPublisher.with_config_hls(pub, ...)` *also* consumes the inner
//!   via `take_inner()` (moving it into the shell). Either path leaves
//!   the handle closed.
//!
//! GIL: `push_ts` / `cut_segment` / `finish` release the GIL via
//! `py.allow_threads` (the disk + HTTP work is pure Rust). Fast read-only
//! getters (`stats`, `hls_stats`, `local_addr`, `render_playlist`) don't
//! release it.

#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use tst_core::publisher::Publisher;
use tst_tcp::hls::{HlsMode, HlsPublisher, HlsPublisherBuilder};

use crate::hls::config::{PyHlsMode, PyHlsStats};
use crate::hls::publisher_abc::PyPublisherStats;
use crate::hls::{make_hls_error, map_hls_error, map_hls_url_error};

// ---------------------------------------------------------------------------
// PyHlsPublisher — wraps tst_tcp::hls::HlsPublisher
// ---------------------------------------------------------------------------

/// HLS publisher (`tstrans.hls.HlsPublisher`). Segments MPEG-TS to disk +
/// serves a built-in HTTP playlist. Build via `HlsPublisher.builder()`.
///
/// Registered (in `tstrans/hls.py`) as a *virtual* subclass of the
/// pure-Python `Publisher` ABC via `Publisher.register(HlsPublisher)`, so
/// `isinstance(pub, Publisher)` is True without native inheritance. We do
/// NOT make this a native subclass of the ABC: a native base would force
/// every constructor (`build`, `with_config_hls`, `finish_into_publisher`)
/// to return a `PyClassInitializer` and would fight the builder pattern.
/// Virtual registration gives the `isinstance` contract cleanly.
///
/// Either `finish()` or handing it to `MuxPublisher.with_config_hls`
/// consumes the inner publisher; subsequent operations raise
/// `HlsError(FINISHED)`.
#[pyclass(name = "HlsPublisher", module = "tstrans.hls")]
pub(crate) struct PyHlsPublisher {
    /// `Option` so `finish()` / `MuxPublisher.with_config_hls` can move
    /// the inner publisher out while leaving the PyClass addressable.
    /// `Mutex` to allow `&self` methods (push_ts / cut_segment) plus a
    /// `take()` on finish.
    inner: Mutex<Option<HlsPublisher>>,
}

impl PyHlsPublisher {
    /// Wrap an owned Rust `HlsPublisher` (used by
    /// `MuxPublisher.finish_into_publisher`).
    pub(crate) fn from_inner(inner: HlsPublisher) -> Self {
        Self {
            inner: Mutex::new(Some(inner)),
        }
    }

    /// Move the inner publisher out (consumes the handle). Used by
    /// `MuxPublisher.with_config_hls`. Returns `None` if already consumed.
    pub(crate) fn take_inner(&mut self) -> Option<HlsPublisher> {
        self.inner.get_mut().ok().and_then(|o| o.take())
    }
}

#[pymethods]
impl PyHlsPublisher {
    /// Return a fresh `HlsPublisherBuilder`.
    #[staticmethod]
    fn builder() -> PyHlsPublisherBuilder {
        PyHlsPublisherBuilder::default()
    }

    /// Push pre-muxed MPEG-TS bytes (must be a whole multiple of 188).
    /// Raises `HlsError(UNALIGNED_PUSH_TS)` otherwise.
    fn push_ts(&self, py: Python<'_>, ts_bytes: &Bound<'_, PyAny>) -> PyResult<()> {
        let coerced = coerce_bytes_like(py, ts_bytes)?;
        // Bind the &[u8] before the closure: the `Bound<PyBytes>` stays on
        // the stack pinning the bytes; the borrowed slice is `Ungil` even
        // though the `Bound` itself is not.
        let slice: &[u8] = coerced.as_bytes();
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("HlsPublisher mutex poisoned"))?;
        let inner = guard
            .as_mut()
            .ok_or_else(|| make_hls_error(py, "FINISHED", "HlsPublisher finished"))?;
        py.allow_threads(|| Publisher::push_ts(inner, slice))
            .map_err(|e| map_hls_error(py, &e))
    }

    /// Hint that the next `push_ts` should start a new segment.
    fn cut_segment(&self, py: Python<'_>) -> PyResult<()> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("HlsPublisher mutex poisoned"))?;
        let inner = guard
            .as_mut()
            .ok_or_else(|| make_hls_error(py, "FINISHED", "HlsPublisher finished"))?;
        py.allow_threads(|| Publisher::cut_segment(inner))
            .map_err(|e| map_hls_error(py, &e))
    }

    /// Finalize: flush the open segment, write the terminal playlist,
    /// tear down the HTTP server. **Consumes** the inner publisher;
    /// subsequent calls raise `HlsError(FINISHED)`.
    fn finish(&self, py: Python<'_>) -> PyResult<()> {
        let inner = {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| PyRuntimeError::new_err("HlsPublisher mutex poisoned"))?;
            guard
                .take()
                .ok_or_else(|| make_hls_error(py, "FINISHED", "HlsPublisher already finished"))?
        };
        py.allow_threads(|| Publisher::finish(inner))
            .map_err(|e| map_hls_error(py, &e))
    }

    /// Universal cross-publisher stats (`PublisherStats`).
    fn stats(&self, py: Python<'_>) -> PyResult<PyPublisherStats> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("HlsPublisher mutex poisoned"))?;
        let inner = guard
            .as_ref()
            .ok_or_else(|| make_hls_error(py, "FINISHED", "HlsPublisher finished"))?;
        Ok(PyPublisherStats::from_core(Publisher::stats(inner)))
    }

    /// Richer HLS-specific stats (`HlsStats`).
    fn hls_stats(&self, py: Python<'_>) -> PyResult<PyHlsStats> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("HlsPublisher mutex poisoned"))?;
        let inner = guard
            .as_ref()
            .ok_or_else(|| make_hls_error(py, "FINISHED", "HlsPublisher finished"))?;
        Ok(PyHlsStats::from(inner.hls_stats()))
    }

    /// Local socket address the HTTP server bound to, as `"ip:port"`, or
    /// `None` if the server is no longer running (e.g. after `finish`).
    fn local_addr(&self, py: Python<'_>) -> PyResult<Option<String>> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("HlsPublisher mutex poisoned"))?;
        let inner = guard
            .as_ref()
            .ok_or_else(|| make_hls_error(py, "FINISHED", "HlsPublisher finished"))?;
        Ok(inner.local_addr().map(|a| a.to_string()))
    }

    /// Convenience: the bound TCP port (0 if no server). Raises
    /// `HlsError(FINISHED)` if consumed.
    fn local_port(&self, py: Python<'_>) -> PyResult<u16> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("HlsPublisher mutex poisoned"))?;
        let inner = guard
            .as_ref()
            .ok_or_else(|| make_hls_error(py, "FINISHED", "HlsPublisher finished"))?;
        Ok(inner.local_addr().map(|a| a.port()).unwrap_or(0))
    }

    /// Render the current playlist text. `is_event` selects the terminal
    /// (final) form when true (writes `#EXT-X-ENDLIST`).
    #[pyo3(signature = (is_event = false))]
    fn render_playlist(&self, py: Python<'_>, is_event: bool) -> PyResult<String> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| PyRuntimeError::new_err("HlsPublisher mutex poisoned"))?;
        let inner = guard
            .as_ref()
            .ok_or_else(|| make_hls_error(py, "FINISHED", "HlsPublisher finished"))?;
        Ok(inner.render_playlist(is_event))
    }

    /// Close = `finish()` semantics but never raises if already finished
    /// (idempotent). Useful in `with`-style cleanup.
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        let inner = {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| PyRuntimeError::new_err("HlsPublisher mutex poisoned"))?;
            guard.take()
        };
        if let Some(inner) = inner {
            py.allow_threads(|| Publisher::finish(inner))
                .map_err(|e| map_hls_error(py, &e))?;
        }
        Ok(())
    }

    fn __repr__(&self) -> String {
        let open = self.inner.lock().map(|g| g.is_some()).unwrap_or(false);
        if open {
            "HlsPublisher(open)".to_string()
        } else {
            "HlsPublisher(finished)".to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// PyHlsPublisherBuilder — builder for PyHlsPublisher
// ---------------------------------------------------------------------------

/// Builder for `HlsPublisher`. Chain setters then call `.build()`.
///
/// The setters mirror `tst_tcp::hls::HlsPublisherBuilder`. Each setter
/// returns `self` for chaining.
#[pyclass(name = "HlsPublisherBuilder", module = "tstrans.hls")]
#[derive(Default)]
pub(crate) struct PyHlsPublisherBuilder {
    /// Stored as `Option` so each move-style setter can `take()` the inner
    /// Rust builder, apply the consuming method, and replace it (the Rust
    /// builder uses move-style `self -> Self` chaining).
    inner: Option<HlsPublisherBuilder>,
}

impl PyHlsPublisherBuilder {
    /// Apply a move-style mutation to the inner Rust builder.
    fn apply<F>(&mut self, f: F) -> PyResult<()>
    where
        F: FnOnce(HlsPublisherBuilder) -> HlsPublisherBuilder,
    {
        let b = self.inner.take().unwrap_or_default();
        self.inner = Some(f(b));
        Ok(())
    }
}

#[pymethods]
impl PyHlsPublisherBuilder {
    #[new]
    fn new() -> Self {
        Self {
            inner: Some(HlsPublisherBuilder::new()),
        }
    }

    /// HTTP server bind address (e.g. `"127.0.0.1:0"` for an OS-assigned
    /// port). Raises `ValueError` on a malformed socket address.
    fn bind<'py>(mut slf: PyRefMut<'py, Self>, addr: &str) -> PyResult<PyRefMut<'py, Self>> {
        let parsed: SocketAddr = addr
            .parse()
            .map_err(|e| PyValueError::new_err(format!("invalid bind address {addr:?}: {e}")))?;
        slf.apply(|b| b.bind(parsed))?;
        Ok(slf)
    }

    /// Filesystem directory for `.ts` segments + `playlist.m3u8`.
    fn output_dir<'py>(mut slf: PyRefMut<'py, Self>, path: &str) -> PyResult<PyRefMut<'py, Self>> {
        let p = path.to_string();
        slf.apply(|b| b.output_dir(p))?;
        Ok(slf)
    }

    /// Target segment duration in milliseconds.
    fn segment_duration_ms(mut slf: PyRefMut<'_, Self>, ms: u64) -> PyResult<PyRefMut<'_, Self>> {
        let d = Duration::from_millis(ms);
        slf.apply(|b| b.segment_duration(d))?;
        Ok(slf)
    }

    /// Rolling-window size (number of segments visible in a LIVE playlist).
    fn playlist_window(mut slf: PyRefMut<'_, Self>, n: usize) -> PyResult<PyRefMut<'_, Self>> {
        slf.apply(|b| b.playlist_window(n))?;
        Ok(slf)
    }

    /// Playlist mode (LIVE / EVENT / VOD).
    fn mode(mut slf: PyRefMut<'_, Self>, mode: PyHlsMode) -> PyResult<PyRefMut<'_, Self>> {
        let rust_mode: HlsMode = mode.into();
        slf.apply(|b| b.mode(rust_mode))?;
        Ok(slf)
    }

    /// Enable HTTP Basic auth with `(user, password)`.
    fn basic_auth<'py>(
        mut slf: PyRefMut<'py, Self>,
        user: &str,
        password: &str,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let (u, p) = (user.to_string(), password.to_string());
        slf.apply(|b| b.basic_auth(u, p))?;
        Ok(slf)
    }

    /// Enable HTTPS by supplying PEM cert + key file paths. Requires the
    /// `tls` cargo feature on tst-tcp; without it `build()` raises
    /// `HlsError(TLS_DISABLED)`.
    fn enable_tls<'py>(
        mut slf: PyRefMut<'py, Self>,
        cert: &str,
        key: &str,
    ) -> PyResult<PyRefMut<'py, Self>> {
        let (c, k) = (cert.to_string(), key.to_string());
        slf.apply(|b| b.enable_tls(c, k))?;
        Ok(slf)
    }

    /// Seed the builder from an `hls://` / `hlss://` URL. Replaces the
    /// builder's accumulated config with the URL-derived one (subsequent
    /// setters overlay on top). Raises `HlsError(URL)` on a bad URL.
    fn from_url<'py>(mut slf: PyRefMut<'py, Self>, url: &str) -> PyResult<PyRefMut<'py, Self>> {
        let py = slf.py();
        let b = HlsPublisherBuilder::from_url(url).map_err(|e| map_hls_url_error(py, &e))?;
        slf.inner = Some(b);
        Ok(slf)
    }

    /// Build the `HlsPublisher` (binds the HTTP server immediately).
    /// Raises `HlsError(BIND_FAILED)` / `HlsError(INVALID_CONFIG)` /
    /// `HlsError(TLS_DISABLED)` per the failure.
    fn build(&mut self, py: Python<'_>) -> PyResult<PyHlsPublisher> {
        let b = self
            .inner
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("HlsPublisherBuilder already consumed"))?;
        let pub_ = py
            .allow_threads(|| b.build())
            .map_err(|e| map_hls_error(py, &e))?;
        Ok(PyHlsPublisher::from_inner(pub_))
    }

    fn __repr__(&self) -> &'static str {
        "HlsPublisherBuilder(...)"
    }
}

/// Coerce a Python bytes-like argument to an owned `Py<PyBytes>` whose
/// `.as_bytes()` borrow lives across a subsequent `py.allow_threads`
/// call. Mirrors the rtp/mux_sender.rs helper.
fn coerce_bytes_like<'py>(
    py: Python<'py>,
    arg: &Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyBytes>> {
    use pyo3::intern;
    if let Ok(b) = arg.downcast::<PyBytes>() {
        return Ok(b.clone());
    }
    py.import_bound("builtins")?
        .getattr(intern!(py, "bytes"))?
        .call1((arg,))?
        .downcast_into::<PyBytes>()
        .map_err(|e| e.into())
}
