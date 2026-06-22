//! Shared JNI marshalling helpers used across the klv (and potentially other)
//! binding modules. Extracted from `mpegts/mod.rs` where `wrap_heap_byte_buffer`
//! originated; the klv module re-uses it for KLV byte payloads.

use jni::JNIEnv;
use jni::objects::{JByteArray, JObject, JValue};
use tst_core::error::KlvFieldError as RustKlvFieldError;
use tst_core::klv::OwnedRawField;

/// Copy `bytes` into a fresh Java `byte[]` and wrap it as a heap `ByteBuffer`
/// (`java.nio.ByteBuffer.wrap`). The returned buffer is backed by JVM-owned
/// memory, safe to retain past the next call / after `close()`. Used by klv
/// Tasks 1–4.
pub fn wrap_heap_byte_buffer<'local>(
    env: &mut JNIEnv<'local>,
    bytes: &[u8],
) -> Result<JObject<'local>, ()> {
    let arr = env.byte_array_from_slice(bytes).map_err(|_| ())?;
    env.call_static_method(
        "java/nio/ByteBuffer",
        "wrap",
        "([B)Ljava/nio/ByteBuffer;",
        &[JValue::Object(&arr)],
    )
    .map_err(|_| ())?
    .l()
    .map_err(|_| ())
}

/// Map a `RustKlvFieldError` variant to a `(KlvFieldErrorKind name, tag)` pair.
/// Mirrors tst-py's `convert_field_error` arm-for-arm; wildcard → `INVALID_LENGTH`.
fn field_error_kind_and_tag(fe: &RustKlvFieldError) -> (&'static str, u32) {
    match fe {
        RustKlvFieldError::OutOfRange { tag, .. } => ("OUT_OF_RANGE", *tag),
        RustKlvFieldError::InvalidUtf8 { tag } => ("INVALID_UTF8", *tag),
        RustKlvFieldError::InvalidLength { tag, .. } => ("INVALID_LENGTH", *tag),
        RustKlvFieldError::InvalidSentinel { tag } => ("INVALID_SENTINEL", *tag),
        RustKlvFieldError::InvalidUtf16 { tag } => ("INVALID_UTF16", *tag),
        RustKlvFieldError::InvalidCodepoint { tag, .. } => ("INVALID_CODEPOINT", *tag),
        RustKlvFieldError::TruncatedField { tag } => ("TRUNCATED_FIELD", *tag),
        RustKlvFieldError::UnsupportedImapbLength { .. } => ("UNSUPPORTED_IMAPB_LENGTH", 0),
        RustKlvFieldError::InvalidImapbParams { .. } => ("INVALID_IMAPB_PARAMS", 0),
        _ => ("INVALID_LENGTH", 0),
    }
}

/// Build a `java.util.List<KlvFieldError>` (an `ArrayList`) from a slice of
/// `RustKlvFieldError`s. Used by klv Tasks 2–4.
///
/// An ST 0601 record's `field_errors` can exceed the default 16-slot JNI
/// local-ref table, so each iteration's element refs (the kind constant, the
/// message string, the record) are minted inside a per-entry
/// `with_local_frame` and reclaimed at the end of the iteration — only the
/// long-lived `list` ref (created in the outer frame) survives. Mirrors the
/// forward-note on `build_pid_list` in `mpegts/mod.rs`.
pub fn build_field_errors<'local>(
    env: &mut JNIEnv<'local>,
    errs: &[RustKlvFieldError],
) -> jni::errors::Result<JObject<'local>> {
    let list = env.new_object("java/util/ArrayList", "()V", &[])?;
    for fe in errs {
        // 8 slots: comfortably covers the 3 element refs + JNI scratch per entry.
        env.with_local_frame(8, |env| -> jni::errors::Result<()> {
            let (kind_name, tag) = field_error_kind_and_tag(fe);
            let kind_obj = env
                .get_static_field(
                    "org/tstrans/klv/KlvFieldErrorKind",
                    kind_name,
                    "Lorg/tstrans/klv/KlvFieldErrorKind;",
                )?
                .l()?;
            let msg = env.new_string(fe.to_string())?;
            let obj = env.new_object(
                "org/tstrans/klv/KlvFieldError",
                "(Lorg/tstrans/klv/KlvFieldErrorKind;JLjava/lang/String;)V",
                &[
                    JValue::Object(&kind_obj),
                    JValue::Long(i64::from(tag)),
                    JValue::Object(&msg),
                ],
            )?;
            env.call_method(
                &list,
                "add",
                "(Ljava/lang/Object;)Z",
                &[JValue::Object(&obj)],
            )?;
            Ok(())
        })?;
    }
    Ok(list)
}

/// Build a `java.util.List<KlvUnknownField>` (an `ArrayList`) from a slice of
/// [`OwnedRawField`]s. Each entry is a heap `ByteBuffer` copy. Used by klv
/// Tasks 2–4 (call sites pass `&record.unknown`).
///
/// As with `build_field_errors`, an ST 0601 record's `unknown` list can exceed
/// the default 16-slot JNI local-ref table, so each iteration's element refs
/// (the byte array + ByteBuffer + record) are minted inside a per-entry
/// `with_local_frame` and reclaimed at the end of the iteration — only the
/// long-lived `list` ref survives.
pub fn build_unknown_list<'local>(
    env: &mut JNIEnv<'local>,
    fields: &[OwnedRawField],
) -> jni::errors::Result<JObject<'local>> {
    let list = env.new_object("java/util/ArrayList", "()V", &[])?;
    for f in fields {
        // 8 slots: covers the byte[] + ByteBuffer + record refs + JNI scratch.
        env.with_local_frame(8, |env| -> jni::errors::Result<()> {
            let arr = env.byte_array_from_slice(&f.value)?;
            let buf = env
                .call_static_method(
                    "java/nio/ByteBuffer",
                    "wrap",
                    "([B)Ljava/nio/ByteBuffer;",
                    &[JValue::Object(&arr)],
                )?
                .l()?;
            let obj = env.new_object(
                "org/tstrans/klv/KlvUnknownField",
                "(JLjava/nio/ByteBuffer;)V",
                &[JValue::Long(i64::from(f.tag)), JValue::Object(&buf)],
            )?;
            env.call_method(
                &list,
                "add",
                "(Ljava/lang/Object;)Z",
                &[JValue::Object(&obj)],
            )?;
            Ok(())
        })?;
    }
    Ok(list)
}

/// Read a `java.util.List<KlvUnknownField>` back into a `Vec<OwnedRawField>`,
/// dropping any entry whose tag collides with a typed tag (typed wins). Used by klv
/// Tasks 2–4 (call sites assign the result back to `record.unknown`).
pub fn read_unknown_list(
    env: &mut JNIEnv,
    list: &JObject,
    is_typed: impl Fn(u32) -> bool,
) -> jni::errors::Result<Vec<OwnedRawField>> {
    let size = env.call_method(list, "size", "()I", &[])?.i()?;
    let mut out = Vec::new();
    for i in 0..size {
        let item = env
            .call_method(list, "get", "(I)Ljava/lang/Object;", &[JValue::Int(i)])?
            .l()?;
        let tag_long = env.call_method(&item, "tag", "()J", &[])?.j()?;
        // KLV BER-OID tags are u32; a Java `long` tag outside 0..=u32::MAX is
        // out-of-range for a real field. Fail-closed (skip) rather than
        // truncating it into a different tag value.
        let Ok(tag) = u32::try_from(tag_long) else {
            continue;
        };
        if is_typed(tag) {
            continue; // typed field wins; drop this unknown entry
        }
        let buf_obj = env
            .call_method(&item, "value", "()Ljava/nio/ByteBuffer;", &[])?
            .l()?;
        // Use read_byte_buffer to honour position/limit and support direct buffers.
        let value = read_byte_buffer(env, &buf_obj)?;
        out.push(OwnedRawField { tag, value });
    }
    Ok(out)
}

// -----------------------------------------------------------------------
// ByteBuffer helper
// -----------------------------------------------------------------------

/// Read a `ByteBuffer`'s REMAINING bytes (honouring position/limit), without
/// mutating the caller's buffer (operates on a duplicate). Works for heap AND
/// direct buffers.
pub fn read_byte_buffer(env: &mut JNIEnv, buf: &JObject) -> jni::errors::Result<Vec<u8>> {
    // duplicate() gives us an independent position/limit view on the same data,
    // so we can call get([B) without advancing the caller's position.
    let dup = env
        .call_method(buf, "duplicate", "()Ljava/nio/ByteBuffer;", &[])?
        .l()?;
    let remaining = env.call_method(&dup, "remaining", "()I", &[])?.i()?;
    let arr = env.new_byte_array(remaining)?;
    env.call_method(
        &dup,
        "get",
        "([B)Ljava/nio/ByteBuffer;",
        &[JValue::Object(&arr)],
    )?;
    env.convert_byte_array(JByteArray::from(arr))
}

// -----------------------------------------------------------------------
// Checked narrowing helpers (encode path)
// -----------------------------------------------------------------------

/// Range-check a Java `int`/`long` value against the u8 range, then narrow.
/// Throws `IllegalArgumentException` and returns `Err(JavaException)` on overflow
/// (matches tst-py's `extract::<u8>()` fail-loud semantics).
pub fn checked_u8(env: &mut JNIEnv, value: i64, field: &str) -> jni::errors::Result<u8> {
    if !(0..=255).contains(&value) {
        let _ = env.throw_new(
            "java/lang/IllegalArgumentException",
            format!("{field} must be 0..=255, got {value}"),
        );
        return Err(jni::errors::Error::JavaException);
    }
    Ok(value as u8)
}

/// Range-check a Java `int`/`long` value against the u16 range, then narrow.
/// Throws `IllegalArgumentException` and returns `Err(JavaException)` on overflow.
pub fn checked_u16(env: &mut JNIEnv, value: i64, field: &str) -> jni::errors::Result<u16> {
    if !(0..=65535).contains(&value) {
        let _ = env.throw_new(
            "java/lang/IllegalArgumentException",
            format!("{field} must be 0..=65535, got {value}"),
        );
        return Err(jni::errors::Error::JavaException);
    }
    Ok(value as u16)
}

/// Range-check a Java `long` value against the u32 range, then narrow.
/// Throws `IllegalArgumentException` and returns `Err(JavaException)` on overflow.
pub fn checked_u32(env: &mut JNIEnv, value: i64, field: &str) -> jni::errors::Result<u32> {
    if !(0..=u32::MAX as i64).contains(&value) {
        let _ = env.throw_new(
            "java/lang/IllegalArgumentException",
            format!("{field} must be 0..=4294967295, got {value}"),
        );
        return Err(jni::errors::Error::JavaException);
    }
    Ok(value as u32)
}

/// Convert a Java `long` (carrying an unsigned ST 0903 u64 value) to `u64`,
/// rejecting only a negative `i64` (which is a caller bug — Java has no
/// unsigned primitive, so large values arrive as a bit-pattern `long`).
pub fn checked_u64(env: &mut JNIEnv, value: i64, field: &str) -> jni::errors::Result<u64> {
    if value < 0 {
        let _ = env.throw_new(
            "java/lang/IllegalArgumentException",
            format!("{field} must be >= 0 (unsigned long), got {value}"),
        );
        return Err(jni::errors::Error::JavaException);
    }
    Ok(value as u64)
}

// -----------------------------------------------------------------------
// Shared nullable accessor helpers (encode path)
// -----------------------------------------------------------------------

/// Read a nullable `Integer` accessor. Returns `None` for null.
pub fn read_nullable_int(
    env: &mut JNIEnv,
    rec: &JObject,
    name: &str,
) -> jni::errors::Result<Option<i32>> {
    let obj = env
        .call_method(rec, name, "()Ljava/lang/Integer;", &[])?
        .l()?;
    if obj.is_null() {
        return Ok(None);
    }
    let v = env.call_method(&obj, "intValue", "()I", &[])?.i()?;
    Ok(Some(v))
}

/// Read a nullable `Long` accessor. Returns `None` for null.
pub fn read_nullable_long(
    env: &mut JNIEnv,
    rec: &JObject,
    name: &str,
) -> jni::errors::Result<Option<i64>> {
    let obj = env.call_method(rec, name, "()Ljava/lang/Long;", &[])?.l()?;
    if obj.is_null() {
        return Ok(None);
    }
    let v = env.call_method(&obj, "longValue", "()J", &[])?.j()?;
    Ok(Some(v))
}

/// Read a nullable `Double` accessor. Returns `None` for null.
pub fn read_nullable_double(
    env: &mut JNIEnv,
    rec: &JObject,
    name: &str,
) -> jni::errors::Result<Option<f64>> {
    let obj = env
        .call_method(rec, name, "()Ljava/lang/Double;", &[])?
        .l()?;
    if obj.is_null() {
        return Ok(None);
    }
    let v = env.call_method(&obj, "doubleValue", "()D", &[])?.d()?;
    Ok(Some(v))
}

/// Read a nullable `String` accessor. Returns `None` for null.
pub fn read_nullable_string(
    env: &mut JNIEnv,
    rec: &JObject,
    name: &str,
) -> jni::errors::Result<Option<String>> {
    let obj = env
        .call_method(rec, name, "()Ljava/lang/String;", &[])?
        .l()?;
    if obj.is_null() {
        return Ok(None);
    }
    let j_str: &jni::objects::JString = (&obj).into();
    let s: String = env.get_string(j_str).map(Into::into)?;
    Ok(Some(s))
}

/// Read a nullable `ByteBuffer` accessor using `read_byte_buffer` (honours
/// position/limit; works for heap and direct buffers). Returns `None` for null.
pub fn read_nullable_byte_buffer(
    env: &mut JNIEnv,
    rec: &JObject,
    name: &str,
) -> jni::errors::Result<Option<Vec<u8>>> {
    let obj = env
        .call_method(rec, name, "()Ljava/nio/ByteBuffer;", &[])?
        .l()?;
    if obj.is_null() {
        return Ok(None);
    }
    let bytes = read_byte_buffer(env, &obj)?;
    Ok(Some(bytes))
}
