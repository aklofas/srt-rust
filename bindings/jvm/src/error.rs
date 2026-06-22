//! JNI exception construction. One `throw_<family>` per Rust error family,
//! mirroring tst-py's `make_<family>_error` helpers. Each constructs the
//! `org.tstrans.<Family>Exception` object with its `Kind` enum value and throws
//! it. Call these, then return a Rust default from the JNI fn — the pending
//! Java exception is raised when control returns to the JVM.

use jni::JNIEnv;
use jni::objects::{JObject, JThrowable, JValue};
use tst_core::codec::CodecParseError;
use tst_core::error::{KlvDecodeError, KlvEncodeError};

/// Variant-specific diagnostic fields forwarded to `CodecParseException`.
/// Every field is `None` except those the producing `CodecParseError` variant
/// carries — mirrors the per-variant kwarg set in tst-py's
/// `codec_parse_error_to_pyerr`.
#[derive(Default)]
pub struct CodecErrFields {
    pub offset_bits: Option<i32>,
    pub needed_bits: Option<i32>,
    pub field: Option<String>,
    pub value: Option<i32>,
    pub profile_idc: Option<i32>,
    pub sps_id: Option<i32>,
    pub vps_id: Option<i32>,
    pub offset_bytes: Option<i32>,
    pub expected: Option<i32>,
    pub found: Option<i32>,
    pub needed: Option<i32>,
    pub had: Option<i32>,
    pub layer: Option<i32>,
}

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

/// Map + throw a Rust `KlvEncodeError`. All 11 Kind literals appear inline
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
        KlvEncodeError::VTargetPackEmpty { target_id } => {
            throw_klv_encode(env, "VTARGET_PACK_EMPTY", Some(*target_id), &msg)
        }
        KlvEncodeError::DuplicateTargetId { target_id } => {
            // Hoist the boxed tag so the `throw_klv_encode(env, "<CONST>", ...)`
            // call stays on one line — required by both rustfmt's width and the
            // error-mapping ratchet's per-constant grep (a brace-less arm with
            // this longer CONST would otherwise split the call across lines and
            // hide the constant from the grep).
            let t = Some(*target_id);
            throw_klv_encode(env, "DUPLICATE_TARGET_ID", t, &msg)
        }
        KlvEncodeError::ForbiddenStandaloneOffset { tag } => {
            let t = Some(u64::from(*tag));
            throw_klv_encode(env, "FORBIDDEN_STANDALONE_OFFSET", t, &msg)
        }
        _ => throw_klv_encode(env, "BUFFER_TOO_SMALL", None, &msg),
    }
}

/// Construct + throw `org.tstrans.CodecParseException`.
/// `kind` MUST be one of the `CodecParseException.Kind` constant names
/// (SCREAMING_SNAKE_CASE). The ratchet greps for `throw_codec(env, "<CONST>", ...)`
/// — note `kind` is the 2nd argument (after `env`), so the literal sits where
/// the ratchet expects it. `message` (LAST arg) is the exception's
/// `getMessage()` text — call sites pass the Rust `Display` string.
pub fn throw_codec(
    env: &mut JNIEnv,
    kind: &str,
    codec: &str,
    fields: &CodecErrFields,
    message: &str,
) {
    if env.exception_check().unwrap_or(false) {
        return; // don't clobber an already-pending exception
    }
    if let Err(e) = throw_codec_inner(env, kind, codec, fields, message) {
        // Fallback: a plain RuntimeException so the failure is never silent.
        let _ = env.throw_new(
            "java/lang/RuntimeException",
            format!("CodecParseException throw failed ({kind}): {e}"),
        );
    }
}

/// Box an `Option<i32>` into a `java.lang.Integer` (or null) JNI argument.
fn boxed_int<'local>(
    env: &mut JNIEnv<'local>,
    v: Option<i32>,
) -> jni::errors::Result<JObject<'local>> {
    match v {
        Some(n) => env.new_object("java/lang/Integer", "(I)V", &[JValue::Int(n)]),
        None => Ok(JObject::null()),
    }
}

fn throw_codec_inner(
    env: &mut JNIEnv,
    kind: &str,
    codec: &str,
    fields: &CodecErrFields,
    message: &str,
) -> jni::errors::Result<()> {
    let exc = build_codec_object(env, kind, codec, fields, message)?;
    env.throw(JThrowable::from(exc))
}

/// Construct (but do NOT throw) an `org.tstrans.CodecParseException` object from
/// a kind literal + diagnostic fields. The object-construction half of
/// [`throw_codec_inner`], split out so the demux audio bytes-fallback path can
/// attach a `CodecParseException` to a `DemuxEvent.Audio` record without raising
/// it. Mirrors tst-py, where `codec_parse_error_to_pyerr` builds a `PyErr` value
/// the audio arm stores on the event rather than throwing.
fn build_codec_object<'local>(
    env: &mut JNIEnv<'local>,
    kind: &str,
    codec: &str,
    fields: &CodecErrFields,
    message: &str,
) -> jni::errors::Result<JObject<'local>> {
    // 17 local refs are live at the `new_object` call (kind_val + codec_str +
    // field_str + 12 boxed ints + msg + exc), over the JNI-guaranteed 16-slot
    // floor — reserve headroom up front, matching the KLV-builder house pattern.
    env.ensure_local_capacity(20)?;
    let kind_sig = "Lorg/tstrans/CodecParseException$Kind;";
    let kind_val = env
        .get_static_field("org/tstrans/CodecParseException$Kind", kind, kind_sig)?
        .l()?;
    let codec_str = env.new_string(codec)?;
    let field_str = match &fields.field {
        Some(f) => env.new_string(f)?.into(),
        None => JObject::null(),
    };
    let offset_bits = boxed_int(env, fields.offset_bits)?;
    let needed_bits = boxed_int(env, fields.needed_bits)?;
    let value = boxed_int(env, fields.value)?;
    let profile_idc = boxed_int(env, fields.profile_idc)?;
    let sps_id = boxed_int(env, fields.sps_id)?;
    let vps_id = boxed_int(env, fields.vps_id)?;
    let offset_bytes = boxed_int(env, fields.offset_bytes)?;
    let expected = boxed_int(env, fields.expected)?;
    let found = boxed_int(env, fields.found)?;
    let needed = boxed_int(env, fields.needed)?;
    let had = boxed_int(env, fields.had)?;
    let layer = boxed_int(env, fields.layer)?;

    // Canonical 16-arg ctor: (Kind, String codec, String message, then the
    // 13 nullable diagnostic fields in declaration order). The message slot
    // carries the Rust `Display` string forwarded by the call site (mirrors
    // tst-py's `format!("{err}")` and `throw_klv_decode`/`throw_demux`).
    let msg = env.new_string(message)?;
    let ctor_sig = "(Lorg/tstrans/CodecParseException$Kind;\
Ljava/lang/String;Ljava/lang/String;\
Ljava/lang/Integer;Ljava/lang/Integer;Ljava/lang/String;\
Ljava/lang/Integer;Ljava/lang/Integer;Ljava/lang/Integer;\
Ljava/lang/Integer;Ljava/lang/Integer;Ljava/lang/Integer;\
Ljava/lang/Integer;Ljava/lang/Integer;Ljava/lang/Integer;\
Ljava/lang/Integer;)V";
    env.new_object(
        "org/tstrans/CodecParseException",
        ctor_sig,
        &[
            JValue::Object(&kind_val),
            JValue::Object(&codec_str),
            JValue::Object(&msg),
            JValue::Object(&offset_bits),
            JValue::Object(&needed_bits),
            JValue::Object(&field_str),
            JValue::Object(&value),
            JValue::Object(&profile_idc),
            JValue::Object(&sps_id),
            JValue::Object(&vps_id),
            JValue::Object(&offset_bytes),
            JValue::Object(&expected),
            JValue::Object(&found),
            JValue::Object(&needed),
            JValue::Object(&had),
            JValue::Object(&layer),
        ],
    )
}

/// Map + throw a Rust `CodecParseError`. All 12 Kind literals appear inline as
/// the 2nd argument to `throw_codec` (satisfies the error-mapping ratchet).
/// `codec` is a short lowercase codec name (e.g. `"h264"`). Mirrors tst-py's
/// `codec_parse_error_to_pyerr` variant-for-variant; the wildcard arm routes
/// any future marked-non-exhaustive variant to `ENGINE_ERROR`.
pub fn map_codec_parse_error(env: &mut JNIEnv, e: &CodecParseError, codec: &str) {
    // The exception message is the Rust `Display` string (mirrors tst-py's
    // `format!("{err}")`); forwarded to every `throw_codec` call below.
    let msg = e.to_string();
    // NOTE: each arm binds the per-variant fields to `f` first, then makes the
    // `throw_codec(env, "<KIND>", codec, &f, &msg)` call on ONE line so the
    // error-mapping ratchet (a line-oriented grep for
    // `throw_codec\s*\(\s*[^,]*,\s*"<KIND>"`) sees `env, "<KIND>"` together.
    match e {
        CodecParseError::TruncatedRbsp {
            offset_bits,
            needed_bits,
        } => {
            let f = CodecErrFields {
                offset_bits: Some(*offset_bits as i32),
                needed_bits: Some(*needed_bits as i32),
                ..Default::default()
            };
            throw_codec(env, "TRUNCATED_RBSP", codec, &f, &msg)
        }
        CodecParseError::InvalidGolomb { offset_bits } => {
            let f = CodecErrFields {
                offset_bits: Some(*offset_bits as i32),
                ..Default::default()
            };
            throw_codec(env, "INVALID_GOLOMB", codec, &f, &msg)
        }
        CodecParseError::ReservedValue { field, value } => {
            let f = CodecErrFields {
                field: Some((*field).to_string()),
                value: Some(*value as i32),
                ..Default::default()
            };
            throw_codec(env, "RESERVED_VALUE", codec, &f, &msg)
        }
        CodecParseError::UnsupportedProfile { profile_idc } => {
            let f = CodecErrFields {
                profile_idc: Some(i32::from(*profile_idc)),
                ..Default::default()
            };
            throw_codec(env, "UNSUPPORTED_PROFILE", codec, &f, &msg)
        }
        CodecParseError::DanglingSpsReference { sps_id } => {
            let f = CodecErrFields {
                sps_id: Some(i32::from(*sps_id)),
                ..Default::default()
            };
            throw_codec(env, "DANGLING_SPS_REFERENCE", codec, &f, &msg)
        }
        CodecParseError::DanglingVpsReference { vps_id } => {
            let f = CodecErrFields {
                vps_id: Some(i32::from(*vps_id)),
                ..Default::default()
            };
            throw_codec(env, "DANGLING_VPS_REFERENCE", codec, &f, &msg)
        }
        CodecParseError::EngineError(_) => {
            throw_codec(env, "ENGINE_ERROR", codec, &CodecErrFields::default(), &msg)
        }
        CodecParseError::InvalidLeb128 { offset_bytes } => {
            let f = CodecErrFields {
                offset_bytes: Some(*offset_bytes as i32),
                ..Default::default()
            };
            throw_codec(env, "INVALID_LEB128", codec, &f, &msg)
        }
        CodecParseError::BadSyncWord { expected, found } => {
            let f = CodecErrFields {
                expected: Some(i32::from(*expected)),
                found: Some(i32::from(*found)),
                ..Default::default()
            };
            throw_codec(env, "BAD_SYNC_WORD", codec, &f, &msg)
        }
        CodecParseError::Truncated { needed, had } => {
            let f = CodecErrFields {
                needed: Some(*needed as i32),
                had: Some(*had as i32),
                ..Default::default()
            };
            throw_codec(env, "TRUNCATED", codec, &f, &msg)
        }
        CodecParseError::Forbidden { field } => {
            let f = CodecErrFields {
                field: Some((*field).to_string()),
                ..Default::default()
            };
            throw_codec(env, "FORBIDDEN", codec, &f, &msg)
        }
        CodecParseError::UnsupportedFreeFormat { layer } => {
            let f = CodecErrFields {
                layer: Some(i32::from(*layer)),
                ..Default::default()
            };
            throw_codec(env, "UNSUPPORTED_FREE_FORMAT", codec, &f, &msg)
        }
        // Catch-all for marked-non-exhaustive additions not yet mapped:
        _ => throw_codec(env, "ENGINE_ERROR", codec, &CodecErrFields::default(), &msg),
    }
}

/// Build (but do NOT throw) an `org.tstrans.CodecParseException` from a Rust
/// `CodecParseError`. The non-throwing twin of [`map_codec_parse_error`], used
/// by the demux audio bytes-fallback path (`DemuxEvent.Audio.codecParseError`):
/// the audio arm needs the exception object to store on the record, not raised.
/// Mirrors tst-py's audio arm storing `codec_parse_error_to_pyerr(...)` on the
/// event. `codec` is the short demux label (`"aac"` / `"mp2"`).
///
/// The per-variant `(kind, fields)` derivation duplicates
/// [`map_codec_parse_error`]'s match by design: that function's inline
/// `throw_codec(env, "<KIND>", ..)` literals are the error-mapping ratchet's
/// coverage contract, so they must stay as direct call sites and cannot be
/// factored into a shared `(kind, fields)` helper.
pub fn build_codec_exception<'local>(
    env: &mut JNIEnv<'local>,
    e: &CodecParseError,
    codec: &str,
) -> Result<JObject<'local>, ()> {
    let msg = e.to_string();
    let (kind, fields): (&str, CodecErrFields) = match e {
        CodecParseError::TruncatedRbsp {
            offset_bits,
            needed_bits,
        } => (
            "TRUNCATED_RBSP",
            CodecErrFields {
                offset_bits: Some(*offset_bits as i32),
                needed_bits: Some(*needed_bits as i32),
                ..Default::default()
            },
        ),
        CodecParseError::InvalidGolomb { offset_bits } => (
            "INVALID_GOLOMB",
            CodecErrFields {
                offset_bits: Some(*offset_bits as i32),
                ..Default::default()
            },
        ),
        CodecParseError::ReservedValue { field, value } => (
            "RESERVED_VALUE",
            CodecErrFields {
                field: Some((*field).to_string()),
                value: Some(*value as i32),
                ..Default::default()
            },
        ),
        CodecParseError::UnsupportedProfile { profile_idc } => (
            "UNSUPPORTED_PROFILE",
            CodecErrFields {
                profile_idc: Some(i32::from(*profile_idc)),
                ..Default::default()
            },
        ),
        CodecParseError::DanglingSpsReference { sps_id } => (
            "DANGLING_SPS_REFERENCE",
            CodecErrFields {
                sps_id: Some(i32::from(*sps_id)),
                ..Default::default()
            },
        ),
        CodecParseError::DanglingVpsReference { vps_id } => (
            "DANGLING_VPS_REFERENCE",
            CodecErrFields {
                vps_id: Some(i32::from(*vps_id)),
                ..Default::default()
            },
        ),
        CodecParseError::EngineError(_) => ("ENGINE_ERROR", CodecErrFields::default()),
        CodecParseError::InvalidLeb128 { offset_bytes } => (
            "INVALID_LEB128",
            CodecErrFields {
                offset_bytes: Some(*offset_bytes as i32),
                ..Default::default()
            },
        ),
        CodecParseError::BadSyncWord { expected, found } => (
            "BAD_SYNC_WORD",
            CodecErrFields {
                expected: Some(i32::from(*expected)),
                found: Some(i32::from(*found)),
                ..Default::default()
            },
        ),
        CodecParseError::Truncated { needed, had } => (
            "TRUNCATED",
            CodecErrFields {
                needed: Some(*needed as i32),
                had: Some(*had as i32),
                ..Default::default()
            },
        ),
        CodecParseError::Forbidden { field } => (
            "FORBIDDEN",
            CodecErrFields {
                field: Some((*field).to_string()),
                ..Default::default()
            },
        ),
        CodecParseError::UnsupportedFreeFormat { layer } => (
            "UNSUPPORTED_FREE_FORMAT",
            CodecErrFields {
                layer: Some(i32::from(*layer)),
                ..Default::default()
            },
        ),
        _ => ("ENGINE_ERROR", CodecErrFields::default()),
    };
    build_codec_object(env, kind, codec, &fields, &msg).map_err(|_| ())
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
