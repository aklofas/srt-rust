//! Rust-side helpers that construct the Python exception classes
//! defined in `tstrans.exceptions`. Type wrappers use these to raise
//! Python-side exceptions — e.g. `Demuxer.feed_bytes` calls
//! `make_demux_error(py, "BAD_PMT", "...")` when the underlying
//! `tst_core::mpegts::Demuxer` returns an error.
//!
//! Implementation note: we deliberately do NOT use PyO3's
//! `create_exception!` (which would mint *new* exception classes on
//! the Rust side, distinct from the Python-defined `class MuxError`).
//! Users need `isinstance(err, tstrans.exceptions.MuxError)` to work
//! whether the error comes from Python or Rust — so the Rust side
//! must *call into* the Python-defined classes, which is what
//! `py.import_bound("tstrans.exceptions").getattr("MuxError")?` does.
//! This is slower than `create_exception!` (per-raise dict lookup +
//! Python call) but the tradeoff is required for the contract.

// PyO3's `#[pyfunction]` macro (Rust 2024 edition) generates extractor
// code that calls `pyo3::impl_::extract_argument::unwrap_required_argument`
// — an unsafe fn — without an explicit `unsafe {}` block in the expansion.
// The `useless_conversion` allow covers a `PyErr -> PyErr` `.into()` emitted
// by the same macro. Both suppressions are scoped to macro-generated code
// only; hand-written code in this file contains no unsafe blocks.
#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Build a `MuxError` Python exception with the right `.kind` Enum
/// value and `message` attribute. Used from inside the
/// `Muxer.push_video` / `push_klv` / `push_audio` wrappers.
///
/// `kind_variant` is the Python-side `MuxErrorKind` Enum variant name
/// (e.g. `"CONFIG_INVALID"`, `"INTERNAL"`). Caller must pass a valid
/// variant — invalid names raise `AttributeError` from
/// `MuxErrorKind.<NAME>` lookup, which surfaces as a `PyErr` and is
/// returned in place of the intended `MuxError`.
pub fn make_mux_error(py: Python<'_>, kind_variant: &str, message: &str) -> PyErr {
    let exceptions = match py.import_bound("tstrans.exceptions") {
        Ok(m) => m,
        Err(e) => return e,
    };
    let kind_enum = match exceptions.getattr(intern!(py, "MuxErrorKind")) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let kind_value = match kind_enum.getattr(kind_variant) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let mux_error_cls = match exceptions.getattr(intern!(py, "MuxError")) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let kwargs = PyDict::new_bound(py);
    if let Err(e) = kwargs.set_item("kind", kind_value) {
        return e;
    }
    if let Err(e) = kwargs.set_item("message", message) {
        return e;
    }
    match mux_error_cls.call((), Some(&kwargs)) {
        Ok(instance) => PyErr::from_value_bound(instance),
        Err(e) => e,
    }
}

/// Build a `DemuxError` Python exception. Mirror of `make_mux_error`
/// targeting `tstrans.exceptions.DemuxError` + `DemuxErrorKind`.
///
/// `kind_variant` must be a Python-side `DemuxErrorKind` Enum variant
/// name (e.g. `"SYNC_LOSS"`, `"BAD_PMT"`, `"INTERNAL"`).
pub fn make_demux_error(py: Python<'_>, kind_variant: &str, message: &str) -> PyErr {
    let exceptions = match py.import_bound("tstrans.exceptions") {
        Ok(m) => m,
        Err(e) => return e,
    };
    let kind_enum = match exceptions.getattr(intern!(py, "DemuxErrorKind")) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let kind_value = match kind_enum.getattr(kind_variant) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let demux_error_cls = match exceptions.getattr(intern!(py, "DemuxError")) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let kwargs = PyDict::new_bound(py);
    if let Err(e) = kwargs.set_item("kind", kind_value) {
        return e;
    }
    if let Err(e) = kwargs.set_item("message", message) {
        return e;
    }
    match demux_error_cls.call((), Some(&kwargs)) {
        Ok(instance) => PyErr::from_value_bound(instance),
        Err(e) => e,
    }
}

/// Build a `KlvError` Python exception. Mirror of `make_mux_error` /
/// `make_demux_error` targeting `tstrans.exceptions.KlvError` +
/// `KlvErrorKind`.
///
/// `kind_variant` must be a Python-side `KlvErrorKind` Enum variant
/// name (e.g. `"BAD_UNIVERSAL_LABEL"`, `"TRUNCATED_SET"`,
/// `"CHECKSUM_MISMATCH"`, `"INTERNAL"`).
pub fn make_klv_error(py: Python<'_>, kind_variant: &str, message: &str) -> PyErr {
    let exceptions = match py.import_bound("tstrans.exceptions") {
        Ok(m) => m,
        Err(e) => return e,
    };
    let kind_enum = match exceptions.getattr(intern!(py, "KlvErrorKind")) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let kind_value = match kind_enum.getattr(kind_variant) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let klv_error_cls = match exceptions.getattr(intern!(py, "KlvError")) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let kwargs = PyDict::new_bound(py);
    if let Err(e) = kwargs.set_item("kind", kind_value) {
        return e;
    }
    if let Err(e) = kwargs.set_item("message", message) {
        return e;
    }
    match klv_error_cls.call((), Some(&kwargs)) {
        Ok(instance) => PyErr::from_value_bound(instance),
        Err(e) => e,
    }
}

/// Build an `RtspError` Python exception. Mirror of `make_mux_error`
/// targeting `tstrans.exceptions.RtspError` + `RtspErrorKind`.
///
/// `kind_variant` must be a Python-side `RtspErrorKind` Enum variant
/// name (e.g. `"PROTOCOL"`, `"AUTH_FAILED"`, `"AUTH_REQUIRED"`,
/// `"NOT_FOUND"`, `"UNSUPPORTED_TRANSPORT"`, `"TLS"`, `"IO"`,
/// `"TIMEOUT"`, `"SERVER"`, `"MOUNT"`).
#[cfg(feature = "rtp")]
pub fn make_rtsp_error(py: Python<'_>, kind_variant: &str, message: &str) -> PyErr {
    let exceptions = match py.import_bound("tstrans.exceptions") {
        Ok(m) => m,
        Err(e) => return e,
    };
    let kind_enum = match exceptions.getattr(intern!(py, "RtspErrorKind")) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let kind_value = match kind_enum.getattr(kind_variant) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let rtsp_error_cls = match exceptions.getattr(intern!(py, "RtspError")) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let kwargs = PyDict::new_bound(py);
    if let Err(e) = kwargs.set_item("kind", kind_value) {
        return e;
    }
    if let Err(e) = kwargs.set_item("message", message) {
        return e;
    }
    match rtsp_error_cls.call((), Some(&kwargs)) {
        Ok(instance) => PyErr::from_value_bound(instance),
        Err(e) => e,
    }
}

/// Build an `RtpError` Python exception. Mirror of `make_mux_error`
/// targeting `tstrans.exceptions.RtpError` + `RtpErrorKind`.
///
/// `kind_variant` must be a Python-side `RtpErrorKind` Enum variant
/// name (e.g. `"TRANSPORT"`, `"MALFORMED_PACKET"`, `"CANCELLED"`).
#[cfg(feature = "rtp")]
pub fn make_rtp_error(py: Python<'_>, kind_variant: &str, message: &str) -> PyErr {
    let exceptions = match py.import_bound("tstrans.exceptions") {
        Ok(m) => m,
        Err(e) => return e,
    };
    let kind_enum = match exceptions.getattr(intern!(py, "RtpErrorKind")) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let kind_value = match kind_enum.getattr(kind_variant) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let rtp_error_cls = match exceptions.getattr(intern!(py, "RtpError")) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let kwargs = PyDict::new_bound(py);
    if let Err(e) = kwargs.set_item("kind", kind_value) {
        return e;
    }
    if let Err(e) = kwargs.set_item("message", message) {
        return e;
    }
    match rtp_error_cls.call((), Some(&kwargs)) {
        Ok(instance) => PyErr::from_value_bound(instance),
        Err(e) => e,
    }
}

/// Build an `SrtError` Python exception. Mirror of `make_rtp_error`
/// targeting `tstrans.exceptions.SrtError` + `SrtErrorKind`.
///
/// `kind_variant` must be a Python-side `SrtErrorKind` Enum variant
/// name (e.g. `"CONNECT_FAILED"`, `"ACCEPT_FAILED"`, `"WOULD_BLOCK"`,
/// `"TIMEOUT"`, `"CLOSED"`, `"BROKEN"`, `"CONFIG_INVALID"`, `"IO"`).
#[cfg(feature = "srt")]
pub fn make_srt_error(py: Python<'_>, kind_variant: &str, message: &str) -> PyErr {
    let exceptions = match py.import_bound("tstrans.exceptions") {
        Ok(m) => m,
        Err(e) => return e,
    };
    let kind_enum = match exceptions.getattr(intern!(py, "SrtErrorKind")) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let kind_value = match kind_enum.getattr(kind_variant) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let srt_error_cls = match exceptions.getattr(intern!(py, "SrtError")) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let kwargs = PyDict::new_bound(py);
    if let Err(e) = kwargs.set_item("kind", kind_value) {
        return e;
    }
    if let Err(e) = kwargs.set_item("message", message) {
        return e;
    }
    match srt_error_cls.call((), Some(&kwargs)) {
        Ok(instance) => PyErr::from_value_bound(instance),
        Err(e) => e,
    }
}

/// Test helper: forces a `MuxError` raise from Rust, used by
/// `test_error_wiring.py` to confirm end-to-end wiring. Exposed only
/// under the `_native._raise_mux_error_for_test` name.
#[pyfunction]
#[pyo3(name = "_raise_mux_error_for_test")]
pub fn raise_mux_error_for_test(py: Python<'_>, message: &str) -> PyResult<()> {
    Err(make_mux_error(py, "INTERNAL", message))
}

/// Test helper: forces an `RtspError` raise from Rust, exposed as
/// `_native._raise_rtsp_error_for_test` so the
/// `check-py-rtsp-error-mapping-coverage.sh` ratchet sees at least
/// one call site for `make_rtsp_error` per kind variant during Wave A.
#[cfg(feature = "rtp")]
#[pyfunction]
#[pyo3(name = "_raise_rtsp_error_for_test")]
pub fn raise_rtsp_error_for_test(py: Python<'_>, kind: &str, message: &str) -> PyResult<()> {
    Err(make_rtsp_error(py, kind, message))
}

/// Test helper: forces an `RtpError` raise from Rust, exposed as
/// `_native._raise_rtp_error_for_test`.
#[cfg(feature = "rtp")]
#[pyfunction]
#[pyo3(name = "_raise_rtp_error_for_test")]
pub fn raise_rtp_error_for_test(py: Python<'_>, kind: &str, message: &str) -> PyResult<()> {
    Err(make_rtp_error(py, kind, message))
}

/// Test helper: forces an `SrtError` raise from Rust, exposed as
/// `_native._raise_srt_error_for_test` so the
/// `check-py-srt-error-mapping-coverage.sh` ratchet sees at least
/// one call site for `make_srt_error` per kind variant during Wave A.
#[cfg(feature = "srt")]
#[pyfunction]
#[pyo3(name = "_raise_srt_error_for_test")]
pub fn raise_srt_error_for_test(py: Python<'_>, kind: &str, message: &str) -> PyResult<()> {
    Err(make_srt_error(py, kind, message))
}

/// Build a `UdpError` Python exception. Mirror of `make_rtsp_error`
/// targeting `tstrans.exceptions.UdpError` + `UdpErrorKind`.
///
/// `kind_variant` must be a Python-side `UdpErrorKind` Enum variant name
/// (e.g. `"IO"`, `"PAYLOAD_TOO_LARGE"`, `"CLOSED"`).
#[cfg(feature = "udp")]
pub fn make_udp_error(py: Python<'_>, kind_variant: &str, message: &str) -> PyErr {
    let exceptions = match py.import_bound("tstrans.exceptions") {
        Ok(m) => m,
        Err(e) => return e,
    };
    let kind_enum = match exceptions.getattr(intern!(py, "UdpErrorKind")) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let kind_value = match kind_enum.getattr(kind_variant) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let udp_error_cls = match exceptions.getattr(intern!(py, "UdpError")) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let kwargs = PyDict::new_bound(py);
    if let Err(e) = kwargs.set_item("kind", kind_value) {
        return e;
    }
    if let Err(e) = kwargs.set_item("message", message) {
        return e;
    }
    match udp_error_cls.call((), Some(&kwargs)) {
        Ok(instance) => PyErr::from_value_bound(instance),
        Err(e) => e,
    }
}

/// Test helper: forces a `UdpError` raise from Rust, exposed as
/// `_native._raise_udp_error_for_test` so the
/// `check-py-udp-error-mapping-coverage.sh` ratchet sees at least one
/// call site for `make_udp_error` per kind variant during Wave A.
#[cfg(feature = "udp")]
#[pyfunction]
#[pyo3(name = "_raise_udp_error_for_test")]
pub fn raise_udp_error_for_test(py: Python<'_>, kind: &str, message: &str) -> PyResult<()> {
    Err(make_udp_error(py, kind, message))
}

/// Build a `TcpError` Python exception. Mirror of `make_udp_error`
/// targeting `tstrans.exceptions.TcpError` + `TcpErrorKind`.
///
/// `kind_variant` must be a Python-side `TcpErrorKind` Enum variant name
/// (e.g. `"IO"`, `"PAYLOAD_TOO_LARGE"`, `"CLOSED"`, `"TLS_DISABLED"`).
#[cfg(feature = "tcp")]
pub fn make_tcp_error(py: Python<'_>, kind_variant: &str, message: &str) -> PyErr {
    let exceptions = match py.import_bound("tstrans.exceptions") {
        Ok(m) => m,
        Err(e) => return e,
    };
    let kind_enum = match exceptions.getattr(intern!(py, "TcpErrorKind")) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let kind_value = match kind_enum.getattr(kind_variant) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let tcp_error_cls = match exceptions.getattr(intern!(py, "TcpError")) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let kwargs = PyDict::new_bound(py);
    if let Err(e) = kwargs.set_item("kind", kind_value) {
        return e;
    }
    if let Err(e) = kwargs.set_item("message", message) {
        return e;
    }
    match tcp_error_cls.call((), Some(&kwargs)) {
        Ok(instance) => PyErr::from_value_bound(instance),
        Err(e) => e,
    }
}

/// Test helper: forces a `TcpError` raise from Rust, exposed as
/// `_native._raise_tcp_error_for_test` so the
/// `check-py-tcp-error-mapping-coverage.sh` ratchet sees at least one
/// call site for `make_tcp_error` per kind variant during Wave B.
#[cfg(feature = "tcp")]
#[pyfunction]
#[pyo3(name = "_raise_tcp_error_for_test")]
pub fn raise_tcp_error_for_test(py: Python<'_>, kind: &str, message: &str) -> PyResult<()> {
    Err(make_tcp_error(py, kind, message))
}

/// Build an `HlsError` Python exception. Mirror of `make_tcp_error`
/// targeting `tstrans.exceptions.HlsError` + `HlsErrorKind`.
///
/// `kind_variant` must be a Python-side `HlsErrorKind` Enum variant name
/// (e.g. `"URL"`, `"IO"`, `"BIND_FAILED"`, `"INVALID_CONFIG"`,
/// `"UNALIGNED_PUSH_TS"`, `"FINISHED"`, `"TLS_DISABLED"`, `"TLS"`,
/// `"INTERNAL"`).
#[cfg(feature = "hls")]
pub fn make_hls_error(py: Python<'_>, kind_variant: &str, message: &str) -> PyErr {
    let exceptions = match py.import_bound("tstrans.exceptions") {
        Ok(m) => m,
        Err(e) => return e,
    };
    let kind_enum = match exceptions.getattr(intern!(py, "HlsErrorKind")) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let kind_value = match kind_enum.getattr(kind_variant) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let hls_error_cls = match exceptions.getattr(intern!(py, "HlsError")) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let kwargs = PyDict::new_bound(py);
    if let Err(e) = kwargs.set_item("kind", kind_value) {
        return e;
    }
    if let Err(e) = kwargs.set_item("message", message) {
        return e;
    }
    match hls_error_cls.call((), Some(&kwargs)) {
        Ok(instance) => PyErr::from_value_bound(instance),
        Err(e) => e,
    }
}

/// Test helper: forces an `HlsError` raise from Rust, exposed as
/// `_native._raise_hls_error_for_test` so the
/// `check-py-hls-error-mapping-coverage.sh` ratchet sees at least one
/// call site for `make_hls_error` per kind variant during Wave C.
#[cfg(feature = "hls")]
#[pyfunction]
#[pyo3(name = "_raise_hls_error_for_test")]
pub fn raise_hls_error_for_test(py: Python<'_>, kind: &str, message: &str) -> PyResult<()> {
    Err(make_hls_error(py, kind, message))
}

/// Build a `RistError` Python exception. Mirror of `make_hls_error`
/// targeting `tstrans.exceptions.RistError` + `RistErrorKind`.
///
/// `kind_variant` must be a Python-side `RistErrorKind` Enum variant name
/// (e.g. `"URL"`, `"FFI"`, `"PAYLOAD_TOO_LARGE"`, `"CLOSED"`,
/// `"INVALID_CONFIG"`, `"ENCRYPTION_DISABLED"`, `"CONTEXT_CREATE_FAILED"`,
/// `"PEER_CREATE_FAILED"`, `"RECV_TIMEOUT"`, `"IO"`).
#[cfg(feature = "rist")]
pub fn make_rist_error(py: Python<'_>, kind_variant: &str, message: &str) -> PyErr {
    let exceptions = match py.import_bound("tstrans.exceptions") {
        Ok(m) => m,
        Err(e) => return e,
    };
    let kind_enum = match exceptions.getattr(intern!(py, "RistErrorKind")) {
        Ok(e) => e,
        Err(e) => return e,
    };
    let kind_value = match kind_enum.getattr(kind_variant) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let rist_error_cls = match exceptions.getattr(intern!(py, "RistError")) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let kwargs = PyDict::new_bound(py);
    if let Err(e) = kwargs.set_item("kind", kind_value) {
        return e;
    }
    if let Err(e) = kwargs.set_item("message", message) {
        return e;
    }
    match rist_error_cls.call((), Some(&kwargs)) {
        Ok(instance) => PyErr::from_value_bound(instance),
        Err(e) => e,
    }
}

/// Test helper: forces a `RistError` raise from Rust, exposed as
/// `_native._raise_rist_error_for_test` so the
/// `check-py-rist-error-mapping-coverage.sh` ratchet sees at least one
/// call site for `make_rist_error` per kind variant during Wave D.
#[cfg(feature = "rist")]
#[pyfunction]
#[pyo3(name = "_raise_rist_error_for_test")]
pub fn raise_rist_error_for_test(py: Python<'_>, kind: &str, message: &str) -> PyResult<()> {
    Err(make_rist_error(py, kind, message))
}

// ---------------------------------------------------------------------------
// Rust-typed → PyErr mappers
// ---------------------------------------------------------------------------

/// Map a Rust `MuxError` to a Python `MuxError` instance. Routes
/// via the 5-variant `MuxSenderErrorKind` coarse classification —
/// the muxer's `kind()` accessor (plan #91) is the source of truth
/// for which Python `MuxErrorKind` variant to use.
///
/// The `MuxSenderErrorKind` enum is `#[non_exhaustive]`; the wildcard
/// arm routes unknown future variants to `INTERNAL` so this fn never
/// panics on a Rust-side enum addition (the test suite will surface
/// the omission when the new variant gets a tagged-test fixture).
///
/// Called from Muxer wrappers.
#[allow(dead_code)]
pub(crate) fn mux_error_to_pyerr(py: Python<'_>, e: tst_core::MuxError) -> PyErr {
    use tst_core::error::MuxSenderErrorKind;
    let kind_str = match e.kind() {
        MuxSenderErrorKind::InputMalformed => "INPUT_MALFORMED",
        MuxSenderErrorKind::ConfigInvalid => "CONFIG_INVALID",
        MuxSenderErrorKind::InvalidUsage => "INVALID_USAGE",
        MuxSenderErrorKind::Backpressure => "BACKPRESSURE",
        MuxSenderErrorKind::Internal => "INTERNAL",
        _ => "INTERNAL",
    };
    // BufferFull gets a Python-only breadcrumb: the most common way to
    // hit it is pushing on the original Muxer inside an active
    // `Muxer.write_file(...)` block — those pushes bypass the drain
    // proxy the `with` statement yields, so nothing ever drains. The
    // hint lives here (not in tst-core's Display) because `write_file`
    // exists only in the Python binding.
    let msg = match &e {
        tst_core::MuxError::BufferFull { .. } => format!(
            "{e}; if pushing inside `Muxer.write_file(...)`, push on the \
             proxy object the `with` statement yields — pushes on the \
             original Muxer bypass the per-push drain"
        ),
        _ => e.to_string(),
    };
    make_mux_error(py, kind_str, &msg)
}

/// Map a Rust `CodecParseError` to a Python `CodecError` instance.
///
/// `codec` is a short lowercase string naming the codec that failed
/// (e.g. `"h264"`, `"h265"`, `"aac"`). Forwards all variant-specific
/// fields as keyword arguments to `CodecError.__init__` so the Python
/// side can read `.offset_bits`, `.field`, `.expected`, etc.
///
/// The wildcard arm routes unknown future variants (added via the
/// `#[non_exhaustive]` hatch on `CodecParseError`) to `ENGINE_ERROR`
/// so this fn never panics on a Rust-side enum addition — the bash
/// ratchet `scripts/check/python/codec-error-mapping-coverage.sh` will surface the
/// omission in CI.
///
/// Called from codec-parser wrappers.
#[allow(dead_code)]
pub(crate) fn codec_parse_error_to_pyerr(
    py: Python<'_>,
    err: &tst_core::codec::CodecParseError,
    codec: &str,
) -> PyErr {
    use tst_core::codec::CodecParseError;
    let exceptions = match py.import_bound("tstrans.exceptions") {
        Ok(m) => m,
        Err(e) => return e,
    };
    let kind_class = match exceptions.getattr(intern!(py, "CodecErrorKind")) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let codec_error_class = match exceptions.getattr(intern!(py, "CodecError")) {
        Ok(c) => c,
        Err(e) => return e,
    };
    let (kind_name, extra_attrs): (&str, Vec<(&str, PyObject)>) = match err {
        CodecParseError::TruncatedRbsp {
            offset_bits,
            needed_bits,
        } => (
            "TRUNCATED_RBSP",
            vec![
                ("offset_bits", offset_bits.into_py(py)),
                ("needed_bits", needed_bits.into_py(py)),
            ],
        ),
        CodecParseError::InvalidGolomb { offset_bits } => (
            "INVALID_GOLOMB",
            vec![("offset_bits", offset_bits.into_py(py))],
        ),
        CodecParseError::ReservedValue { field, value } => (
            "RESERVED_VALUE",
            vec![
                ("field", (*field).into_py(py)),
                ("value", value.into_py(py)),
            ],
        ),
        CodecParseError::UnsupportedProfile { profile_idc } => (
            "UNSUPPORTED_PROFILE",
            vec![("profile_idc", profile_idc.into_py(py))],
        ),
        CodecParseError::DanglingSpsReference { sps_id } => (
            "DANGLING_SPS_REFERENCE",
            vec![("sps_id", sps_id.into_py(py))],
        ),
        CodecParseError::DanglingVpsReference { vps_id } => (
            "DANGLING_VPS_REFERENCE",
            vec![("vps_id", vps_id.into_py(py))],
        ),
        CodecParseError::EngineError(_) => ("ENGINE_ERROR", vec![]),
        CodecParseError::InvalidLeb128 { offset_bytes } => (
            "INVALID_LEB128",
            vec![("offset_bytes", offset_bytes.into_py(py))],
        ),
        CodecParseError::BadSyncWord { expected, found } => (
            "BAD_SYNC_WORD",
            vec![
                ("expected", expected.into_py(py)),
                ("found", found.into_py(py)),
            ],
        ),
        CodecParseError::Truncated { needed, had } => (
            "TRUNCATED",
            vec![("needed", needed.into_py(py)), ("had", had.into_py(py))],
        ),
        CodecParseError::Forbidden { field } => {
            ("FORBIDDEN", vec![("field", (*field).into_py(py))])
        }
        CodecParseError::UnsupportedFreeFormat { layer } => (
            "UNSUPPORTED_FREE_FORMAT",
            vec![("layer", layer.into_py(py))],
        ),
        // Catch-all for #[non_exhaustive] additions not yet mapped:
        _ => ("ENGINE_ERROR", vec![]),
    };
    let kind = match kind_class.getattr(kind_name) {
        Ok(k) => k,
        Err(e) => return e,
    };
    let message = format!("{err}");
    let kwargs = PyDict::new_bound(py);
    if let Err(e) = kwargs.set_item("kind", kind) {
        return e;
    }
    if let Err(e) = kwargs.set_item("codec", codec) {
        return e;
    }
    if let Err(e) = kwargs.set_item("message", &message) {
        return e;
    }
    for (k, v) in extra_attrs {
        if let Err(e) = kwargs.set_item(k, v) {
            return e;
        }
    }
    let positional_args = pyo3::types::PyTuple::empty_bound(py);
    match codec_error_class.call(positional_args, Some(&kwargs)) {
        Ok(instance) => PyErr::from_value_bound(instance),
        Err(e) => e,
    }
}

/// Map a Rust `KlvEncodeError` to a Python `KlvEncodeError` instance.
/// Covers all 8 variants; the wildcard arm routes to `BUFFER_TOO_SMALL`
/// (a benign "encode failed; widen output buffer" fallback) for any
/// future Rust variants introduced through the `#[non_exhaustive]`
/// hatch — explicit arms get added as new variants surface.
///
/// Where the Rust variant carries a numeric identifier it is forwarded to
/// the Python `KlvEncodeError.tag` attribute: a KLV tag for `OutOfRange`,
/// `StringTooLong`, `MissingMandatoryItem`, `ReservedTagInUnknown`, and
/// `ForbiddenStandaloneOffset`; the VTarget Pack `target_id` for
/// `VTargetPackEmpty` and `DuplicateTargetId`. Variants without one
/// (`BufferTooSmall`, `RecordTooLarge`, `UnsupportedImapbLength`,
/// `InvalidImapbParams`) leave `.tag = None`.
///
/// Called from KLV `encode_*` wrappers.
#[allow(dead_code)]
pub(crate) fn klv_encode_error_to_pyerr(py: Python<'_>, e: tst_core::KlvEncodeError) -> PyErr {
    use tst_core::error::KlvEncodeError as RustE;
    let (kind_str, tag): (&str, Option<u32>) = match &e {
        RustE::BufferTooSmall { .. } => ("BUFFER_TOO_SMALL", None),
        RustE::RecordTooLarge => ("RECORD_TOO_LARGE", None),
        RustE::OutOfRange { tag, .. } => ("OUT_OF_RANGE", Some(*tag)),
        RustE::StringTooLong { tag, .. } => ("STRING_TOO_LONG", Some(*tag)),
        RustE::UnsupportedImapbLength { .. } => ("UNSUPPORTED_IMAPB_LENGTH", None),
        RustE::InvalidImapbParams { .. } => ("INVALID_IMAPB_PARAMS", None),
        RustE::MissingMandatoryItem { tag, .. } => {
            ("MISSING_MANDATORY_ITEM", Some(u32::from(*tag)))
        }
        RustE::ReservedTagInUnknown { tag } => ("RESERVED_TAG_IN_UNKNOWN", Some(*tag)),
        RustE::VTargetPackEmpty { target_id } => ("VTARGET_PACK_EMPTY", Some(*target_id as u32)),
        RustE::DuplicateTargetId { target_id } => ("DUPLICATE_TARGET_ID", Some(*target_id as u32)),
        RustE::ForbiddenStandaloneOffset { tag } => ("FORBIDDEN_STANDALONE_OFFSET", Some(*tag)),
        _ => ("BUFFER_TOO_SMALL", None),
    };
    let msg = e.to_string();
    let exceptions = match py.import_bound("tstrans.exceptions") {
        Ok(m) => m,
        Err(err) => return err,
    };
    let kind_enum = match exceptions.getattr(intern!(py, "KlvEncodeErrorKind")) {
        Ok(en) => en,
        Err(err) => return err,
    };
    let kind_value = match kind_enum.getattr(kind_str) {
        Ok(v) => v,
        Err(err) => return err,
    };
    let cls = match exceptions.getattr(intern!(py, "KlvEncodeError")) {
        Ok(c) => c,
        Err(err) => return err,
    };
    let kwargs = PyDict::new_bound(py);
    if let Err(err) = kwargs.set_item("kind", kind_value) {
        return err;
    }
    if let Some(t) = tag {
        if let Err(err) = kwargs.set_item("tag", t) {
            return err;
        }
    }
    match cls.call((msg,), Some(&kwargs)) {
        Ok(instance) => PyErr::from_value_bound(instance),
        Err(err) => err,
    }
}
