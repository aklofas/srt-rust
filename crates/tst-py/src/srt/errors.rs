//! `tstrans.srt` Rust→Python error mapping helpers.
//!
//! Centralized exhaustive mappings from every Rust enum that can flow
//! through a `tstrans.srt` code path to one of the 8 `SrtErrorKind`
//! variants declared in `tstrans.exceptions`. Each public helper builds
//! an `SrtError` via `crate::errors::make_srt_error`.
//!
//! Routing rules (one paragraph each):
//!
//! - **`UrlError`** — every variant collapses to `CONFIG_INVALID`. URL
//!   parse failures are caller-misconfiguration by definition; the
//!   variant detail is preserved in the free-text message via
//!   `Display`.
//! - **`ConnectError`** — `InvalidAddress` / `InvalidOption` →
//!   `CONFIG_INVALID` (caller supplied a bad address or option);
//!   `TimedOut` → `TIMEOUT`; everything else (`Refused`, `BadEncryption`,
//!   `Rejected`, `System`, `Other` + future `#[non_exhaustive]`) →
//!   `CONNECT_FAILED`.
//! - **`BindError`** — `InvalidAddress` / `InvalidOption` →
//!   `CONFIG_INVALID`; everything else (`AddressInUse`,
//!   `PermissionDenied`, `System`, `Other` + future variants) →
//!   `CONNECT_FAILED`. The listener failed to come up, which the
//!   user-facing API treats as a connect-side failure.
//! - **`AcceptError`** — `TimedOut` → `TIMEOUT`; `ListenerClosed` →
//!   `CLOSED`; everything else (`PeerRejected`, `System`, `Other` +
//!   future variants) → `ACCEPT_FAILED`. Kept distinct from
//!   `CONNECT_FAILED` so callers can tell "I bound but the peer broke
//!   things" apart from "I could not bind at all".
//! - **`IoError`** — `SocketClosed` → `CLOSED`; everything else
//!   (`System(io::Error)`, `Other`, future variants) → `IO`.
//! - **`TransportError`** — `Backpressure` → `WOULD_BLOCK`; `Broken` →
//!   `BROKEN`; `Closed` / `ExplicitClose` → `CLOSED`; `TooLarge` →
//!   `CONFIG_INVALID` (the cap is a configurable payload size, so the
//!   caller can tune it); future `#[non_exhaustive]` additions → `IO`.
//!
//! The bash ratchet `scripts/check-py-srt-error-mapping-coverage.sh`
//! verifies every `SrtErrorKind` variant has at least one
//! `make_srt_error(py, "<KIND>", ...)` call site under
//! `crates/tst-py/src/srt/`. The single-line kind literal is required
//! by the line-based grep — multi-line wraps will not match.

use pyo3::prelude::*;

use tst_core::transport::TransportError;
use tst_srt::UrlError;
use tst_srt::error::{AcceptError, BindError, ConnectError, IoError};

use crate::errors::make_srt_error;

/// Map a `tst_srt::UrlError` (raised by `SrtUrl::parse` and friends) to
/// a `tstrans.exceptions.SrtError` with kind `CONFIG_INVALID`.
///
/// `UrlError` is `#[non_exhaustive]`; the single arm catches every
/// current variant (`Syntax`, `WrongScheme`, `MissingPort`,
/// `MissingHost`, `UserinfoNotSupported`, `UnsupportedMode`,
/// `UnsupportedKey`, `FfmpegAliasNotExposed`, `UnknownKey`,
/// `InvalidValue`, `OptionValidation`) AND any future addition — they
/// are all caller-misconfiguration by definition.
pub(crate) fn url_error_to_pyerr(py: Python<'_>, e: UrlError) -> PyErr {
    make_srt_error(py, "CONFIG_INVALID", &e.to_string())
}

/// Map a `tst_srt::ConnectError` (raised by `Socket::connect_with`) to
/// a `tstrans.exceptions.SrtError`. Exhaustive match against the 8
/// concrete variants today, plus a wildcard arm for
/// `#[non_exhaustive]` additions.
pub(crate) fn connect_error_to_pyerr(py: Python<'_>, e: ConnectError) -> PyErr {
    let msg = e.to_string();
    match e {
        ConnectError::InvalidAddress(_) | ConnectError::InvalidOption(_) => {
            make_srt_error(py, "CONFIG_INVALID", &msg)
        }
        ConnectError::TimedOut => make_srt_error(py, "TIMEOUT", &msg),
        ConnectError::Refused
        | ConnectError::BadEncryption { .. }
        | ConnectError::Rejected { .. }
        | ConnectError::System(_)
        | ConnectError::Other { .. } => make_srt_error(py, "CONNECT_FAILED", &msg),
        // Catch-all for future #[non_exhaustive] additions — surface as
        // CONNECT_FAILED since the variant describes a failed handshake.
        _ => make_srt_error(py, "CONNECT_FAILED", &msg),
    }
}

/// Map a `tst_srt::BindError` (raised by `Listener::bind_with`) to a
/// `tstrans.exceptions.SrtError`. Exhaustive match against the 6
/// concrete variants today.
pub(crate) fn bind_error_to_pyerr(py: Python<'_>, e: BindError) -> PyErr {
    let msg = e.to_string();
    match e {
        BindError::InvalidAddress(_) | BindError::InvalidOption(_) => {
            make_srt_error(py, "CONFIG_INVALID", &msg)
        }
        BindError::AddressInUse
        | BindError::PermissionDenied
        | BindError::System(_)
        | BindError::Other { .. } => make_srt_error(py, "CONNECT_FAILED", &msg),
        // Catch-all for #[non_exhaustive] additions.
        _ => make_srt_error(py, "CONNECT_FAILED", &msg),
    }
}

/// Map a `tst_srt::AcceptError` (raised by `Listener::accept`) to a
/// `tstrans.exceptions.SrtError`. Exhaustive match against the 5
/// concrete variants today.
pub(crate) fn accept_error_to_pyerr(py: Python<'_>, e: AcceptError) -> PyErr {
    let msg = e.to_string();
    match e {
        AcceptError::TimedOut => make_srt_error(py, "TIMEOUT", &msg),
        AcceptError::ListenerClosed => make_srt_error(py, "CLOSED", &msg),
        AcceptError::PeerRejected { .. } | AcceptError::System(_) | AcceptError::Other { .. } => {
            make_srt_error(py, "ACCEPT_FAILED", &msg)
        }
        // Catch-all for #[non_exhaustive] additions.
        _ => make_srt_error(py, "ACCEPT_FAILED", &msg),
    }
}

/// Map a `tst_srt::error::IoError` (raised by `SrtTransport::stats` and
/// other low-level libsrt IO entry points) to a
/// `tstrans.exceptions.SrtError`.
///
/// Used by both T2 (transport stats) and T3 (Socket/Listener low-level
/// surface) — kept here so the per-variant routing is consistent.
pub(crate) fn io_error_to_pyerr(py: Python<'_>, e: IoError) -> PyErr {
    let msg = e.to_string();
    match e {
        IoError::SocketClosed => make_srt_error(py, "CLOSED", &msg),
        IoError::System(_) | IoError::Other { .. } => make_srt_error(py, "IO", &msg),
        // Catch-all for #[non_exhaustive] additions.
        _ => make_srt_error(py, "IO", &msg),
    }
}

/// Map a `tst_core::transport::TransportError` (the unified transport
/// failure surface used by `tst_pipeline::Sender` / `Receiver`) to a
/// `tstrans.exceptions.SrtError`.
///
/// `Backpressure` is the only variant a polling caller would
/// reasonably retry; `Broken` requires re-establishing the transport.
/// `Closed` and `ExplicitClose` both surface as `CLOSED` — the
/// distinction (peer-EOS vs caller-initiated close) is not exposed at
/// the SRT Python surface today.
pub(crate) fn transport_error_to_pyerr(py: Python<'_>, e: TransportError) -> PyErr {
    match e {
        TransportError::Backpressure { msg, .. } => make_srt_error(py, "WOULD_BLOCK", &msg),
        TransportError::Broken { msg, .. } => make_srt_error(py, "BROKEN", &msg),
        TransportError::Closed => make_srt_error(py, "CLOSED", "transport closed"),
        TransportError::ExplicitClose => make_srt_error(py, "CLOSED", "transport explicit close"),
        TransportError::TooLarge { len, max } => {
            let msg = format!("payload too large: {len} bytes exceeds {max}-byte cap");
            make_srt_error(py, "CONFIG_INVALID", &msg)
        }
        // Catch-all for future #[non_exhaustive] additions (e.g.
        // `Cancelled` once Plan B lands). Surface as IO so the kind
        // is at least categorized; the message preserves the variant.
        other => make_srt_error(py, "IO", &other.to_string()),
    }
}
