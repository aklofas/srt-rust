//! `org.tstrans.rtp` Rust→Java error mapping. Maps every `tst_core` /
//! `tst_rtp` transport error that flows through the RTP surface onto one of the
//! three `RtpException.Kind` variants. Ported 1:1 from tst-py's
//! `bindings/python/src/rtp/transport.rs` (`transport_error_to_pyerr` +
//! `connect_error_to_pyerr`). The ratchet greps for
//! `throw_rtp(env, "<CONST>", ...)` — keep each KIND literal on the call line.

use jni::JNIEnv;
use jni::objects::{JObject, JThrowable, JValue};
use tst_core::transport::TransportError;
use tst_rtp::ConnectError;

/// Construct + throw `org.tstrans.RtpException(Kind.<kind>, message)`.
/// `kind` MUST be one of the `RtpException.Kind` constant names.
pub(crate) fn throw_rtp(env: &mut JNIEnv, kind: &str, message: &str) {
    if env.exception_check().unwrap_or(false) {
        return; // don't clobber an already-pending exception
    }
    if let Err(e) = throw_rtp_inner(env, kind, message) {
        let _ = env.throw_new(
            "java/lang/RuntimeException",
            format!("RtpException throw failed ({kind}): {e}"),
        );
    }
}

fn throw_rtp_inner(env: &mut JNIEnv, kind: &str, message: &str) -> jni::errors::Result<()> {
    let kind_sig = "Lorg/tstrans/RtpException$Kind;";
    let kind_val = env
        .get_static_field("org/tstrans/RtpException$Kind", kind, kind_sig)?
        .l()?;
    let msg = env.new_string(message)?;
    let exc: JObject = env.new_object(
        "org/tstrans/RtpException",
        format!("({kind_sig}Ljava/lang/String;)V"),
        &[JValue::Object(&kind_val), JValue::Object(&msg)],
    )?;
    env.throw(JThrowable::from(exc))
}

/// Map a `TransportError` from `send_bytes` / `recv_bytes` onto an
/// `RtpException`. Mirrors tst-py `transport_error_to_pyerr`:
/// - `ExplicitClose` → `CANCELLED`
/// - `TooLarge`      → `MALFORMED_PACKET`
/// - all others (`Backpressure`, `Broken`, `Closed`) → `TRANSPORT`
pub(crate) fn transport_error_to_rtp(env: &mut JNIEnv, e: &TransportError) {
    match e {
        TransportError::ExplicitClose => {
            throw_rtp(env, "CANCELLED", "transport cancelled by caller")
        }
        TransportError::TooLarge { len, max } => {
            let msg = format!("payload too large: {len} bytes exceeds {max}-byte cap");
            throw_rtp(env, "MALFORMED_PACKET", &msg)
        }
        other => throw_rtp(env, "TRANSPORT", &other.to_string()),
    }
}

/// Map a `ConnectError` from `RtpSocketBuilder::build` /
/// `RtpRecvSocketBuilder::build` onto an `RtpException`. Mirrors tst-py
/// `connect_error_to_pyerr` — all connect-time failures surface as `TRANSPORT`;
/// the free-text message carries the specific Rust variant.
pub(crate) fn connect_error_to_rtp(env: &mut JNIEnv, e: &ConnectError) {
    throw_rtp(env, "TRANSPORT", &e.to_string());
}
