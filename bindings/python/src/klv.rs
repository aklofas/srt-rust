//! PyO3 wrappers for `tst_core::klv::*` typed sets.
//!
//! Translation strategy: each Rust `*Ls` / `*Pack` struct is converted
//! to an instance of a Python-side dataclass under `tstrans.klv.*` via
//! per-set translator functions (`convert_uas_datalink`, etc.). Decode
//! entry points are `#[pyfunction]`s that map `KlvDecodeError` to
//! `tstrans.exceptions.KlvError` via `make_klv_error`.
//!
//! Covers ST 0601 / ST 0102 / ST 0605 / ST 0903 decode and encode
//! with field-error surfacing on the decode path.
//!
//! `#![allow(...)]` mirrors the pattern in `errors.rs` and `mpegts.rs` —
//! PyO3 0.22 + Rust 2024 macro expansions trip these lints. Hand-
//! written code in this module has no unsafe blocks.
#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use pyo3::exceptions::PyValueError;
use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use tst_core::error::KlvDecodeError;
use tst_core::error::KlvFieldError as RustKlvFieldError;
use tst_core::error::KlvPatchError;
use tst_core::klv::UniversalLabel;
use tst_core::klv::pack::OwnedRawField;
use tst_core::klv::st0102::{
    ClassifyingCountryCodingMethod as RustClsCountry, ObjectCountryCodingMethod as RustObjCountry,
    SecurityClassification as RustSecCls, SecurityLs, decode as decode_st0102_lenient,
    decode_strict as decode_st0102_strict,
    encode_strict_compliance as encode_st0102_strict_compliance, encode_to_vec as encode_st0102,
};
use tst_core::klv::st0601::{
    EncodeConfig as St0601EncodeConfig, IcingDetected as RustIcingDetected,
    MismmsViolation as RustMismmsViolation, OperationalMode as RustOperationalMode,
    OutOfRangePolicy as RustOutOfRangePolicy, SensorFovName as RustSensorFovName,
    St0601SentinelMeaning, UasDatalinkLs, decode as decode_st0601_lenient,
    decode_strict as decode_st0601_strict,
    decode_strict_compliance as decode_st0601_strict_compliance,
    encode_strict_compliance as encode_st0601_strict_compliance,
    encode_to_vec_with as encode_st0601_with, patch as patch_st0601,
    st0601_sentinel_meaning as rust_st0601_sentinel_meaning,
    validate_mismms as rust_validate_mismms,
};
use tst_core::klv::st0605::{
    PrecisionTimeStampPack, TimeStatus as RustTimeStatus, decode as decode_st0605,
    encode as encode_st0605,
};
use tst_core::klv::st0903::{
    VTargetPack as RustVTargetPack, VmtiLs, decode as decode_st0903_lenient,
    decode_strict as decode_st0903_strict,
    encode_standalone_strict_compliance as encode_st0903_standalone_strict_compliance,
    encode_strict_compliance as encode_st0903_strict_compliance, encode_to_vec as encode_st0903,
    encode_to_vec_standalone as encode_st0903_standalone,
};
use tst_core::klv::st1204::{
    CoreId as RustCoreId, IdType as RustIdType, St1204Error, decode as decode_st1204,
    encode_to_vec as encode_st1204,
};

use crate::errors::{klv_encode_error_to_pyerr, make_klv_error};

// ---------------------------------------------------------------------------
// KlvDecodeError → KlvError mapping
// ---------------------------------------------------------------------------

/// Map a Rust `KlvDecodeError` to a Python `KlvError` instance. Covers
/// every Rust variant; `KlvDecodeError` carries the non-exhaustive
/// attribute so the default arm routes to `INTERNAL` and we'll add new
/// explicit arms as new Rust variants surface.
pub(crate) fn klv_decode_error_to_pyerr(py: Python<'_>, e: KlvDecodeError) -> PyErr {
    let msg = format!("{e}");
    let kind = match &e {
        KlvDecodeError::Truncated { .. }
        | KlvDecodeError::MalformedLength { .. }
        | KlvDecodeError::LengthOverflow { .. } => "TRUNCATED_SET",
        KlvDecodeError::UnexpectedUniversalLabel { .. } => "BAD_UNIVERSAL_LABEL",
        KlvDecodeError::ChecksumMismatch { .. } => "CHECKSUM_MISMATCH",
        KlvDecodeError::DuplicateTag { .. } => "DUPLICATE_TAG",
        KlvDecodeError::Tag2NotFirst
        | KlvDecodeError::Tag1NotLast
        | KlvDecodeError::MissingTag65
        | KlvDecodeError::St0102MissingRequiredTag { .. }
        | KlvDecodeError::St0903MissingRequiredTag { .. } => "MISSING_REQUIRED_TAG",
        KlvDecodeError::MalformedTag { .. }
        | KlvDecodeError::NonCanonicalLength { .. }
        | KlvDecodeError::NonCanonicalTag { .. }
        | KlvDecodeError::TrailingBytes { .. }
        | KlvDecodeError::BadTimeStampPackLength { .. }
        | KlvDecodeError::ReservedBitsInvalid { .. }
        | KlvDecodeError::St0903InvalidVTargetPack { .. }
        | KlvDecodeError::FieldError(_) => "MALFORMED_BYTES",
        _ => "INTERNAL",
    };
    make_klv_error(py, kind, &msg)
}

// ---------------------------------------------------------------------------
// St1204Error → KlvError mapping
// ---------------------------------------------------------------------------

/// Map a Rust `St1204Error` to a Python `KlvError` instance.
fn st1204_error_to_pyerr(py: Python<'_>, e: St1204Error) -> PyErr {
    let msg = format!("{e}");
    let kind = match &e {
        St1204Error::Truncated => "TRUNCATED_SET",
        St1204Error::UnsupportedVersion(_) => "MALFORMED_BYTES",
        St1204Error::ReservedBitsSet => "MALFORMED_BYTES",
        St1204Error::InvalidUsage => "MALFORMED_BYTES",
        St1204Error::TrailingBytes => "MALFORMED_BYTES",
        _ => "INTERNAL",
    };
    make_klv_error(py, kind, &msg)
}

// ---------------------------------------------------------------------------
// ST 0605 — Precision Time Stamp Pack
// ---------------------------------------------------------------------------

/// Translate a Rust `PrecisionTimeStampPack` to a Python
/// `tstrans.klv.PrecisionTimeStampPack` dataclass instance.
fn convert_precision_timestamp_pack(
    py: Python<'_>,
    pack: &PrecisionTimeStampPack,
) -> PyResult<PyObject> {
    let klv_mod = py.import_bound("tstrans.klv")?;
    let time_status_cls = klv_mod.getattr(intern!(py, "TimeStatus"))?;
    let pack_cls = klv_mod.getattr(intern!(py, "PrecisionTimeStampPack"))?;

    let ts_kwargs = PyDict::new_bound(py);
    ts_kwargs.set_item("raw", pack.time_status.0)?;
    let time_status_py = time_status_cls.call((), Some(&ts_kwargs))?;

    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("time_status", time_status_py)?;
    kwargs.set_item("timestamp_us", pack.timestamp_us)?;
    Ok(pack_cls.call((), Some(&kwargs))?.unbind())
}

/// Decode an ST 0605 §7 Precision Time Stamp Pack. `buf` is the full
/// 26-byte wire-format pack (16-byte UL + 1-byte BER length + 1-byte
/// TimeStatus + 8-byte BE microsecond timestamp).
#[pyfunction]
#[pyo3(name = "decode_precision_timestamp")]
fn decode_precision_timestamp_py(py: Python<'_>, buf: &[u8]) -> PyResult<PyObject> {
    // Audit #11 / GIL-release decision: NOT wrapped in `py.allow_threads`.
    // The Rust `decode_st0605` call is ~1us for the 26-byte spec pack;
    // GIL transition overhead exceeds the decode time for any realistic
    // payload size at this entry point. See `bindings/python/tests/
    // test_gil_release.py` and the workspace reference memo
    // `reference_pyo3_allow_threads_pattern.md` for the empirical
    // breakeven point (~50us per call).
    match decode_st0605(buf) {
        Ok(pack) => convert_precision_timestamp_pack(py, &pack),
        Err(e) => Err(klv_decode_error_to_pyerr(py, e)),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers — KlvFieldError + OwnedRawField translators
// ---------------------------------------------------------------------------

/// Translate a Rust `KlvFieldError` to a Python `KlvFieldError` dataclass
/// instance. Variant→`KlvFieldErrorKind` mapping is exhaustive over the
/// current Rust enum; the wildcard arm covers future non-exhaustive
/// attribute additions by routing to `INVALID_LENGTH` (consumers should
/// treat unrecognized kinds as best-effort diagnostic only).
fn convert_field_error(py: Python<'_>, fe: &RustKlvFieldError) -> PyResult<PyObject> {
    let klv_mod = py.import_bound("tstrans.klv")?;
    let kind_enum = klv_mod.getattr(intern!(py, "KlvFieldErrorKind"))?;
    let cls = klv_mod.getattr(intern!(py, "KlvFieldError"))?;
    let (variant, tag): (&str, u32) = match fe {
        RustKlvFieldError::OutOfRange { tag, .. } => ("OUT_OF_RANGE", *tag),
        RustKlvFieldError::InvalidUtf8 { tag } => ("INVALID_UTF8", *tag),
        RustKlvFieldError::InvalidLength { tag, .. } => ("INVALID_LENGTH", *tag),
        RustKlvFieldError::InvalidUtf16 { tag } => ("INVALID_UTF16", *tag),
        RustKlvFieldError::InvalidCodepoint { tag, .. } => ("INVALID_CODEPOINT", *tag),
        RustKlvFieldError::TruncatedField { tag } => ("TRUNCATED_FIELD", *tag),
        RustKlvFieldError::UnsupportedImapbLength { .. } => ("UNSUPPORTED_IMAPB_LENGTH", 0),
        RustKlvFieldError::InvalidImapbParams { .. } => ("INVALID_IMAPB_PARAMS", 0),
        _ => ("INVALID_LENGTH", 0),
    };
    let kind_value = kind_enum.getattr(variant)?;
    let kwargs = PyDict::new_bound(py);
    kwargs.set_item("kind", kind_value)?;
    kwargs.set_item("tag", tag)?;
    kwargs.set_item("message", format!("{fe}"))?;
    Ok(cls.call((), Some(&kwargs))?.unbind())
}

/// Translate a list of `KlvFieldError`s to a Python tuple of
/// `KlvFieldError` dataclass instances.
fn convert_field_errors(py: Python<'_>, errs: &[RustKlvFieldError]) -> PyResult<PyObject> {
    let items: Vec<PyObject> = errs
        .iter()
        .map(|e| convert_field_error(py, e))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(pyo3::types::PyTuple::new_bound(py, items).unbind().into())
}

/// Translate a list of `OwnedRawField` to a Python tuple of `(tag, bytes)`
/// 2-tuples. The shape matches `SecurityLs.unknown` / `UasDatalinkLs.unknown`
/// in the Python typed-set dataclasses.
fn convert_unknown(py: Python<'_>, unknown: &[OwnedRawField]) -> PyResult<PyObject> {
    let items: Vec<PyObject> = unknown
        .iter()
        .map(|f| {
            pyo3::types::PyTuple::new_bound(
                py,
                &[
                    f.tag.into_py(py),
                    pyo3::types::PyBytes::new_bound(py, &f.value).into_py(py),
                ],
            )
            .unbind()
            .into()
        })
        .collect();
    Ok(pyo3::types::PyTuple::new_bound(py, items).unbind().into())
}

/// Inverse of `convert_unknown`: extracts the `unknown` field from a
/// Python typed-set dataclass into a `Vec<OwnedRawField>` for the
/// Rust struct. Each entry must be a 2-tuple `(int, bytes)`; malformed
/// shapes raise `TypeError` / `ValueError` rather than silently
/// corrupting the Rust side (audit #6's "validate-don't-drop" stance).
///
/// `is_typed_tag` is the per-set predicate identifying tags the
/// encoder's typed table covers. When a Python-supplied `unknown` entry
/// collides with a typed tag, the entry is silently dropped — the typed
/// field wins. Three reasons:
///
/// 1. Real decode never produces such an entry (the decoder routes
///    typed tags to typed fields, not to `unknown`), so this only
///    affects user-hand-constructed records.
/// 2. ST 0601's encoder errors with `ReservedTagInUnknown` on the same
///    collision pattern; filtering here keeps the four sets consistent
///    (the others' encoders would otherwise emit duplicate TLVs).
/// 3. "Drop on collision" produces deterministic, valid wire output
///    rather than failing the round-trip — matches the audit #5
///    "deterministic precedence" requirement.
fn py_to_unknown(
    p: &Bound<'_, PyAny>,
    is_typed_tag: impl Fn(u32) -> bool,
) -> PyResult<Vec<OwnedRawField>> {
    let py = p.py();
    let unknown_obj = p.getattr(intern!(py, "unknown"))?;
    let mut out = Vec::new();
    for item in unknown_obj.iter()? {
        let item = item?;
        // Each entry must be a 2-tuple. We rely on tuple-shaped extraction
        // via `(u32, Vec<u8>)` — PyO3 enforces the 2-arity + element types.
        let (tag, value): (u32, Vec<u8>) = item.extract().map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!(
                "unknown TLV must be a (int, bytes) 2-tuple: {e}"
            ))
        })?;
        if is_typed_tag(tag) {
            // Collision: typed field wins; drop the unknown entry.
            continue;
        }
        out.push(OwnedRawField { tag, value });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Per-set typed-tag predicates — narrow inventories duplicated locally
// (a few u8 constants) rather than threading internal Rust APIs out of
// `tst-core`. Each predicate mirrors the typed inventory listed in the
// corresponding `encode.rs`.
// ---------------------------------------------------------------------------

/// ST 0102 LS typed tags: 1..=14 + 22 + 23 + 24.
fn is_st0102_typed_tag(tag: u32) -> bool {
    matches!(tag, 1..=14 | 22 | 23 | 24)
}

/// ST 0601 LS typed + reserved tags — mirrors `tags::TAGS` in
/// `crates/tst-core/src/klv/st0601/tags.rs` (103 entries as of WP-A: 1-65,
/// 67-80, 82-95, 97-101, 106-108, 129, 135). Tags 3, 4, 66, 81, 96,
/// 102-105, 109-128, 130-134, 136..=255 are forward-compat and may
/// legitimately appear in `unknown`. WP-A (Tables A1-A4) extended the
/// range past 91 — keep this in sync with `tags::TAGS` when new tags are
/// typed, or a caller-supplied `unknown` entry for a newly-typed tag will
/// slip past this filter and get rejected downstream by the real Rust
/// encoder's own (stricter, canonical) check instead of being silently
/// dropped here per the documented "typed wins" collision policy.
fn is_st0601_typed_tag(tag: u32) -> bool {
    matches!(tag, 1..=65 | 67..=80 | 82..=95 | 97..=101 | 106..=108 | 129 | 135)
}

/// ST 0903.6 VMTI LS typed tags: 1 (Checksum), 2..=13, 101..=103.
/// Tag 7 is deprecated in v6; treat it as typed so a colliding user
/// `unknown` entry doesn't sneak it back in.
fn is_st0903_vmti_typed_tag(tag: u32) -> bool {
    matches!(tag, 1..=13 | 101..=103)
}

/// ST 0903.6 VTargetPack typed tags: 1..=23 + 100..=107.
fn is_st0903_vtarget_typed_tag(tag: u32) -> bool {
    matches!(tag, 1..=23 | 100..=107)
}

// ---------------------------------------------------------------------------
// ST 0102 — Security Metadata LS
// ---------------------------------------------------------------------------

fn convert_security_classification(py: Python<'_>, v: RustSecCls) -> PyResult<PyObject> {
    match v {
        RustSecCls::Unknown(b) => Ok(b.into_py(py)),
        known => {
            let klv_mod = py.import_bound("tstrans.klv")?;
            let cls = klv_mod.getattr(intern!(py, "SecurityClassification"))?;
            Ok(cls.call1((known.to_u8(),))?.unbind())
        }
    }
}

fn convert_classifying_country(py: Python<'_>, v: RustClsCountry) -> PyResult<PyObject> {
    match v {
        RustClsCountry::Unknown(b) => Ok(b.into_py(py)),
        known => {
            let klv_mod = py.import_bound("tstrans.klv")?;
            let cls = klv_mod.getattr(intern!(py, "ClassifyingCountryCodingMethod"))?;
            Ok(cls.call1((known.to_u8(),))?.unbind())
        }
    }
}

fn convert_object_country(py: Python<'_>, v: RustObjCountry) -> PyResult<PyObject> {
    match v {
        RustObjCountry::Unknown(b) => Ok(b.into_py(py)),
        known => {
            let klv_mod = py.import_bound("tstrans.klv")?;
            let cls = klv_mod.getattr(intern!(py, "ObjectCountryCodingMethod"))?;
            Ok(cls.call1((known.to_u8(),))?.unbind())
        }
    }
}

fn convert_security_ls(py: Python<'_>, sec: &SecurityLs) -> PyResult<PyObject> {
    let klv_mod = py.import_bound("tstrans.klv")?;
    let cls = klv_mod.getattr(intern!(py, "SecurityLs"))?;
    let kwargs = PyDict::new_bound(py);

    if let Some(v) = sec.security_classification {
        kwargs.set_item(
            "security_classification",
            convert_security_classification(py, v)?,
        )?;
    }
    if let Some(v) = sec.classifying_country_coding_method {
        kwargs.set_item(
            "classifying_country_coding_method",
            convert_classifying_country(py, v)?,
        )?;
    }
    if let Some(v) = &sec.classifying_country {
        kwargs.set_item("classifying_country", v.as_str())?;
    }
    if let Some(v) = sec.object_country_coding_method {
        kwargs.set_item(
            "object_country_coding_method",
            convert_object_country(py, v)?,
        )?;
    }
    if let Some(v) = &sec.object_country_codes {
        kwargs.set_item("object_country_codes", v.as_str())?;
    }
    if let Some(v) = sec.version {
        kwargs.set_item("version", v)?;
    }
    if let Some(v) = &sec.sci_shi_info {
        kwargs.set_item("sci_shi_info", v.as_str())?;
    }
    if let Some(v) = &sec.caveats {
        kwargs.set_item("caveats", v.as_str())?;
    }
    if let Some(v) = &sec.releasing_instructions {
        kwargs.set_item("releasing_instructions", v.as_str())?;
    }
    if let Some(v) = &sec.classified_by {
        kwargs.set_item("classified_by", v.as_str())?;
    }
    if let Some(v) = &sec.derived_from {
        kwargs.set_item("derived_from", v.as_str())?;
    }
    if let Some(v) = &sec.classification_reason {
        kwargs.set_item("classification_reason", v.as_str())?;
    }
    if let Some(v) = &sec.declassification_date {
        kwargs.set_item("declassification_date", v.as_str())?;
    }
    if let Some(v) = &sec.classification_marking_system {
        kwargs.set_item("classification_marking_system", v.as_str())?;
    }
    if let Some(v) = &sec.classification_comments {
        kwargs.set_item("classification_comments", v.as_str())?;
    }
    if let Some(v) = &sec.classifying_country_coding_method_version_date {
        kwargs.set_item("classifying_country_coding_method_version_date", v.as_str())?;
    }
    if let Some(v) = &sec.object_country_coding_method_version_date {
        kwargs.set_item("object_country_coding_method_version_date", v.as_str())?;
    }
    kwargs.set_item("unknown", convert_unknown(py, &sec.unknown)?)?;
    kwargs.set_item("field_errors", convert_field_errors(py, &sec.field_errors)?)?;

    Ok(cls.call((), Some(&kwargs))?.unbind())
}

/// Decode an ST 0102 Security LS. `buf` is **body-only** (no UL / outer
/// BER length wrapper). With `strict=True`, rejects missing required
/// tags + unknown codepoints + non-canonical BER + malformed UTF-16.
#[pyfunction]
#[pyo3(name = "decode_security", signature = (buf, *, strict = false))]
fn decode_security_py(py: Python<'_>, buf: &[u8], strict: bool) -> PyResult<PyObject> {
    // Audit #11 / GIL-release decision: NOT wrapped — same rationale as
    // `decode_precision_timestamp_py`. ST 0102 records are typically
    // 20-200 bytes, well under the GIL-transition breakeven.
    let result = if strict {
        decode_st0102_strict(buf)
    } else {
        decode_st0102_lenient(buf)
    };
    match result {
        Ok(sec) => convert_security_ls(py, &sec),
        Err(e) => Err(klv_decode_error_to_pyerr(py, e)),
    }
}

// ---------------------------------------------------------------------------
// ST 0102 — Python → Rust inverse translator + encode entry point
// ---------------------------------------------------------------------------

/// Extract a u8 codepoint from a Python field that may be either an
/// `enum.Enum` instance (with `.value: int`) or a raw `int` (Unknown
/// codepoint pass-through). Mirrors the asymmetric forward translator
/// (`convert_security_classification` / `convert_classifying_country` /
/// `convert_object_country`) which emits a typed enum for known
/// codepoints and a raw int for `Unknown(b)`.
fn enum_field_to_u8(p: &Bound<'_, PyAny>) -> PyResult<u8> {
    if let Ok(value_attr) = p.getattr("value") {
        // enum.Enum instance — pull `.value`
        value_attr.extract()
    } else {
        // raw int — extract directly
        p.extract()
    }
}

/// Inverse of `convert_security_ls`: extracts every field from a Python
/// `tstrans.klv.SecurityLs` dataclass into a Rust `SecurityLs`.
///
/// `field_errors` is a parser-only diagnostic and is not round-tripped.
///
/// `unknown` IS round-tripped: forward-compat TLVs the decoder preserved
/// are forwarded into the encoder so `decode -> encode -> decode` is
/// lossless (audit #5). Entries whose tag collides with a typed field
/// (see `is_st0102_typed_tag`) are silently dropped — typed wins.
fn py_to_security_ls(p: &Bound<'_, PyAny>) -> PyResult<SecurityLs> {
    let mut r = SecurityLs::default();
    let py = p.py();

    // 3 typed-enum fields: Python sends either an enum instance OR a raw int
    // (Unknown codepoint). Use enum_field_to_u8 to coerce both shapes.
    let sc_obj = p.getattr(intern!(py, "security_classification"))?;
    if !sc_obj.is_none() {
        r.security_classification = Some(RustSecCls::from_u8(enum_field_to_u8(&sc_obj)?));
    }
    let cc_obj = p.getattr(intern!(py, "classifying_country_coding_method"))?;
    if !cc_obj.is_none() {
        r.classifying_country_coding_method =
            Some(RustClsCountry::from_u8(enum_field_to_u8(&cc_obj)?));
    }
    let oc_obj = p.getattr(intern!(py, "object_country_coding_method"))?;
    if !oc_obj.is_none() {
        r.object_country_coding_method = Some(RustObjCountry::from_u8(enum_field_to_u8(&oc_obj)?));
    }

    // Optional<String> fields (15 of them) + Optional<u16> for version.
    macro_rules! os {
        ($field:ident) => {
            if let Some(s) = p
                .getattr(intern!(py, stringify!($field)))?
                .extract::<Option<String>>()?
            {
                r.$field = Some(s);
            }
        };
    }
    macro_rules! op {
        ($field:ident, $ty:ty) => {
            if let Some(v) = p
                .getattr(intern!(py, stringify!($field)))?
                .extract::<Option<$ty>>()?
            {
                r.$field = Some(v);
            }
        };
    }

    os!(classifying_country);
    os!(object_country_codes);
    op!(version, u16);
    os!(sci_shi_info);
    os!(caveats);
    os!(releasing_instructions);
    os!(classified_by);
    os!(derived_from);
    os!(classification_reason);
    os!(declassification_date);
    os!(classification_marking_system);
    os!(classification_comments);
    os!(classifying_country_coding_method_version_date);
    os!(object_country_coding_method_version_date);

    r.unknown = py_to_unknown(p, is_st0102_typed_tag)?;

    Ok(r)
}

/// Encode a Python `SecurityLs` to wire bytes (lenient — emits only the
/// populated fields, no mandatory-tag enforcement). Returns `bytes`
/// containing the ST 0102 body (no outer UL / BER length wrapper).
#[pyfunction]
#[pyo3(name = "encode_security")]
fn encode_security_py(py: Python<'_>, record: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    let rust_rec = py_to_security_ls(record)?;
    let bytes = encode_st0102(&rust_rec).map_err(|e| klv_encode_error_to_pyerr(py, e))?;
    Ok(pyo3::types::PyBytes::new_bound(py, &bytes).unbind().into())
}

// ---------------------------------------------------------------------------
// ST 0605 — Python → Rust inverse translator + encode entry point
// ---------------------------------------------------------------------------

/// Inverse of `convert_precision_timestamp_pack`.
fn py_to_precision_timestamp_pack(p: &Bound<'_, PyAny>) -> PyResult<PrecisionTimeStampPack> {
    let py = p.py();
    let ts_obj = p.getattr(intern!(py, "time_status"))?;
    let raw: u8 = ts_obj.getattr(intern!(py, "raw"))?.extract()?;
    let timestamp_us: u64 = p.getattr(intern!(py, "timestamp_us"))?.extract()?;
    Ok(PrecisionTimeStampPack {
        time_status: RustTimeStatus(raw),
        timestamp_us,
    })
}

/// Encode a Python `PrecisionTimeStampPack` to the 26-byte wire form
/// (16-byte UL + 1-byte BER length + 1-byte TimeStatus + 8-byte BE
/// microsecond timestamp). Returns `bytes`.
#[pyfunction]
#[pyo3(name = "encode_precision_timestamp")]
fn encode_precision_timestamp_py(py: Python<'_>, pack: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    let rust_pack = py_to_precision_timestamp_pack(pack)?;
    let bytes = encode_st0605(&rust_pack);
    Ok(pyo3::types::PyBytes::new_bound(py, &bytes).unbind().into())
}

// ---------------------------------------------------------------------------
// ST 0903 — VTargetPack translator (entry points further down)
// ---------------------------------------------------------------------------

/// Translate a Rust `VTargetPack` to a Python `tstrans.klv.VTargetPack`
/// dataclass instance.
fn convert_vtarget_pack(py: Python<'_>, p: &RustVTargetPack) -> PyResult<PyObject> {
    let klv_mod = py.import_bound("tstrans.klv")?;
    let cls = klv_mod.getattr(intern!(py, "VTargetPack"))?;
    let kwargs = PyDict::new_bound(py);

    kwargs.set_item("target_id", p.target_id)?;

    macro_rules! opt_set {
        ($key:expr, $val:expr) => {
            if let Some(v) = $val {
                kwargs.set_item($key, v)?;
            }
        };
    }
    // Optional<Vec<u8>> — emit as Python `bytes` (not list[int]); the
    // Python dataclass fields are typed `bytes | None`. Matches the
    // `ob!` macro in `convert_uas_datalink_ls`.
    macro_rules! opt_set_ref {
        ($key:expr, $val:expr) => {
            if let Some(v) = $val.as_ref() {
                kwargs.set_item($key, pyo3::types::PyBytes::new_bound(py, v.as_slice()))?;
            }
        };
    }

    opt_set!("centroid_pixel", p.centroid_pixel);
    opt_set!("bbox_top_left_pixel", p.bbox_top_left_pixel);
    opt_set!("bbox_bottom_right_pixel", p.bbox_bottom_right_pixel);
    opt_set!("priority", p.priority);
    opt_set!("confidence_level", p.confidence_level);
    opt_set!("history", p.history);
    opt_set!("percentage_of_target_pixels", p.percentage_of_target_pixels);

    if let Some([r, g, b]) = p.target_color {
        kwargs.set_item(
            "target_color",
            pyo3::types::PyTuple::new_bound(py, [r, g, b]),
        )?;
    }

    opt_set!("target_intensity", p.target_intensity);
    opt_set!("centroid_lat_offset", p.centroid_lat_offset);
    opt_set!("centroid_lon_offset", p.centroid_lon_offset);
    opt_set!("centroid_hae", p.centroid_hae);
    opt_set!("bbox_top_left_lat_offset", p.bbox_top_left_lat_offset);
    opt_set!("bbox_top_left_lon_offset", p.bbox_top_left_lon_offset);
    opt_set!(
        "bbox_bottom_right_lat_offset",
        p.bbox_bottom_right_lat_offset
    );
    opt_set!(
        "bbox_bottom_right_lon_offset",
        p.bbox_bottom_right_lon_offset
    );
    opt_set_ref!("target_location", p.target_location);
    opt_set_ref!("geospatial_contour_series", p.geospatial_contour_series);
    opt_set!("centroid_pix_row", p.centroid_pix_row);
    opt_set!("centroid_pix_col", p.centroid_pix_col);
    opt_set!("algorithm_id", p.algorithm_id);
    opt_set!("detection_status", p.detection_status);
    opt_set_ref!("vmask", p.vmask);
    opt_set_ref!("vtracker", p.vtracker);
    opt_set_ref!("vchip", p.vchip);
    opt_set_ref!("vchip_series", p.vchip_series);
    opt_set_ref!("vobject_series", p.vobject_series);

    kwargs.set_item("unknown", convert_unknown(py, &p.unknown)?)?;
    kwargs.set_item("field_errors", convert_field_errors(py, &p.field_errors)?)?;

    Ok(cls.call((), Some(&kwargs))?.unbind())
}

// ---------------------------------------------------------------------------
// ST 0903 — VmtiLs entry point
// ---------------------------------------------------------------------------

/// Translate a Rust `VmtiLs` to a Python `tstrans.klv.VmtiLs`
/// dataclass instance.
fn convert_vmti_ls(py: Python<'_>, v: &VmtiLs) -> PyResult<PyObject> {
    let klv_mod = py.import_bound("tstrans.klv")?;
    let cls = klv_mod.getattr(intern!(py, "VmtiLs"))?;
    let kwargs = PyDict::new_bound(py);

    if let Some(c) = v.checksum {
        kwargs.set_item("checksum", c)?;
    }
    if let Some(t) = v.precision_time_stamp {
        kwargs.set_item("precision_time_stamp", t)?;
    }
    if let Some(s) = &v.vmti_system_name {
        kwargs.set_item("vmti_system_name", s.as_str())?;
    }
    if let Some(n) = v.version_number {
        kwargs.set_item("version_number", n)?;
    }
    if let Some(n) = v.total_targets_in_frame {
        kwargs.set_item("total_targets_in_frame", n)?;
    }
    if let Some(n) = v.num_targets_reported {
        kwargs.set_item("num_targets_reported", n)?;
    }
    if let Some(n) = v.frame_width {
        kwargs.set_item("frame_width", n)?;
    }
    if let Some(n) = v.frame_height {
        kwargs.set_item("frame_height", n)?;
    }
    if let Some(s) = &v.source_sensor {
        kwargs.set_item("source_sensor", s.as_str())?;
    }
    if let Some(f) = v.horizontal_fov {
        kwargs.set_item("horizontal_fov", f)?;
    }
    if let Some(f) = v.vertical_fov {
        kwargs.set_item("vertical_fov", f)?;
    }
    if let Some(m) = v.miis_id.as_ref() {
        kwargs.set_item("miis_id", pyo3::types::PyBytes::new_bound(py, m.as_slice()))?;
    }
    if let Some(b) = v.algorithm_series.as_ref() {
        kwargs.set_item(
            "algorithm_series",
            pyo3::types::PyBytes::new_bound(py, b.as_slice()),
        )?;
    }
    if let Some(b) = v.ontology_series.as_ref() {
        kwargs.set_item(
            "ontology_series",
            pyo3::types::PyBytes::new_bound(py, b.as_slice()),
        )?;
    }
    let targets: Vec<PyObject> = v
        .targets
        .iter()
        .map(|t| convert_vtarget_pack(py, t))
        .collect::<PyResult<Vec<_>>>()?;
    kwargs.set_item("targets", pyo3::types::PyTuple::new_bound(py, targets))?;
    kwargs.set_item("unknown", convert_unknown(py, &v.unknown)?)?;
    kwargs.set_item("field_errors", convert_field_errors(py, &v.field_errors)?)?;

    Ok(cls.call((), Some(&kwargs))?.unbind())
}

/// Decode an ST 0903 VMTI LS. `buf` is **body-only** (no UL / outer
/// BER length wrapper). With `strict=True`, rejects missing required
/// tags per ST 0903.6 §6 Table 1, duplicate tags, malformed UTF-8.
#[pyfunction]
#[pyo3(name = "decode_vmti", signature = (buf, *, strict = false))]
fn decode_vmti_py(py: Python<'_>, buf: &[u8], strict: bool) -> PyResult<PyObject> {
    // Audit #11 / GIL-release decision: NOT wrapped — same rationale as
    // `decode_precision_timestamp_py`. VMTI records can be large with
    // many targets, but the per-target convert step (which builds Py
    // objects) keeps cumulative GIL hold low even for big records.
    // If a future profile shows VMTI workloads benefiting from release,
    // re-evaluate at that point.
    let result = if strict {
        decode_st0903_strict(buf)
    } else {
        decode_st0903_lenient(buf)
    };
    match result {
        Ok(vmti) => convert_vmti_ls(py, &vmti),
        Err(e) => Err(klv_decode_error_to_pyerr(py, e)),
    }
}

// ---------------------------------------------------------------------------
// ST 0903 — Python → Rust inverse translators + encode entry points
// ---------------------------------------------------------------------------

/// Inverse of `convert_vtarget_pack`.
///
/// `field_errors` is a parser-only diagnostic and is not round-tripped.
///
/// `unknown` IS round-tripped (audit #5): forward-compat TLVs preserved
/// by the VTargetPack decoder flow back into the encoder. Entries whose
/// tag collides with a typed field (see `is_st0903_vtarget_typed_tag`)
/// are silently dropped — typed wins.
#[allow(clippy::cognitive_complexity)]
fn py_to_vtarget_pack(p: &Bound<'_, PyAny>) -> PyResult<RustVTargetPack> {
    let mut r = RustVTargetPack::default();
    let py = p.py();

    // BER-OID `target_id` (mandatory, no Tag).
    r.target_id = p.getattr(intern!(py, "target_id"))?.extract::<u64>()?;

    macro_rules! op {
        ($field:ident, $ty:ty) => {
            if let Some(v) = p
                .getattr(intern!(py, stringify!($field)))?
                .extract::<Option<$ty>>()?
            {
                r.$field = Some(v);
            }
        };
    }
    macro_rules! ob {
        ($field:ident) => {
            if let Some(v) = p
                .getattr(intern!(py, stringify!($field)))?
                .extract::<Option<Vec<u8>>>()?
            {
                r.$field = Some(v);
            }
        };
    }

    op!(centroid_pixel, u64);
    op!(bbox_top_left_pixel, u64);
    op!(bbox_bottom_right_pixel, u64);
    op!(priority, u8);
    op!(confidence_level, u8);
    op!(history, u16);
    op!(percentage_of_target_pixels, u8);

    // target_color: Optional<tuple[int, int, int]> → Option<[u8; 3]>.
    // `None` is a valid value — the field is simply absent from the LS.
    // A non-None value with the wrong tuple length is a caller bug; raise
    // instead of silently dropping (audit #6).
    let tc = p.getattr(intern!(py, "target_color"))?;
    if !tc.is_none() {
        let arr: Vec<u8> = tc.extract()?;
        if arr.len() != 3 {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "target_color must be a 3-tuple, got length {}",
                arr.len()
            )));
        }
        r.target_color = Some([arr[0], arr[1], arr[2]]);
    }

    op!(target_intensity, u32);
    op!(centroid_lat_offset, f64);
    op!(centroid_lon_offset, f64);
    op!(centroid_hae, f64);
    op!(bbox_top_left_lat_offset, f64);
    op!(bbox_top_left_lon_offset, f64);
    op!(bbox_bottom_right_lat_offset, f64);
    op!(bbox_bottom_right_lon_offset, f64);
    ob!(target_location);
    ob!(geospatial_contour_series);
    op!(centroid_pix_row, u64);
    op!(centroid_pix_col, u64);
    op!(algorithm_id, u32);
    op!(detection_status, u8);
    ob!(vmask);
    ob!(vtracker);
    ob!(vchip);
    ob!(vchip_series);
    ob!(vobject_series);

    r.unknown = py_to_unknown(p, is_st0903_vtarget_typed_tag)?;

    Ok(r)
}

/// Inverse of `convert_vmti_ls`.
///
/// `field_errors` is parser-only and is not round-tripped.
///
/// `unknown` IS round-tripped (audit #5): forward-compat TLVs preserved
/// by the VMTI LS decoder flow back into the encoder. Entries whose tag
/// collides with a typed field (see `is_st0903_vmti_typed_tag`) are
/// silently dropped — typed wins.
fn py_to_vmti_ls(p: &Bound<'_, PyAny>) -> PyResult<VmtiLs> {
    let mut r = VmtiLs::default();
    let py = p.py();

    macro_rules! op {
        ($field:ident, $ty:ty) => {
            if let Some(v) = p
                .getattr(intern!(py, stringify!($field)))?
                .extract::<Option<$ty>>()?
            {
                r.$field = Some(v);
            }
        };
    }
    macro_rules! os {
        ($field:ident) => {
            if let Some(s) = p
                .getattr(intern!(py, stringify!($field)))?
                .extract::<Option<String>>()?
            {
                r.$field = Some(s);
            }
        };
    }
    macro_rules! ob {
        ($field:ident) => {
            if let Some(v) = p
                .getattr(intern!(py, stringify!($field)))?
                .extract::<Option<Vec<u8>>>()?
            {
                r.$field = Some(v);
            }
        };
    }

    // Note: `checksum` is intentionally ignored by `encode_standalone`
    // (it computes a fresh substrate checksum from the framing) and
    // dropped by `encode` (embedded-VMTI per ST 0903.6-120). We still
    // populate the Rust field for symmetry — encoders consult their own
    // policy.
    op!(checksum, u16);
    op!(precision_time_stamp, u64);
    os!(vmti_system_name);
    op!(version_number, u16);
    op!(total_targets_in_frame, u32);
    op!(num_targets_reported, u32);
    op!(frame_width, u32);
    op!(frame_height, u32);
    os!(source_sensor);
    op!(horizontal_fov, f64);
    op!(vertical_fov, f64);
    ob!(miis_id);
    ob!(algorithm_series);
    ob!(ontology_series);

    // targets: tuple[VTargetPack, ...] → Vec<VTargetPack>. The Python
    // dataclass uses a tuple (frozen + hashable); iterate generically.
    let targets_obj = p.getattr(intern!(py, "targets"))?;
    for t in targets_obj.iter()? {
        let t = t?;
        r.targets.push(py_to_vtarget_pack(&t)?);
    }

    r.unknown = py_to_unknown(p, is_st0903_vmti_typed_tag)?;

    Ok(r)
}

/// Encode a Python `VmtiLs` to wire bytes — VMTI LS **body only** (no UL
/// / outer BER length wrapper / no Tag 1 checksum per ST 0903.6-120).
/// Use for embedded-VMTI carried inside MPEG-TS via tst-pipeline. Returns
/// `bytes`.
#[pyfunction]
#[pyo3(name = "encode_vmti")]
fn encode_vmti_py(py: Python<'_>, record: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    let rust_rec = py_to_vmti_ls(record)?;
    let bytes = encode_st0903(&rust_rec).map_err(|e| klv_encode_error_to_pyerr(py, e))?;
    Ok(pyo3::types::PyBytes::new_bound(py, &bytes).unbind().into())
}

/// Encode a Python `VmtiLs` as a **standalone-VMTI** wire record:
/// `[VMTI_LS_UL:16][outer BER length][body][Tag 1 checkSum TLV]` per
/// ST 0903.4-17 / ST 0903.6-119. The Tag 1 checksum is computed from
/// the assembled framing; any value in `VmtiLs.checksum` is ignored.
/// Returns `bytes`.
#[pyfunction]
#[pyo3(name = "encode_vmti_standalone")]
fn encode_vmti_standalone_py(py: Python<'_>, record: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    let rust_rec = py_to_vmti_ls(record)?;
    let bytes =
        encode_st0903_standalone(&rust_rec).map_err(|e| klv_encode_error_to_pyerr(py, e))?;
    Ok(pyo3::types::PyBytes::new_bound(py, &bytes).unbind().into())
}

/// Encode a Python `SecurityLs` to wire bytes with strict compliance per
/// ST 0102.12 — requires Tags 1 (Security Classification), 2 (Country
/// Coding Method), 3 (Classifying Country), 12 (Object Country Coding
/// Method), 13 (Object Country Codes), and 22 (Version). Raises
/// `KlvEncodeError(MISSING_MANDATORY_ITEM)` if any required tag is absent.
#[pyfunction]
#[pyo3(name = "encode_security_strict_compliance")]
fn encode_security_strict_compliance_py(
    py: Python<'_>,
    record: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let rust_rec = py_to_security_ls(record)?;
    let bytes =
        encode_st0102_strict_compliance(&rust_rec).map_err(|e| klv_encode_error_to_pyerr(py, e))?;
    Ok(pyo3::types::PyBytes::new_bound(py, &bytes).unbind().into())
}

/// Encode a Python `VmtiLs` to wire bytes with strict compliance for
/// **embedded** VMTI carriage — requires Tags 4 (version) and 6
/// (num_targets_reported); all VTargetPacks must be non-empty and have
/// unique target_ids. Raises `KlvEncodeError` on validation failure.
#[pyfunction]
#[pyo3(name = "encode_vmti_strict_compliance")]
fn encode_vmti_strict_compliance_py(
    py: Python<'_>,
    record: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let rust_rec = py_to_vmti_ls(record)?;
    let bytes =
        encode_st0903_strict_compliance(&rust_rec).map_err(|e| klv_encode_error_to_pyerr(py, e))?;
    Ok(pyo3::types::PyBytes::new_bound(py, &bytes).unbind().into())
}

/// Encode a Python `VmtiLs` as a **standalone** wire record with strict
/// compliance — all checks from `encode_vmti_strict_compliance` plus
/// standalone-required Tags 2 (precision_time_stamp), 11 (horizontal_fov),
/// 12 (vertical_fov), 13 (miis_id), and forbids per-pack offset tags
/// (10/11/13/14/15/16). Raises `KlvEncodeError` on validation failure.
#[pyfunction]
#[pyo3(name = "encode_vmti_standalone_strict_compliance")]
fn encode_vmti_standalone_strict_compliance_py(
    py: Python<'_>,
    record: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let rust_rec = py_to_vmti_ls(record)?;
    let bytes = encode_st0903_standalone_strict_compliance(&rust_rec)
        .map_err(|e| klv_encode_error_to_pyerr(py, e))?;
    Ok(pyo3::types::PyBytes::new_bound(py, &bytes).unbind().into())
}

// ---------------------------------------------------------------------------
// ST 0601 — Tags 34/63/77 coded enums (WP-A Table A3)
// ---------------------------------------------------------------------------
//
// `IcingDetected::to_wire`/`from_wire` (and the SensorFovName /
// OperationalMode equivalents) are `pub(crate)`-scoped to tst-core, so the
// tiny wire-code tables are duplicated locally here — same rationale as the
// `is_st0601_typed_tag`-family predicates above (narrow inventories kept
// local rather than threading internal Rust APIs out of tst-core).

/// Translate a Rust `IcingDetected` to the matching Python
/// `tstrans.klv.IcingDetected` enum instance for known codepoints, or a
/// raw `int` for `Other(code)` (wire-unknown, round-trips byte-exact) —
/// mirrors the ST 0102 `SecurityClassification::Unknown(b)` asymmetry.
fn convert_icing_detected(py: Python<'_>, v: RustIcingDetected) -> PyResult<PyObject> {
    let code = match v {
        RustIcingDetected::DetectorOff => 0u8,
        RustIcingDetected::NoIcingDetected => 1,
        RustIcingDetected::IcingDetected => 2,
        RustIcingDetected::Other(b) => return Ok(b.into_py(py)),
        // #[non_exhaustive] in tst-core: a wildcard is required even though
        // every current variant is covered above.
        _ => {
            return Err(PyValueError::new_err(
                "unknown IcingDetected variant crossing the binding",
            ));
        }
    };
    let klv_mod = py.import_bound("tstrans.klv")?;
    let cls = klv_mod.getattr(intern!(py, "IcingDetected"))?;
    Ok(cls.call1((code,))?.unbind())
}

/// Inverse of `convert_icing_detected`.
fn icing_detected_from_wire(b: u8) -> RustIcingDetected {
    match b {
        0 => RustIcingDetected::DetectorOff,
        1 => RustIcingDetected::NoIcingDetected,
        2 => RustIcingDetected::IcingDetected,
        other => RustIcingDetected::Other(other),
    }
}

/// Translate a Rust `SensorFovName` to the matching Python
/// `tstrans.klv.SensorFovName` enum instance for known codepoints, or a
/// raw `int` for `Other(code)`.
fn convert_sensor_fov_name(py: Python<'_>, v: RustSensorFovName) -> PyResult<PyObject> {
    let code = match v {
        RustSensorFovName::Ultranarrow => 0u8,
        RustSensorFovName::Narrow => 1,
        RustSensorFovName::Medium => 2,
        RustSensorFovName::Wide => 3,
        RustSensorFovName::Ultrawide => 4,
        RustSensorFovName::NarrowMedium => 5,
        RustSensorFovName::TwoXUltranarrow => 6,
        RustSensorFovName::FourXUltranarrow => 7,
        RustSensorFovName::ContinuousZoom => 8,
        RustSensorFovName::Other(b) => return Ok(b.into_py(py)),
        _ => {
            return Err(PyValueError::new_err(
                "unknown SensorFovName variant crossing the binding",
            ));
        }
    };
    let klv_mod = py.import_bound("tstrans.klv")?;
    let cls = klv_mod.getattr(intern!(py, "SensorFovName"))?;
    Ok(cls.call1((code,))?.unbind())
}

/// Inverse of `convert_sensor_fov_name`.
fn sensor_fov_name_from_wire(b: u8) -> RustSensorFovName {
    match b {
        0 => RustSensorFovName::Ultranarrow,
        1 => RustSensorFovName::Narrow,
        2 => RustSensorFovName::Medium,
        3 => RustSensorFovName::Wide,
        4 => RustSensorFovName::Ultrawide,
        5 => RustSensorFovName::NarrowMedium,
        6 => RustSensorFovName::TwoXUltranarrow,
        7 => RustSensorFovName::FourXUltranarrow,
        8 => RustSensorFovName::ContinuousZoom,
        other => RustSensorFovName::Other(other),
    }
}

/// Translate a Rust `OperationalMode` to the matching Python
/// `tstrans.klv.OperationalMode` enum instance for known codepoints, or a
/// raw `int` for `Other(code)`.
fn convert_operational_mode(py: Python<'_>, v: RustOperationalMode) -> PyResult<PyObject> {
    let code = match v {
        RustOperationalMode::OtherMode => 0u8,
        RustOperationalMode::Operational => 1,
        RustOperationalMode::Training => 2,
        RustOperationalMode::Exercise => 3,
        RustOperationalMode::Maintenance => 4,
        RustOperationalMode::Test => 5,
        RustOperationalMode::Other(b) => return Ok(b.into_py(py)),
        _ => {
            return Err(PyValueError::new_err(
                "unknown OperationalMode variant crossing the binding",
            ));
        }
    };
    let klv_mod = py.import_bound("tstrans.klv")?;
    let cls = klv_mod.getattr(intern!(py, "OperationalMode"))?;
    Ok(cls.call1((code,))?.unbind())
}

/// Inverse of `convert_operational_mode`.
fn operational_mode_from_wire(b: u8) -> RustOperationalMode {
    match b {
        0 => RustOperationalMode::OtherMode,
        1 => RustOperationalMode::Operational,
        2 => RustOperationalMode::Training,
        3 => RustOperationalMode::Exercise,
        4 => RustOperationalMode::Maintenance,
        5 => RustOperationalMode::Test,
        other => RustOperationalMode::Other(other),
    }
}

// ---------------------------------------------------------------------------
// ST 0601 — UAS Datalink LS
// ---------------------------------------------------------------------------

/// Translate a Rust `UasDatalinkLs` to a Python `tstrans.klv.UasDatalinkLs`
/// dataclass instance. Mechanical 107-field projection.
#[allow(clippy::cognitive_complexity)]
fn convert_uas_datalink_ls(py: Python<'_>, r: &UasDatalinkLs) -> PyResult<PyObject> {
    let klv_mod = py.import_bound("tstrans.klv")?;
    let cls = klv_mod.getattr(intern!(py, "UasDatalinkLs"))?;
    let kwargs = PyDict::new_bound(py);

    kwargs.set_item(
        "universal_label",
        pyo3::types::PyBytes::new_bound(py, &r.universal_label.0),
    )?;
    kwargs.set_item("declared_version", r.declared_version)?;

    // Optional<String>
    macro_rules! os {
        ($k:expr, $v:expr) => {
            if let Some(s) = $v.as_ref() {
                kwargs.set_item($k, s.as_str())?;
            }
        };
    }
    // Optional<scalar> (Copy)
    macro_rules! op {
        ($k:expr, $v:expr) => {
            if let Some(v) = $v {
                kwargs.set_item($k, v)?;
            }
        };
    }
    // Optional<Vec<u8>> — emit as Python `bytes` (not list[int]); the
    // Python dataclass field is typed `bytes | None` and downstream
    // typed decoders (`decode_security` / `decode_vmti`) take `bytes`.
    macro_rules! ob {
        ($k:expr, $v:expr) => {
            if let Some(b) = $v.as_ref() {
                kwargs.set_item($k, pyo3::types::PyBytes::new_bound(py, b.as_slice()))?;
            }
        };
    }

    os!("mission_id", r.mission_id);
    os!("platform_tail_number", r.platform_tail_number);
    os!("platform_designation", r.platform_designation);
    os!("image_source_sensor", r.image_source_sensor);
    os!("image_coordinate_system", r.image_coordinate_system);
    os!("platform_call_sign", r.platform_call_sign);
    op!("uas_ls_version", r.uas_ls_version);
    op!("timestamp_us", r.timestamp_us);
    op!("platform_heading_deg", r.platform_heading_deg);
    op!("platform_pitch_deg", r.platform_pitch_deg);
    op!("platform_roll_deg", r.platform_roll_deg);
    op!("platform_true_airspeed", r.platform_true_airspeed);
    op!("platform_indicated_airspeed", r.platform_indicated_airspeed);
    op!("platform_pitch_full_deg", r.platform_pitch_full_deg);
    op!("platform_roll_full_deg", r.platform_roll_full_deg);
    op!(
        "platform_angle_of_attack_deg",
        r.platform_angle_of_attack_deg
    );
    op!("sensor_lat_deg", r.sensor_lat_deg);
    op!("sensor_lon_deg", r.sensor_lon_deg);
    op!("sensor_alt_m", r.sensor_alt_m);
    op!("sensor_ellipsoid_height_m", r.sensor_ellipsoid_height_m);
    op!("sensor_hfov_deg", r.sensor_hfov_deg);
    op!("sensor_vfov_deg", r.sensor_vfov_deg);
    op!("sensor_rel_az_deg", r.sensor_rel_az_deg);
    op!("sensor_rel_el_deg", r.sensor_rel_el_deg);
    op!("sensor_rel_roll_deg", r.sensor_rel_roll_deg);
    op!("slant_range_m", r.slant_range_m);
    op!("target_width_m", r.target_width_m);
    op!("frame_center_lat_deg", r.frame_center_lat_deg);
    op!("frame_center_lon_deg", r.frame_center_lon_deg);
    op!("frame_center_elev_m", r.frame_center_elev_m);
    op!(
        "frame_center_ellipsoid_height_m",
        r.frame_center_ellipsoid_height_m
    );
    op!("corner_lat_offset_p1_deg", r.corner_lat_offset_p1_deg);
    op!("corner_lon_offset_p1_deg", r.corner_lon_offset_p1_deg);
    op!("corner_lat_offset_p2_deg", r.corner_lat_offset_p2_deg);
    op!("corner_lon_offset_p2_deg", r.corner_lon_offset_p2_deg);
    op!("corner_lat_offset_p3_deg", r.corner_lat_offset_p3_deg);
    op!("corner_lon_offset_p3_deg", r.corner_lon_offset_p3_deg);
    op!("corner_lat_offset_p4_deg", r.corner_lat_offset_p4_deg);
    op!("corner_lon_offset_p4_deg", r.corner_lon_offset_p4_deg);
    op!("corner_lat_p1_deg", r.corner_lat_p1_deg);
    op!("corner_lon_p1_deg", r.corner_lon_p1_deg);
    op!("corner_lat_p2_deg", r.corner_lat_p2_deg);
    op!("corner_lon_p2_deg", r.corner_lon_p2_deg);
    op!("corner_lat_p3_deg", r.corner_lat_p3_deg);
    op!("corner_lon_p3_deg", r.corner_lon_p3_deg);
    op!("corner_lat_p4_deg", r.corner_lat_p4_deg);
    op!("corner_lon_p4_deg", r.corner_lon_p4_deg);
    op!("generic_flag_data", r.generic_flag_data);
    ob!("security_local_set", r.security_local_set);
    ob!("vmti", r.vmti);
    ob!("miis_core_id", r.miis_core_id);

    // WP-A Table A1 — ranged f64 fields (tags 40-46, 35-38/49/53-55,
    // 51/52/56-58/64/92/93, 67-69/71/76, 79-80).
    op!("target_location_lat_deg", r.target_location_lat_deg);
    op!("target_location_lon_deg", r.target_location_lon_deg);
    op!("target_location_elev_m", r.target_location_elev_m);
    op!("target_track_gate_width_px", r.target_track_gate_width_px);
    op!("target_track_gate_height_px", r.target_track_gate_height_px);
    op!("target_error_ce90_m", r.target_error_ce90_m);
    op!("target_error_le90_m", r.target_error_le90_m);
    op!("wind_direction_deg", r.wind_direction_deg);
    op!("wind_speed", r.wind_speed);
    op!("static_pressure_mbar", r.static_pressure_mbar);
    op!("density_altitude_m", r.density_altitude_m);
    op!("differential_pressure_mbar", r.differential_pressure_mbar);
    op!(
        "airfield_barometric_pressure_mbar",
        r.airfield_barometric_pressure_mbar
    );
    op!("airfield_elevation_m", r.airfield_elevation_m);
    op!("relative_humidity_pct", r.relative_humidity_pct);
    op!("platform_vertical_speed", r.platform_vertical_speed);
    op!("platform_sideslip_deg", r.platform_sideslip_deg);
    op!("platform_ground_speed", r.platform_ground_speed);
    op!("ground_range_m", r.ground_range_m);
    op!("platform_fuel_remaining_kg", r.platform_fuel_remaining_kg);
    op!(
        "platform_magnetic_heading_deg",
        r.platform_magnetic_heading_deg
    );
    op!(
        "platform_angle_of_attack_full_deg",
        r.platform_angle_of_attack_full_deg
    );
    op!("platform_sideslip_full_deg", r.platform_sideslip_full_deg);
    op!("alternate_platform_lat_deg", r.alternate_platform_lat_deg);
    op!("alternate_platform_lon_deg", r.alternate_platform_lon_deg);
    op!("alternate_platform_alt_m", r.alternate_platform_alt_m);
    op!(
        "alternate_platform_heading_deg",
        r.alternate_platform_heading_deg
    );
    op!(
        "alternate_platform_ellipsoid_height_m",
        r.alternate_platform_ellipsoid_height_m
    );
    op!("sensor_north_velocity", r.sensor_north_velocity);
    op!("sensor_east_velocity", r.sensor_east_velocity);

    // WP-A Table A4 — named nested-set raw fields (tags 73, 95, 97-101).
    ob!("rvt", r.rvt);
    ob!("sar_mi_local_set", r.sar_mi_local_set);
    ob!("range_image_local_set", r.range_image_local_set);
    ob!("geo_registration_local_set", r.geo_registration_local_set);
    ob!("composite_imaging_local_set", r.composite_imaging_local_set);
    ob!("segment_local_set", r.segment_local_set);
    ob!("amend_local_set", r.amend_local_set);

    // WP-A Table A2 — raw/simple scalar + string fields (tags 39, 60-62,
    // 70, 72, 106-108, 129, 135).
    op!("outside_air_temp_c", r.outside_air_temp_c);
    op!("weapon_load", r.weapon_load);
    op!("weapon_fired", r.weapon_fired);
    op!("laser_prf_code", r.laser_prf_code);
    os!("alternate_platform_name", r.alternate_platform_name);
    op!("event_start_time_us", r.event_start_time_us);
    os!("stream_designator", r.stream_designator);
    os!("operational_base", r.operational_base);
    os!("broadcast_source", r.broadcast_source);
    os!("target_id", r.target_id);
    os!("communications_method", r.communications_method);

    // WP-A Table A3 — coded enums (tags 34, 63, 77). Known codepoints
    // become a Python enum instance; wire-unknown `Other(code)` becomes a
    // raw int (mirrors the ST 0102 `SecurityClassification::Unknown(b)`
    // asymmetry).
    if let Some(v) = r.icing_detected {
        kwargs.set_item("icing_detected", convert_icing_detected(py, v)?)?;
    }
    if let Some(v) = r.sensor_fov_name {
        kwargs.set_item("sensor_fov_name", convert_sensor_fov_name(py, v)?)?;
    }
    if let Some(v) = r.operational_mode {
        kwargs.set_item("operational_mode", convert_operational_mode(py, v)?)?;
    }

    kwargs.set_item("unknown", convert_unknown(py, &r.unknown)?)?;
    kwargs.set_item("field_errors", convert_field_errors(py, &r.field_errors)?)?;
    let sentinel_tuple =
        pyo3::types::PyTuple::new_bound(py, r.sentinel_tags.iter().map(|&t| t as u64));
    kwargs.set_item("sentinel_tags", sentinel_tuple)?;

    Ok(cls.call((), Some(&kwargs))?.unbind())
}

/// Decode an ST 0601 UAS Datalink LS. `buf` is the full wire-format
/// payload starting with the 16-byte Universal Label.
///
/// - Default (lenient): any 16-byte UL accepted, checksum verified,
///   field-level malformations collected in `.field_errors`.
/// - `strict=True`: also requires the ST 0601 family UL pattern.
/// - `compliance=True`: also enforces Tag 2 first / Tag 1 last /
///   Tag 65 present / no duplicate tags / canonical BER. Implies
///   `strict=True`.
#[pyfunction]
#[pyo3(
    name = "decode_uas_datalink",
    signature = (buf, *, strict = false, compliance = false)
)]
fn decode_uas_datalink_py(
    py: Python<'_>,
    buf: &[u8],
    strict: bool,
    compliance: bool,
) -> PyResult<PyObject> {
    // Audit #11 / GIL-release decision: NOT wrapped — same rationale as
    // `decode_precision_timestamp_py`. Even at the upper end (~10 KB
    // record with 100 unknown TLVs), Rust decode is ~10us per call,
    // below the GIL transition breakeven. Worse, the 80-field
    // `convert_uas_datalink_ls` projection dominates per-call CPU
    // time, so releasing the GIL for the small Rust slice produces
    // GIL ping-pong under hot batch loops (measured: 30K decodes
    // taking 50+ seconds under contention vs 0.5s without the wrap).
    let result = if compliance {
        decode_st0601_strict_compliance(buf)
    } else if strict {
        decode_st0601_strict(buf)
    } else {
        decode_st0601_lenient(buf)
    };
    match result {
        Ok(rec) => convert_uas_datalink_ls(py, &rec),
        Err(e) => Err(klv_decode_error_to_pyerr(py, e)),
    }
}

// ---------------------------------------------------------------------------
// ST 0601 — Python → Rust inverse translator + encode entry points
// ---------------------------------------------------------------------------

/// Inverse of `convert_uas_datalink_ls`: extracts every field from a
/// Python `tstrans.klv.UasDatalinkLs` dataclass into a Rust
/// `UasDatalinkLs`.
///
/// `field_errors` is a parser-only diagnostic and is not round-tripped.
///
/// `unknown` IS round-tripped (audit #5): forward-compat TLVs preserved
/// by the ST 0601 decoder flow back into the encoder. Entries whose tag
/// collides with a typed field (see `is_st0601_typed_tag`) are silently
/// dropped — typed wins. Without this filter, ST 0601's encoder would
/// reject the call with `KlvEncodeError::ReservedTagInUnknown`; dropping
/// at the boundary lets `decode -> encode -> decode` succeed for any
/// record the decoder produced.
#[allow(clippy::cognitive_complexity)]
fn py_to_uas_datalink_ls(p: &Bound<'_, PyAny>) -> PyResult<UasDatalinkLs> {
    let mut r = UasDatalinkLs::default();

    // universal_label: 16-byte bytes → UniversalLabel. Any other length
    // is a caller bug; raise instead of silently leaving the field at the
    // default 16-byte zero UL (audit #6).
    let ul_bytes: Vec<u8> = p.getattr(intern!(p.py(), "universal_label"))?.extract()?;
    if ul_bytes.len() != 16 {
        return Err(pyo3::exceptions::PyValueError::new_err(format!(
            "universal_label must be 16 bytes, got {}",
            ul_bytes.len()
        )));
    }
    let mut ul = [0u8; 16];
    ul.copy_from_slice(&ul_bytes);
    r.universal_label = UniversalLabel(ul);

    // declared_version: u8 in Rust, int in Python
    r.declared_version = p.getattr(intern!(p.py(), "declared_version"))?.extract()?;

    // Optional<String>
    macro_rules! os {
        ($field:ident) => {
            if let Some(s) = p
                .getattr(intern!(p.py(), stringify!($field)))?
                .extract::<Option<String>>()?
            {
                r.$field = Some(s);
            }
        };
    }
    // Optional<scalar> — works for u8 / u64 / f64 fields
    macro_rules! op {
        ($field:ident, $ty:ty) => {
            if let Some(v) = p
                .getattr(intern!(p.py(), stringify!($field)))?
                .extract::<Option<$ty>>()?
            {
                r.$field = Some(v);
            }
        };
    }
    // Optional<Vec<u8>>
    macro_rules! ob {
        ($field:ident) => {
            if let Some(b) = p
                .getattr(intern!(p.py(), stringify!($field)))?
                .extract::<Option<Vec<u8>>>()?
            {
                r.$field = Some(b);
            }
        };
    }

    os!(mission_id);
    os!(platform_tail_number);
    os!(platform_designation);
    os!(image_source_sensor);
    os!(image_coordinate_system);
    os!(platform_call_sign);
    op!(uas_ls_version, u8);
    op!(timestamp_us, u64);
    op!(platform_heading_deg, f64);
    op!(platform_pitch_deg, f64);
    op!(platform_roll_deg, f64);
    op!(platform_true_airspeed, f64);
    op!(platform_indicated_airspeed, f64);
    op!(platform_pitch_full_deg, f64);
    op!(platform_roll_full_deg, f64);
    op!(platform_angle_of_attack_deg, f64);
    op!(sensor_lat_deg, f64);
    op!(sensor_lon_deg, f64);
    op!(sensor_alt_m, f64);
    op!(sensor_ellipsoid_height_m, f64);
    op!(sensor_hfov_deg, f64);
    op!(sensor_vfov_deg, f64);
    op!(sensor_rel_az_deg, f64);
    op!(sensor_rel_el_deg, f64);
    op!(sensor_rel_roll_deg, f64);
    op!(slant_range_m, f64);
    op!(target_width_m, f64);
    op!(frame_center_lat_deg, f64);
    op!(frame_center_lon_deg, f64);
    op!(frame_center_elev_m, f64);
    op!(frame_center_ellipsoid_height_m, f64);
    op!(corner_lat_offset_p1_deg, f64);
    op!(corner_lon_offset_p1_deg, f64);
    op!(corner_lat_offset_p2_deg, f64);
    op!(corner_lon_offset_p2_deg, f64);
    op!(corner_lat_offset_p3_deg, f64);
    op!(corner_lon_offset_p3_deg, f64);
    op!(corner_lat_offset_p4_deg, f64);
    op!(corner_lon_offset_p4_deg, f64);
    op!(corner_lat_p1_deg, f64);
    op!(corner_lon_p1_deg, f64);
    op!(corner_lat_p2_deg, f64);
    op!(corner_lon_p2_deg, f64);
    op!(corner_lat_p3_deg, f64);
    op!(corner_lon_p3_deg, f64);
    op!(corner_lat_p4_deg, f64);
    op!(corner_lon_p4_deg, f64);
    op!(generic_flag_data, u8);
    ob!(security_local_set);
    ob!(vmti);
    ob!(miis_core_id);

    // WP-A Table A1 — ranged f64 fields.
    op!(target_location_lat_deg, f64);
    op!(target_location_lon_deg, f64);
    op!(target_location_elev_m, f64);
    op!(target_track_gate_width_px, f64);
    op!(target_track_gate_height_px, f64);
    op!(target_error_ce90_m, f64);
    op!(target_error_le90_m, f64);
    op!(wind_direction_deg, f64);
    op!(wind_speed, f64);
    op!(static_pressure_mbar, f64);
    op!(density_altitude_m, f64);
    op!(differential_pressure_mbar, f64);
    op!(airfield_barometric_pressure_mbar, f64);
    op!(airfield_elevation_m, f64);
    op!(relative_humidity_pct, f64);
    op!(platform_vertical_speed, f64);
    op!(platform_sideslip_deg, f64);
    op!(platform_ground_speed, f64);
    op!(ground_range_m, f64);
    op!(platform_fuel_remaining_kg, f64);
    op!(platform_magnetic_heading_deg, f64);
    op!(platform_angle_of_attack_full_deg, f64);
    op!(platform_sideslip_full_deg, f64);
    op!(alternate_platform_lat_deg, f64);
    op!(alternate_platform_lon_deg, f64);
    op!(alternate_platform_alt_m, f64);
    op!(alternate_platform_heading_deg, f64);
    op!(alternate_platform_ellipsoid_height_m, f64);
    op!(sensor_north_velocity, f64);
    op!(sensor_east_velocity, f64);

    // WP-A Table A4 — named nested-set raw fields.
    ob!(rvt);
    ob!(sar_mi_local_set);
    ob!(range_image_local_set);
    ob!(geo_registration_local_set);
    ob!(composite_imaging_local_set);
    ob!(segment_local_set);
    ob!(amend_local_set);

    // WP-A Table A2 — raw/simple scalar + string fields.
    op!(outside_air_temp_c, i8);
    op!(weapon_load, u16);
    op!(weapon_fired, u8);
    op!(laser_prf_code, u16);
    os!(alternate_platform_name);
    op!(event_start_time_us, u64);
    os!(stream_designator);
    os!(operational_base);
    os!(broadcast_source);
    os!(target_id);
    os!(communications_method);

    // WP-A Table A3 — coded enums. `enum_field_to_u8` accepts either a
    // Python enum instance or a raw int (wire-unknown pass-through).
    let icing_obj = p.getattr(intern!(p.py(), "icing_detected"))?;
    if !icing_obj.is_none() {
        r.icing_detected = Some(icing_detected_from_wire(enum_field_to_u8(&icing_obj)?));
    }
    let fov_obj = p.getattr(intern!(p.py(), "sensor_fov_name"))?;
    if !fov_obj.is_none() {
        r.sensor_fov_name = Some(sensor_fov_name_from_wire(enum_field_to_u8(&fov_obj)?));
    }
    let opmode_obj = p.getattr(intern!(p.py(), "operational_mode"))?;
    if !opmode_obj.is_none() {
        r.operational_mode = Some(operational_mode_from_wire(enum_field_to_u8(&opmode_obj)?));
    }

    r.unknown = py_to_unknown(p, is_st0601_typed_tag)?;

    // sentinel_tags: tuple[int, ...] → Vec<u32>. Extracting u32 directly
    // raises OverflowError on out-of-range values instead of truncating.
    r.sentinel_tags = p
        .getattr(intern!(p.py(), "sentinel_tags"))?
        .extract::<Vec<u32>>()?;

    Ok(r)
}

// ---------------------------------------------------------------------------
// OutOfRangePolicy — IntEnum-shaped frozen PyClass.
// ---------------------------------------------------------------------------

/// Policy for ranged values that fall outside their ST 0601 mapped range
/// during encoding.
///
/// - ``ERROR`` (default): raise ``KlvEncodeError(OUT_OF_RANGE)`` — the
///   encoder never silently alters the caller's data.
/// - ``INDICATOR``: emit the tag's spec-defined Out-of-Range special value
///   instead of raising. This applies only to the tags whose INT_MIN sentinel
///   (``0x8000`` for 2-byte, ``0x80000000`` for 4-byte signed mappings) means
///   "Out of Range" per ST 0601.19 §7.5 / requirement ST 0601.13-27.  Of the
///   currently encodable ``UasDatalinkLs`` fields, these are: platform pitch /
///   roll / angle-of-attack (Tags 6, 7, 50) and full-range pitch / roll (Tags
///   90, 91). All other fields, and any non-finite value, still raise even
///   under ``INDICATOR``.
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[pyclass(name = "OutOfRangePolicy", module = "tstrans.klv", eq, eq_int, frozen)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PyOutOfRangePolicy {
    ERROR = 0,
    INDICATOR = 1,
}

impl From<PyOutOfRangePolicy> for RustOutOfRangePolicy {
    fn from(p: PyOutOfRangePolicy) -> Self {
        match p {
            PyOutOfRangePolicy::ERROR => RustOutOfRangePolicy::Error,
            PyOutOfRangePolicy::INDICATOR => RustOutOfRangePolicy::Indicator,
        }
    }
}

/// Encode a Python `UasDatalinkLs` to wire bytes (lenient — emits only
/// the populated fields, no mandatory-tag enforcement). Returns
/// `bytes` containing the 16-byte UL + BER length + body.
///
/// The optional keyword-only `out_of_range_policy` (default: `ERROR`) controls
/// how values outside their ST 0601 mapped range are handled — see
/// `OutOfRangePolicy` for details.
#[pyfunction]
#[pyo3(name = "encode_uas_datalink", signature = (record, *, out_of_range_policy = None))]
fn encode_uas_datalink_py(
    py: Python<'_>,
    record: &Bound<'_, PyAny>,
    out_of_range_policy: Option<PyOutOfRangePolicy>,
) -> PyResult<PyObject> {
    let rust_rec = py_to_uas_datalink_ls(record)?;
    let mut opts = St0601EncodeConfig::default();
    if let Some(p) = out_of_range_policy {
        opts.out_of_range_policy = p.into();
    }
    let bytes =
        encode_st0601_with(&rust_rec, &opts).map_err(|e| klv_encode_error_to_pyerr(py, e))?;
    Ok(pyo3::types::PyBytes::new_bound(py, &bytes).unbind().into())
}

/// Encode a Python `UasDatalinkLs` to wire bytes with strict compliance
/// per ST 0601.19 — requires Tag 2 (precision timestamp), Tag 1
/// (checksum slot — synthesized), and Tag 65 (version). Raises
/// `KlvEncodeError(MISSING_MANDATORY_ITEM)` if a required tag is absent.
#[pyfunction]
#[pyo3(name = "encode_uas_datalink_strict_compliance")]
fn encode_uas_datalink_strict_compliance_py(
    py: Python<'_>,
    record: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let rust_rec = py_to_uas_datalink_ls(record)?;
    let bytes =
        encode_st0601_strict_compliance(&rust_rec).map_err(|e| klv_encode_error_to_pyerr(py, e))?;
    Ok(pyo3::types::PyBytes::new_bound(py, &bytes).unbind().into())
}

/// Byte-faithful tag-level patch of a raw ST 0601 local set: edited
/// tags re-encoded, every other TLV copied verbatim, checksum
/// recomputed. See `tstrans.klv.patch_uas_datalink` for the
/// dict-accepting user-facing wrapper.
///
/// No `allow_threads` for the same reason as `decode_uas_datalink`:
/// the per-call Rust work is small; releasing the GIL produces
/// ping-pong under hot batch loops.
#[pyfunction]
#[pyo3(name = "patch_uas_datalink")]
fn patch_uas_datalink_py(
    py: Python<'_>,
    raw: &[u8],
    edits: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    let rust_edits = py_to_uas_datalink_ls(edits)?;
    match patch_st0601(raw, &rust_edits) {
        Ok(bytes) => Ok(pyo3::types::PyBytes::new_bound(py, &bytes).unbind().into()),
        Err(KlvPatchError::Decode(e)) => Err(klv_decode_error_to_pyerr(py, e)),
        Err(KlvPatchError::Encode(e)) => Err(klv_encode_error_to_pyerr(py, e)),
        // KlvPatchError is #[non_exhaustive] in tst-core, so a wildcard
        // arm is required from this crate.
        Err(other) => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
            "unhandled KlvPatchError variant: {other}"
        ))),
    }
}

/// Look up the ST 0601.19 spec-defined meaning of the INT_MIN sentinel
/// wire value for `tag`. Returns `"out_of_range"`, `"reserved"`,
/// `"not_available"`, or `None` if the spec assigns no INT_MIN special
/// value for that tag (this does NOT mean the tag is unsigned or that
/// INT_MIN is a valid wire value for that tag). See
/// `UasDatalinkLs.sentinel_tags` for where this lookup applies.
#[pyfunction]
#[pyo3(name = "st0601_sentinel_meaning")]
fn st0601_sentinel_meaning_py(tag: u32) -> Option<&'static str> {
    let meaning = rust_st0601_sentinel_meaning(tag)?;
    Some(match meaning {
        St0601SentinelMeaning::OutOfRange => "out_of_range",
        St0601SentinelMeaning::Reserved => "reserved",
        St0601SentinelMeaning::NotAvailable => "not_available",
        // #[non_exhaustive] in tst-core; no current variant reaches here.
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// ST 1204.3 — MIIS Core Identifier
// ---------------------------------------------------------------------------

/// Translate a Rust `IdType` to the matching Python `tstrans.klv.IdType`
/// enum member. Returns an error if an unknown variant is encountered.
fn convert_id_type(py: Python<'_>, ty: RustIdType) -> PyResult<PyObject> {
    let klv_mod = py.import_bound("tstrans.klv")?;
    let cls = klv_mod.getattr(intern!(py, "IdType"))?;
    let variant = match ty {
        RustIdType::Physical => "PHYSICAL",
        RustIdType::Virtual => "VIRTUAL",
        RustIdType::Managed => "MANAGED",
        _ => {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "unknown IdType variant crossing the binding",
            ));
        }
    };
    Ok(cls.getattr(variant)?.unbind())
}

/// Translate a Rust `CoreId` to a Python `tstrans.klv.CoreId` dataclass.
fn convert_core_id(py: Python<'_>, id: &RustCoreId) -> PyResult<PyObject> {
    let klv_mod = py.import_bound("tstrans.klv")?;
    let cls = klv_mod.getattr(intern!(py, "CoreId"))?;
    let kwargs = PyDict::new_bound(py);

    kwargs.set_item("version", id.version)?;

    if let Some((ref ty, ref uuid)) = id.sensor {
        let id_type_py = convert_id_type(py, *ty)?;
        let uuid_bytes = pyo3::types::PyBytes::new_bound(py, uuid.as_slice());
        let tuple = pyo3::types::PyTuple::new_bound(py, [id_type_py, uuid_bytes.unbind().into()]);
        kwargs.set_item("sensor", tuple)?;
    }

    if let Some((ref ty, ref uuid)) = id.platform {
        let id_type_py = convert_id_type(py, *ty)?;
        let uuid_bytes = pyo3::types::PyBytes::new_bound(py, uuid.as_slice());
        let tuple = pyo3::types::PyTuple::new_bound(py, [id_type_py, uuid_bytes.unbind().into()]);
        kwargs.set_item("platform", tuple)?;
    }

    if let Some(ref uuid) = id.window {
        kwargs.set_item(
            "window",
            pyo3::types::PyBytes::new_bound(py, uuid.as_slice()),
        )?;
    }
    if let Some(ref uuid) = id.minor {
        kwargs.set_item(
            "minor",
            pyo3::types::PyBytes::new_bound(py, uuid.as_slice()),
        )?;
    }

    Ok(cls.call((), Some(&kwargs))?.unbind())
}

/// Inverse of `convert_id_type`: extract an `IdType` from a Python
/// `tstrans.klv.IdType` enum member.
fn py_to_id_type(p: &Bound<'_, PyAny>) -> PyResult<RustIdType> {
    let name: String = p.getattr("name")?.extract()?;
    match name.as_str() {
        "PHYSICAL" => Ok(RustIdType::Physical),
        "VIRTUAL" => Ok(RustIdType::Virtual),
        "MANAGED" => Ok(RustIdType::Managed),
        other => Err(pyo3::exceptions::PyValueError::new_err(format!(
            "unknown IdType variant: {other}"
        ))),
    }
}

/// Inverse of `convert_core_id`: extract a Rust `CoreId` from a Python
/// `tstrans.klv.CoreId` dataclass.
fn py_to_core_id(p: &Bound<'_, PyAny>) -> PyResult<RustCoreId> {
    let py = p.py();

    let version: u8 = p.getattr(intern!(py, "version"))?.extract()?;

    let sensor_obj = p.getattr(intern!(py, "sensor"))?;
    let sensor = if sensor_obj.is_none() {
        None
    } else {
        let (ty_py, uuid_bytes): (Bound<'_, PyAny>, Vec<u8>) = sensor_obj.extract()?;
        let ty = py_to_id_type(&ty_py)?;
        if uuid_bytes.len() != 16 {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "CoreId.sensor UUID must be 16 bytes, got {}",
                uuid_bytes.len()
            )));
        }
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&uuid_bytes);
        Some((ty, uuid))
    };

    let platform_obj = p.getattr(intern!(py, "platform"))?;
    let platform = if platform_obj.is_none() {
        None
    } else {
        let (ty_py, uuid_bytes): (Bound<'_, PyAny>, Vec<u8>) = platform_obj.extract()?;
        let ty = py_to_id_type(&ty_py)?;
        if uuid_bytes.len() != 16 {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "CoreId.platform UUID must be 16 bytes, got {}",
                uuid_bytes.len()
            )));
        }
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&uuid_bytes);
        Some((ty, uuid))
    };

    let window_obj = p.getattr(intern!(py, "window"))?;
    let window = if window_obj.is_none() {
        None
    } else {
        let bytes: Vec<u8> = window_obj.extract()?;
        if bytes.len() != 16 {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "CoreId.window UUID must be 16 bytes, got {}",
                bytes.len()
            )));
        }
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&bytes);
        Some(uuid)
    };

    let minor_obj = p.getattr(intern!(py, "minor"))?;
    let minor = if minor_obj.is_none() {
        None
    } else {
        let bytes: Vec<u8> = minor_obj.extract()?;
        if bytes.len() != 16 {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "CoreId.minor UUID must be 16 bytes, got {}",
                bytes.len()
            )));
        }
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&bytes);
        Some(uuid)
    };

    Ok(RustCoreId::new(version, sensor, platform, window, minor))
}

/// Decode a MISB ST 1204.3 MIIS Core Identifier from binary wire form.
/// `buf` must be exactly the bytes of one Core Identifier (no framing).
/// Raises `KlvError` on any decode failure.
#[pyfunction]
#[pyo3(name = "decode_core_id")]
fn decode_core_id_py(py: Python<'_>, buf: &[u8]) -> PyResult<PyObject> {
    match decode_st1204(buf) {
        Ok(id) => convert_core_id(py, &id),
        Err(e) => Err(st1204_error_to_pyerr(py, e)),
    }
}

/// Encode a Python `CoreId` to its binary wire form. Returns `bytes`.
#[pyfunction]
#[pyo3(name = "encode_core_id")]
fn encode_core_id_py(py: Python<'_>, core_id: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    let rust_id = py_to_core_id(core_id)?;
    let bytes = encode_st1204(&rust_id);
    Ok(pyo3::types::PyBytes::new_bound(py, &bytes).unbind().into())
}

/// Return the ST 1204.3 §7.4.2 textual representation of a `CoreId`.
#[pyfunction]
#[pyo3(name = "core_id_text")]
fn core_id_text_py(_py: Python<'_>, core_id: &Bound<'_, PyAny>) -> PyResult<String> {
    let rust_id = py_to_core_id(core_id)?;
    Ok(rust_id.to_string())
}

// ---------------------------------------------------------------------------
// ST 0902.8 MISMMS validator
// ---------------------------------------------------------------------------

/// Translate a Rust `MismmsViolation` to a Python `tstrans.klv.MismmsViolation`
/// dataclass instance.
fn convert_mismms_violation(py: Python<'_>, v: &RustMismmsViolation) -> PyResult<PyObject> {
    let klv_mod = py.import_bound("tstrans.klv")?;
    let cls = klv_mod.getattr(intern!(py, "MismmsViolation"))?;
    let kwargs = PyDict::new_bound(py);

    match v {
        RustMismmsViolation::MissingItem { tag, name } => {
            kwargs.set_item("kind", "missing")?;
            kwargs.set_item("tag", *tag as u32)?;
            kwargs.set_item("name", *name)?;
        }
        RustMismmsViolation::MissingSecurityItem { tag, name } => {
            kwargs.set_item("kind", "missing_security")?;
            kwargs.set_item("tag", *tag as u32)?;
            kwargs.set_item("name", *name)?;
        }
        RustMismmsViolation::ZeroLengthItem { tag } => {
            kwargs.set_item("kind", "zero_length")?;
            kwargs.set_item("tag", *tag as u32)?;
        }
        RustMismmsViolation::AlternationConflict { tag_a, tag_b } => {
            kwargs.set_item("kind", "alternation_conflict")?;
            kwargs.set_item("tag", *tag_a as u32)?;
            kwargs.set_item("tag_b", *tag_b as u32)?;
        }
        _ => {
            return Err(PyValueError::new_err(
                "unknown MismmsViolation variant crossing the binding",
            ));
        }
    }

    Ok(cls.call((), Some(&kwargs))?.unbind())
}

/// Validate a Python `UasDatalinkLs` record against the ST 0902.8 Minimum
/// Metadata Set (Table 1). Returns a `list[MismmsViolation]`; an empty list
/// means the record satisfies every MISMMS requirement at the record level.
///
/// Reuses `py_to_uas_datalink_ls` — the same converter used by
/// `encode_uas_datalink` — so the Rust-side check sees an identical record.
#[pyfunction]
#[pyo3(name = "validate_mismms")]
fn validate_mismms_py(py: Python<'_>, record: &Bound<'_, PyAny>) -> PyResult<PyObject> {
    let rust_rec = py_to_uas_datalink_ls(record)?;
    let violations = rust_validate_mismms(&rust_rec);
    let items: Vec<PyObject> = violations
        .iter()
        .map(|v| convert_mismms_violation(py, v))
        .collect::<PyResult<Vec<_>>>()?;
    Ok(pyo3::types::PyList::new_bound(py, items).unbind().into())
}

// ---------------------------------------------------------------------------
// Module registration
// ---------------------------------------------------------------------------

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyOutOfRangePolicy>()?;
    m.add_function(wrap_pyfunction!(decode_precision_timestamp_py, m)?)?;
    m.add_function(wrap_pyfunction!(decode_security_py, m)?)?;
    m.add_function(wrap_pyfunction!(decode_vmti_py, m)?)?;
    m.add_function(wrap_pyfunction!(decode_uas_datalink_py, m)?)?;
    m.add_function(wrap_pyfunction!(encode_uas_datalink_py, m)?)?;
    m.add_function(wrap_pyfunction!(
        encode_uas_datalink_strict_compliance_py,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(patch_uas_datalink_py, m)?)?;
    m.add_function(wrap_pyfunction!(st0601_sentinel_meaning_py, m)?)?;
    m.add_function(wrap_pyfunction!(encode_security_py, m)?)?;
    m.add_function(wrap_pyfunction!(encode_security_strict_compliance_py, m)?)?;
    m.add_function(wrap_pyfunction!(encode_precision_timestamp_py, m)?)?;
    m.add_function(wrap_pyfunction!(encode_vmti_py, m)?)?;
    m.add_function(wrap_pyfunction!(encode_vmti_standalone_py, m)?)?;
    m.add_function(wrap_pyfunction!(encode_vmti_strict_compliance_py, m)?)?;
    m.add_function(wrap_pyfunction!(
        encode_vmti_standalone_strict_compliance_py,
        m
    )?)?;
    m.add_function(wrap_pyfunction!(decode_core_id_py, m)?)?;
    m.add_function(wrap_pyfunction!(encode_core_id_py, m)?)?;
    m.add_function(wrap_pyfunction!(core_id_text_py, m)?)?;
    m.add_function(wrap_pyfunction!(validate_mismms_py, m)?)?;
    Ok(())
}
