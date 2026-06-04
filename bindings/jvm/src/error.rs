//! JNI exception construction. One `throw_<family>` per Rust error family,
//! mirroring tst-py's `make_<family>_error` helpers. Each constructs the
//! `org.tstrans.<Family>Exception` object with its `Kind` enum value and throws
//! it. Call these, then return a Rust default from the JNI fn — the pending
//! Java exception is raised when control returns to the JVM.

use jni::JNIEnv;
use jni::objects::{JObject, JThrowable, JValue};
use tst_core::error::{KlvDecodeError, KlvEncodeError};

/// Construct + throw `org.tstrans.DemuxException(Kind.<kind>, message)`.
/// `kind` MUST be one of the `DemuxException.Kind` enum constant names
/// (SCREAMING_SNAKE_CASE), matching the Rust `DemuxError` variants 1:1.
pub fn throw_demux(env: &mut JNIEnv, kind: &str, message: &str) {
    if env.exception_check().unwrap_or(false) {
        return; // don't clobber an already-pending exception
    }
    if let Err(e) = throw_kinded(
        env,
        "org/tstrans/DemuxException",
        "Lorg/tstrans/DemuxException$Kind;",
        kind,
        message,
    ) {
        // Fallback: a plain RuntimeException so the failure is never silent.
        let _ = env.throw_new(
            "java/lang/RuntimeException",
            format!("DemuxException throw failed ({kind}): {e}"),
        );
    }
}

/// Construct + throw `org.tstrans.MuxException(Kind.<kind>, message)`.
/// `kind` MUST be one of the `MuxException.Kind` enum constant names
/// (SCREAMING_SNAKE_CASE), matching the 5-variant `MuxSenderErrorKind` buckets.
pub fn throw_mux(env: &mut JNIEnv, kind: &str, message: &str) {
    if env.exception_check().unwrap_or(false) {
        return; // don't clobber an already-pending exception
    }
    if let Err(e) = throw_kinded(
        env,
        "org/tstrans/MuxException",
        "Lorg/tstrans/MuxException$Kind;",
        kind,
        message,
    ) {
        // Fallback: a plain RuntimeException so the failure is never silent.
        let _ = env.throw_new(
            "java/lang/RuntimeException",
            format!("MuxException throw failed ({kind}): {e}"),
        );
    }
}

/// Construct + throw `org.tstrans.KlvDecodeException(Kind.<kind>, message)`.
/// `kind` MUST be one of the `KlvDecodeException.Kind` constant names
/// (SCREAMING_SNAKE_CASE). The ratchet greps for `throw_klv_decode(env, "<CONST>", ...)`.
pub fn throw_klv_decode(env: &mut JNIEnv, kind: &str, message: &str) {
    if env.exception_check().unwrap_or(false) {
        return; // don't clobber an already-pending exception
    }
    if let Err(e) = throw_kinded(
        env,
        "org/tstrans/KlvDecodeException",
        "Lorg/tstrans/KlvDecodeException$Kind;",
        kind,
        message,
    ) {
        // Fallback: a plain RuntimeException so the failure is never silent.
        let _ = env.throw_new(
            "java/lang/RuntimeException",
            format!("KlvDecodeException throw failed ({kind}): {e}"),
        );
    }
}

/// Construct + throw `org.tstrans.KlvEncodeException(Kind.<kind>, tag, message)`.
/// `tag` = `None` → uses the `(Kind, String)` ctor; `Some(t)` → uses
/// `(Kind, Long, String)`. The ratchet greps for `throw_klv_encode(env, "<CONST>", ...)`.
pub fn throw_klv_encode(env: &mut JNIEnv, kind: &str, tag: Option<u64>, message: &str) {
    if env.exception_check().unwrap_or(false) {
        return; // don't clobber an already-pending exception
    }
    if let Err(e) = throw_klv_encode_inner(env, kind, tag, message) {
        // Fallback: a plain RuntimeException so the failure is never silent.
        let _ = env.throw_new(
            "java/lang/RuntimeException",
            format!("KlvEncodeException throw failed ({kind}): {e}"),
        );
    }
}

fn throw_klv_encode_inner(
    env: &mut JNIEnv,
    kind: &str,
    tag: Option<u64>,
    message: &str,
) -> jni::errors::Result<()> {
    let kind_sig = "Lorg/tstrans/KlvEncodeException$Kind;";
    let kind_val = env
        .get_static_field("org/tstrans/KlvEncodeException$Kind", kind, kind_sig)?
        .l()?;
    let msg = env.new_string(message)?;
    let exc = match tag {
        Some(t) => {
            let boxed = env.new_object("java/lang/Long", "(J)V", &[JValue::Long(t as i64)])?;
            env.new_object(
                "org/tstrans/KlvEncodeException",
                format!("({kind_sig}Ljava/lang/Long;Ljava/lang/String;)V"),
                &[
                    JValue::Object(&kind_val),
                    JValue::Object(&boxed),
                    JValue::Object(&msg),
                ],
            )?
        }
        None => env.new_object(
            "org/tstrans/KlvEncodeException",
            format!("({kind_sig}Ljava/lang/String;)V"),
            &[JValue::Object(&kind_val), JValue::Object(&msg)],
        )?,
    };
    env.throw(JThrowable::from(exc))
}

/// Map + throw a Rust `KlvDecodeError`. All 7 Kind literals appear inline
/// (satisfies the error-mapping ratchet). Used by the per-set JNI fns (Tasks 1–4).
pub fn map_klv_decode_error(env: &mut JNIEnv, e: &KlvDecodeError) {
    let msg = e.to_string();
    match e {
        KlvDecodeError::Truncated { .. }
        | KlvDecodeError::MalformedLength { .. }
        | KlvDecodeError::LengthOverflow { .. } => throw_klv_decode(env, "TRUNCATED_SET", &msg),
        KlvDecodeError::UnexpectedUniversalLabel { .. } => {
            throw_klv_decode(env, "BAD_UNIVERSAL_LABEL", &msg)
        }
        KlvDecodeError::ChecksumMismatch { .. } => throw_klv_decode(env, "CHECKSUM_MISMATCH", &msg),
        KlvDecodeError::DuplicateTag { .. } => throw_klv_decode(env, "DUPLICATE_TAG", &msg),
        KlvDecodeError::Tag2NotFirst
        | KlvDecodeError::Tag1NotLast
        | KlvDecodeError::MissingTag65
        | KlvDecodeError::St0102MissingRequiredTag { .. }
        | KlvDecodeError::St0903MissingRequiredTag { .. } => {
            throw_klv_decode(env, "MISSING_REQUIRED_TAG", &msg)
        }
        KlvDecodeError::MalformedTag { .. }
        | KlvDecodeError::NonCanonicalLength { .. }
        | KlvDecodeError::NonCanonicalTag { .. }
        | KlvDecodeError::TrailingBytes { .. }
        | KlvDecodeError::BadTimeStampPackLength { .. }
        | KlvDecodeError::ReservedBitsInvalid { .. }
        | KlvDecodeError::St0903InvalidVTargetPack { .. }
        | KlvDecodeError::FieldError(_) => throw_klv_decode(env, "MALFORMED_BYTES", &msg),
        _ => throw_klv_decode(env, "INTERNAL", &msg),
    }
}

/// Map + throw a Rust `KlvEncodeError`. All 8 Kind literals appear inline
/// (satisfies the error-mapping ratchet). Used by the per-set JNI fns (Tasks 1–4).
/// The forward-compat wildcard arm aliases to `BUFFER_TOO_SMALL` (matching
/// tst-py's `klv_encode_error_to_pyerr`), not `INTERNAL`.
pub fn map_klv_encode_error(env: &mut JNIEnv, e: &KlvEncodeError) {
    let msg = e.to_string();
    match e {
        KlvEncodeError::BufferTooSmall { .. } => {
            throw_klv_encode(env, "BUFFER_TOO_SMALL", None, &msg)
        }
        KlvEncodeError::RecordTooLarge => throw_klv_encode(env, "RECORD_TOO_LARGE", None, &msg),
        KlvEncodeError::OutOfRange { tag, .. } => {
            throw_klv_encode(env, "OUT_OF_RANGE", Some(u64::from(*tag)), &msg)
        }
        KlvEncodeError::StringTooLong { tag, .. } => {
            throw_klv_encode(env, "STRING_TOO_LONG", Some(u64::from(*tag)), &msg)
        }
        KlvEncodeError::UnsupportedImapbLength { .. } => {
            throw_klv_encode(env, "UNSUPPORTED_IMAPB_LENGTH", None, &msg)
        }
        KlvEncodeError::InvalidImapbParams { .. } => {
            throw_klv_encode(env, "INVALID_IMAPB_PARAMS", None, &msg)
        }
        KlvEncodeError::MissingMandatoryItem { tag, .. } => {
            throw_klv_encode(env, "MISSING_MANDATORY_ITEM", Some(u64::from(*tag)), &msg)
        }
        KlvEncodeError::ReservedTagInUnknown { tag } => {
            throw_klv_encode(env, "RESERVED_TAG_IN_UNKNOWN", Some(u64::from(*tag)), &msg)
        }
        _ => throw_klv_encode(env, "BUFFER_TOO_SMALL", None, &msg),
    }
}

/// Shared builder: looks up `Kind.<kind>` static field, calls the
/// `(<kind_sig>, String)` constructor, throws the result.
fn throw_kinded(
    env: &mut JNIEnv,
    exc_class: &str,
    kind_sig: &str,
    kind: &str,
    message: &str,
) -> jni::errors::Result<()> {
    let kind_class = format!("{exc_class}$Kind");
    let kind_val = env.get_static_field(&kind_class, kind, kind_sig)?.l()?;
    let msg = env.new_string(message)?;
    let ctor_sig = format!("({kind_sig}Ljava/lang/String;)V");
    let exc: JObject = env.new_object(
        exc_class,
        &ctor_sig,
        &[JValue::Object(&kind_val), JValue::Object(&msg)],
    )?;
    env.throw(jni::objects::JThrowable::from(exc))
}
