//! Rust-side helpers that construct the Python exception classes
//! defined in `tstrans.exceptions`. Phase 2+ plans use these from
//! within type wrappers — e.g. `Demuxer.feed_bytes` calls
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
/// value and `message` attribute. Phase 4 (Muxer wrap) uses this from
/// inside the `Muxer.push_video` / `push_klv` / `push_audio` wrappers.
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

/// Test helper: forces a `MuxError` raise from Rust, used by
/// `test_error_wiring.py` to confirm end-to-end wiring. Exposed only
/// under the `_native._raise_mux_error_for_test` name.
#[pyfunction]
#[pyo3(name = "_raise_mux_error_for_test")]
pub fn raise_mux_error_for_test(py: Python<'_>, message: &str) -> PyResult<()> {
    Err(make_mux_error(py, "INTERNAL", message))
}

// ---------------------------------------------------------------------------
// Rust-typed → PyErr mappers — Phase 4 (Muxer wrap)
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
/// Called from Phase 4 Muxer wrappers — unused until those land.
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
    let msg = e.to_string();
    make_mux_error(py, kind_str, &msg)
}

/// Map a Rust `KlvEncodeError` to a Python `KlvEncodeError` instance.
/// Covers all 8 variants; the wildcard arm routes to `BUFFER_TOO_SMALL`
/// (a benign "encode failed; widen output buffer" fallback) for any
/// future Rust variants introduced through the `#[non_exhaustive]`
/// hatch — explicit arms get added as new variants surface.
///
/// Where the Rust variant carries a `tag` field (`OutOfRange`,
/// `StringTooLong`, `MissingMandatoryItem`, `ReservedTagInUnknown`)
/// it is forwarded to the Python `KlvEncodeError.tag` attribute.
/// Variants without a tag (`BufferTooSmall`, `RecordTooLarge`,
/// `UnsupportedImapbLength`, `InvalidImapbParams`) leave `.tag = None`.
///
/// Called from Phase 4 KLV `encode_*` wrappers — unused until those land.
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
