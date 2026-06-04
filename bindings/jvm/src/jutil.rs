//! Shared JNI marshalling helpers used across the klv (and potentially other)
//! binding modules. Extracted from `mpegts/mod.rs` where `wrap_heap_byte_buffer`
//! originated; the klv module re-uses it for KLV byte payloads.

use jni::JNIEnv;
use jni::objects::{JObject, JValue};
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
        // Read the ByteBuffer's remaining bytes via ByteBuffer.array() (heap-backed copy).
        let arr = env.call_method(&buf_obj, "array", "()[B", &[])?.l()?;
        let arr = jni::objects::JByteArray::from(arr);
        let value = env.convert_byte_array(&arr)?;
        out.push(OwnedRawField { tag, value });
    }
    Ok(out)
}
