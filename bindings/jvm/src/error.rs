//! JNI exception construction. One `throw_<family>` per Rust error family,
//! mirroring tst-py's `make_<family>_error` helpers. Each constructs the
//! `org.tstrans.<Family>Exception` object with its `Kind` enum value and throws
//! it. Call these, then return a Rust default from the JNI fn — the pending
//! Java exception is raised when control returns to the JVM.

use jni::JNIEnv;
use jni::objects::{JObject, JValue};

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
