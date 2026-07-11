//! Python bindings for tst-hls (`tstrans.hls`). Gated on `feature = "hls"`.
//!
//! Populated by Plan A5b Wave C. HLS lives in the `tst-hls` crate
//! (`tst_hls`) — the `hls` cargo feature here pulls
//! `tst-hls` + `tst-pipeline` (for the `MuxPublisher` shell).
//!
//! Surface (module `"tstrans.hls"`):
//! - `Publisher` (ABC) + `PublisherStats` — T10
//! - `MuxPublisher` + `MuxPublisherStats` — T11
//! - `HlsPublisher` + `HlsPublisherBuilder` — T12
//! - `HlsMode` + `HlsStats` — T13
//! - `HlsError` / `HlsErrorKind` (in `tstrans.exceptions`) + error
//!   mapping + ratchets — T14
//!
//! GIL boundaries: `push_ts` / `cut_segment` / `finish` / builder
//! `build` release the GIL via `py.allow_threads` (disk + HTTP work is
//! pure Rust). Read-only getters do not release it.
//!
//! Error mapping: `tst_hls::HlsError` → `tstrans.exceptions.HlsError`
//! via `map_hls_error` (exhaustive over `HlsErrorKind`, with a wildcard
//! for the `#[non_exhaustive]` enum). The Rust `HlsErrorKind` is
//! 1-indexed; the Python `HlsErrorKind` IntEnum is 0-indexed — the
//! mapping uses enum *names* so there's no off-by-one.
//!
//! Two ratchets back this module:
//! - `scripts/check-py-hls-error-mapping-coverage.sh` — every
//!   `HlsErrorKind` variant has a `make_hls_error(py, "<KIND>", ...)`
//!   call site.
//! - `scripts/check/python/publisher-class-mirror.sh` — the Python
//!   `Publisher` ABC's abstract methods mirror the Rust
//!   `tst_core::publisher::Publisher` trait.

#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use pyo3::prelude::*;

use tst_hls::{HlsError, HlsErrorKind, HlsUrlError};
use tst_pipeline::MuxPublisherError;

pub(crate) use crate::errors::make_hls_error;

pub(crate) mod config;
pub(crate) mod mux_publisher;
pub(crate) mod publisher;
pub(crate) mod publisher_abc;

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/// Map a `tst_hls::HlsError` to a `tstrans.exceptions.HlsError`.
///
/// Exhaustive over the 9 `HlsErrorKind` variants; the wildcard arm routes
/// any future `#[non_exhaustive]` addition to `INTERNAL` so this fn never
/// panics on a Rust-side enum growth. The
/// `check-py-hls-error-mapping-coverage.sh` ratchet surfaces an
/// unmapped variant in CI.
pub(crate) fn map_hls_error(py: Python<'_>, e: &HlsError) -> PyErr {
    let msg = e.to_string();
    match e.kind() {
        HlsErrorKind::Url => make_hls_error(py, "URL", &msg),
        HlsErrorKind::Io => make_hls_error(py, "IO", &msg),
        HlsErrorKind::BindFailed => make_hls_error(py, "BIND_FAILED", &msg),
        HlsErrorKind::InvalidConfig => make_hls_error(py, "INVALID_CONFIG", &msg),
        HlsErrorKind::UnalignedPushTs => make_hls_error(py, "UNALIGNED_PUSH_TS", &msg),
        HlsErrorKind::Finished => make_hls_error(py, "FINISHED", &msg),
        HlsErrorKind::TlsDisabled => make_hls_error(py, "TLS_DISABLED", &msg),
        HlsErrorKind::Tls => make_hls_error(py, "TLS", &msg),
        HlsErrorKind::Internal => make_hls_error(py, "INTERNAL", &msg),
        // Wildcard for #[non_exhaustive] additions not yet mapped.
        _ => make_hls_error(py, "INTERNAL", &msg),
    }
}

/// Map a `tst_hls::HlsUrlError` (from `builder.from_url`) to a
/// `tstrans.exceptions.HlsError` with `kind=URL`.
pub(crate) fn map_hls_url_error(py: Python<'_>, e: &HlsUrlError) -> PyErr {
    make_hls_error(py, "URL", &e.to_string())
}

/// Map a `tst_pipeline::MuxPublisherError<HlsError>` raised by a
/// `MuxPublisher` send/cut into a Python exception.
///
/// Routing:
/// - `Publisher(HlsError)` → the inner HLS error, fully discriminated via
///   `map_hls_error` (so an unaligned push or a finished sink surfaces
///   with the right kind).
/// - `Mux(MuxError)` → `HlsError(INVALID_CONFIG)` (muxer rejected the
///   input; the free-text message carries the detail).
/// - `Closed` → `HlsError(FINISHED)` (shell was consumed via
///   `finish_into_publisher`).
/// - wildcard (`#[non_exhaustive]`) → `HlsError(INTERNAL)`.
pub(crate) fn map_mux_publisher_error(py: Python<'_>, e: MuxPublisherError<HlsError>) -> PyErr {
    match e {
        MuxPublisherError::Publisher(hls_err) => map_hls_error(py, &hls_err),
        MuxPublisherError::Mux(mux_err) => {
            make_hls_error(py, "INVALID_CONFIG", &mux_err.to_string())
        }
        MuxPublisherError::Closed => {
            make_hls_error(py, "FINISHED", "MuxPublisher already finished")
        }
        // Wildcard for #[non_exhaustive] additions not yet mapped.
        ref other => make_hls_error(py, "INTERNAL", &other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Module registration
// ---------------------------------------------------------------------------

pub(crate) fn register(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let m = PyModule::new_bound(parent.py(), "hls")?;
    // T10 — PublisherStats. The `Publisher` ABC itself is a pure-Python
    // `abc.ABC` defined in `tstrans/hls.py` (see the NOTE below).
    m.add_class::<publisher_abc::PyPublisherStats>()?;
    // T13 — HlsMode + HlsStats.
    m.add_class::<config::PyHlsMode>()?;
    m.add_class::<config::PyHlsStats>()?;
    // T12 — HlsPublisher + builder + server handle.
    m.add_class::<publisher::PyHlsPublisher>()?;
    m.add_class::<publisher::PyHlsPublisherBuilder>()?;
    m.add_class::<publisher::PyHlsServerHandle>()?;
    // T11 — MuxPublisher + MuxPublisherStats.
    m.add_class::<mux_publisher::PyMuxPublisher>()?;
    m.add_class::<mux_publisher::PyMuxPublisherStats>()?;

    // NOTE: the `Publisher` ABC + `Publisher.register(HlsPublisher)`
    // virtual-subclass wiring lives in the Python layer
    // (`tstrans/hls.py`), NOT here. A native PyO3 pyclass has the plain
    // `type` metaclass and no `.register()` classmethod, so the ABC must
    // be a real `abc.ABC` built in Python; the native crate exposes only
    // `PublisherStats` here.

    parent.add_submodule(&m)?;
    Ok(())
}
