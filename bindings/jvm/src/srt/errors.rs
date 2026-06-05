//! `org.tstrans.srt` Rust→Java error mapping. Centralized exhaustive mappings
//! from every tst_srt / tst_core error enum that flows through the SRT surface
//! to one of the 8 `SrtException.Kind` variants. Ported 1:1 from tst-py's
//! `bindings/python/src/srt/errors.rs`. The ratchet greps for
//! `throw_srt(env, "<CONST>", ...)` — keep each KIND literal on the call line.

use jni::JNIEnv;
use jni::objects::{JObject, JThrowable, JValue};
use tst_core::transport::TransportError;
use tst_srt::UrlError;
use tst_srt::error::{AcceptError, BindError, ConnectError, IoError};

/// Construct + throw `org.tstrans.SrtException(Kind.<kind>, message)`.
/// `kind` MUST be one of the `SrtException.Kind` enum constant names
/// (SCREAMING_SNAKE_CASE). The ratchet greps for `throw_srt(env, "<CONST>", ...)`.
pub(crate) fn throw_srt(env: &mut JNIEnv, kind: &str, message: &str) {
    if env.exception_check().unwrap_or(false) {
        return; // don't clobber an already-pending exception
    }
    if let Err(e) = throw_srt_inner(env, kind, message) {
        let _ = env.throw_new(
            "java/lang/RuntimeException",
            format!("SrtException throw failed ({kind}): {e}"),
        );
    }
}

fn throw_srt_inner(env: &mut JNIEnv, kind: &str, message: &str) -> jni::errors::Result<()> {
    let kind_sig = "Lorg/tstrans/SrtException$Kind;";
    let kind_val = env
        .get_static_field("org/tstrans/SrtException$Kind", kind, kind_sig)?
        .l()?;
    let msg = env.new_string(message)?;
    let exc: JObject = env.new_object(
        "org/tstrans/SrtException",
        format!("({kind_sig}Ljava/lang/String;)V"),
        &[JValue::Object(&kind_val), JValue::Object(&msg)],
    )?;
    env.throw(JThrowable::from(exc))
}

/// Map a `tst_srt::UrlError` (raised by `SrtUrl::parse`) to
/// `SrtException(Kind.CONFIG_INVALID)`. All variants are caller
/// misconfiguration by definition.
pub(crate) fn url_error(env: &mut JNIEnv, e: &UrlError) {
    throw_srt(env, "CONFIG_INVALID", &e.to_string());
}

/// Map a `tst_srt::ConnectError` (raised by `Socket::connect_with`) to
/// an `SrtException`. `InvalidAddress` / `InvalidOption` → CONFIG_INVALID;
/// `TimedOut` → TIMEOUT; everything else → CONNECT_FAILED.
pub(crate) fn connect_error(env: &mut JNIEnv, e: &ConnectError) {
    let msg = e.to_string();
    match e {
        ConnectError::InvalidAddress(_) | ConnectError::InvalidOption(_) => {
            throw_srt(env, "CONFIG_INVALID", &msg)
        }
        ConnectError::TimedOut => throw_srt(env, "TIMEOUT", &msg),
        ConnectError::Refused
        | ConnectError::BadEncryption { .. }
        | ConnectError::Rejected { .. }
        | ConnectError::System(_)
        | ConnectError::Other { .. } => throw_srt(env, "CONNECT_FAILED", &msg),
        // Catch-all for future #[non_exhaustive] additions.
        _ => throw_srt(env, "CONNECT_FAILED", &msg),
    }
}

/// Map a `tst_srt::BindError` (raised by `Listener::bind_with`) to an
/// `SrtException`. `InvalidAddress` / `InvalidOption` → CONFIG_INVALID;
/// everything else → CONNECT_FAILED (the listener failed to come up,
/// treated as a connect-side failure).
pub(crate) fn bind_error(env: &mut JNIEnv, e: &BindError) {
    let msg = e.to_string();
    match e {
        BindError::InvalidAddress(_) | BindError::InvalidOption(_) => {
            throw_srt(env, "CONFIG_INVALID", &msg)
        }
        BindError::AddressInUse
        | BindError::PermissionDenied
        | BindError::System(_)
        | BindError::Other { .. } => throw_srt(env, "CONNECT_FAILED", &msg),
        // Catch-all for future #[non_exhaustive] additions.
        _ => throw_srt(env, "CONNECT_FAILED", &msg),
    }
}

/// Map a `tst_srt::AcceptError` (raised by `Listener::accept`) to an
/// `SrtException`. `TimedOut` → TIMEOUT; `ListenerClosed` → CLOSED;
/// everything else → ACCEPT_FAILED.
pub(crate) fn accept_error(env: &mut JNIEnv, e: &AcceptError) {
    let msg = e.to_string();
    match e {
        AcceptError::TimedOut => throw_srt(env, "TIMEOUT", &msg),
        AcceptError::ListenerClosed => throw_srt(env, "CLOSED", &msg),
        AcceptError::PeerRejected { .. } | AcceptError::System(_) | AcceptError::Other { .. } => {
            throw_srt(env, "ACCEPT_FAILED", &msg)
        }
        // Catch-all for future #[non_exhaustive] additions.
        _ => throw_srt(env, "ACCEPT_FAILED", &msg),
    }
}

/// Map a `tst_srt::error::IoError` (raised by `SrtTransport::stats` and
/// other low-level libsrt IO paths) to an `SrtException`.
/// `SocketClosed` → CLOSED; everything else → IO.
pub(crate) fn io_error(env: &mut JNIEnv, e: &IoError) {
    let msg = e.to_string();
    match e {
        IoError::SocketClosed => throw_srt(env, "CLOSED", &msg),
        IoError::System(_) | IoError::Other { .. } => throw_srt(env, "IO", &msg),
        // Catch-all for future #[non_exhaustive] additions.
        _ => throw_srt(env, "IO", &msg),
    }
}

/// Map a `tst_core::transport::TransportError` (used by `tst_pipeline::Sender`
/// / `Receiver`) to an `SrtException`. `Backpressure` → WOULD_BLOCK;
/// `Broken` → BROKEN; `Closed`/`ExplicitClose` → CLOSED; `TooLarge` →
/// CONFIG_INVALID; future variants → IO.
pub(crate) fn transport_error(env: &mut JNIEnv, e: &TransportError) {
    match e {
        TransportError::Backpressure { msg, .. } => throw_srt(env, "WOULD_BLOCK", msg),
        TransportError::Broken { msg, .. } => throw_srt(env, "BROKEN", msg),
        TransportError::Closed => throw_srt(env, "CLOSED", "transport closed"),
        TransportError::ExplicitClose => throw_srt(env, "CLOSED", "transport explicit close"),
        TransportError::TooLarge { len, max } => throw_srt(
            env,
            "CONFIG_INVALID",
            &format!("payload too large: {len} bytes exceeds {max}-byte cap"),
        ),
        // Catch-all for future #[non_exhaustive] additions.
        other => throw_srt(env, "IO", &other.to_string()),
    }
}
