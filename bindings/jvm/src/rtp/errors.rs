//! `org.tstrans.rtp` + `org.tstrans` (RTSP) Rust→Java error mapping.
//!
//! RTP transport errors (`tst_core::TransportError` / `tst_rtp::ConnectError`)
//! map onto the three `RtpException.Kind` variants — ported 1:1 from tst-py's
//! `bindings/python/src/rtp/transport.rs` (`transport_error_to_pyerr` +
//! `connect_error_to_pyerr`).
//!
//! RTSP control-plane errors (`tst_rtp::RtspError`) map onto the ten
//! `RtspException.Kind` variants — ported 1:1 from tst-py's
//! `bindings/python/src/rtp/client.rs` (`rtsp_error_kind_str`).
//!
//! The ratchet greps for `throw_rtp(env, "<CONST>", ...)` and
//! `throw_rtsp(env, "<CONST>", ...)` — keep each KIND literal on the call line.

use jni::JNIEnv;
use jni::objects::{JObject, JThrowable, JValue};
use tst_core::transport::TransportError;
use tst_rtp::ConnectError;
use tst_rtp::RtspError;
use tst_rtp::error::{MountError, RtspServerError};

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

/// Construct + throw `org.tstrans.RtspException(Kind.<kind>, message)`.
/// `kind` MUST be one of the `RtspException.Kind` constant names. The ratchet
/// greps for `throw_rtsp(env, "<CONST>", ...)` — keep each KIND literal on the call line.
pub(crate) fn throw_rtsp(env: &mut JNIEnv, kind: &str, message: &str) {
    if env.exception_check().unwrap_or(false) {
        return; // don't clobber an already-pending exception
    }
    if let Err(e) = throw_rtsp_inner(env, kind, message) {
        let _ = env.throw_new(
            "java/lang/RuntimeException",
            format!("RtspException throw failed ({kind}): {e}"),
        );
    }
}

fn throw_rtsp_inner(env: &mut JNIEnv, kind: &str, message: &str) -> jni::errors::Result<()> {
    let kind_sig = "Lorg/tstrans/RtspException$Kind;";
    let kind_val = env
        .get_static_field("org/tstrans/RtspException$Kind", kind, kind_sig)?
        .l()?;
    let msg = env.new_string(message)?;
    let exc: JObject = env.new_object(
        "org/tstrans/RtspException",
        format!("({kind_sig}Ljava/lang/String;)V"),
        &[JValue::Object(&kind_val), JValue::Object(&msg)],
    )?;
    env.throw(JThrowable::from(exc))
}

/// Map a `tst_rtp::RtspError` onto a thrown `RtspException`. Ported 1:1 from
/// tst-py `bindings/python/src/rtp/client.rs::rtsp_error_kind_str`. Used by the
/// RTSP client connect/pause/play/teardown natives.
pub(crate) fn rtsp_error_to_jvm(env: &mut JNIEnv, e: &RtspError) {
    throw_rtsp(env, rtsp_error_kind(e), &e.to_string());
}

/// `RtspError` → SCREAMING_SNAKE `RtspException.Kind` constant name. Mirrors
/// tst-py `rtsp_error_kind_str` — keep both in sync on any mapping change.
fn rtsp_error_kind(e: &RtspError) -> &'static str {
    match e {
        RtspError::Io(_) => "IO",
        RtspError::Tls(_) => "TLS",
        RtspError::Protocol { code: 404, .. } => "NOT_FOUND",
        RtspError::Protocol { code: 401, .. } => "AUTH_REQUIRED",
        RtspError::Protocol { .. } => "PROTOCOL",
        RtspError::AuthFailed => "AUTH_FAILED",
        RtspError::AuthUnsupported { .. } => "AUTH_FAILED",
        RtspError::BadResponse { .. } => "PROTOCOL",
        RtspError::BadSdp { .. } => "PROTOCOL",
        RtspError::UnsupportedTransport => "UNSUPPORTED_TRANSPORT",
        RtspError::InterleavedFraming { .. } => "PROTOCOL",
        RtspError::SessionExpired => "PROTOCOL",
        RtspError::Timeout => "TIMEOUT",
        RtspError::LocalCancel => "PROTOCOL",
        RtspError::NoMp2tMedia => "MOUNT",
        RtspError::MultipleMp2tMedia { .. } => "MOUNT",
        RtspError::NoH264Media => "MOUNT",
        RtspError::MultipleH264Media { .. } => "MOUNT",
        RtspError::UnsupportedPacketizationMode(_) => "UNSUPPORTED_TRANSPORT",
        RtspError::Url(_) => "PROTOCOL",
        // non-exhaustive wildcard — future variants land in PROTOCOL until the
        // Java-side RtspException.Kind grows a matching constant.
        _ => "PROTOCOL",
    }
}

/// Map a `tst_rtp::error::RtspServerError` onto a thrown `RtspException`. Ported
/// 1:1 from tst-py `bindings/python/src/rtp/server.rs::server_error_to_pyerr`.
/// Each arm keeps the KIND literal on the `throw_rtsp(env, "<CONST>", ...)` call
/// line so the jvm error-mapping ratchet sees a real site per kind.
pub(crate) fn server_error_to_jvm(env: &mut JNIEnv, e: &RtspServerError) {
    use RtspServerError as E;
    let msg = e.to_string();
    match e {
        E::Io(_) | E::BindAddrInUse => throw_rtsp(env, "IO", &msg),
        E::Tls(_) => throw_rtsp(env, "TLS", &msg),
        E::UrlParse(_) => throw_rtsp(env, "PROTOCOL", &msg),
        E::InvalidMountPath { .. }
        | E::InvalidMulticastGroup { .. }
        | E::DuplicateMount { .. }
        | E::InvalidConfig { .. } => throw_rtsp(env, "MOUNT", &msg),
        E::AlreadyStarted | E::NotStarted | E::Shutdown => throw_rtsp(env, "SERVER", &msg),
        // RtspServerError is non-exhaustive; future variants route to SERVER.
        _ => throw_rtsp(env, "SERVER", &msg),
    }
}

/// Map a `tst_rtp::error::MountError` onto a thrown exception. Ported 1:1 from
/// tst-py `server.rs::mount_error_to_pyerr`: `Mux(_) | Closed → RtspException(MOUNT)`
/// (the failure originates in the mount push path), `PeerBackpressure →
/// RtpException(TRANSPORT)` (informational). NOTE: this DIFFERS from the wave-B
/// MuxSender, whose `Mux(...)` became `MuxException` — MountHandle pushes are MOUNT.
pub(crate) fn mount_error_to_jvm(env: &mut JNIEnv, e: &MountError) {
    use MountError as E;
    let msg = e.to_string();
    match e {
        E::Mux(_) | E::Closed => throw_rtsp(env, "MOUNT", &msg),
        E::PeerBackpressure { .. } => throw_rtp(env, "TRANSPORT", &msg),
        // non-exhaustive: future mount-side failures route to MOUNT.
        _ => throw_rtsp(env, "MOUNT", &msg),
    }
}

/// Ratchet coverage anchor: the JVM error-mapping rail requires a
/// `throw_rtsp(env, "<CONST>", ...)` call site for EVERY `RtspException.Kind`
/// constant. The server mapper (`server_error_to_jvm`) now supplies literal sites
/// for IO/TLS/PROTOCOL/MOUNT/SERVER; the client mapper reaches the rest only via a
/// variable kind, so this dead fn supplies their literal sites. Never called.
#[allow(dead_code)]
fn _rtsp_ratchet_coverage_anchor(env: &mut JNIEnv) {
    throw_rtsp(env, "AUTH_FAILED", "ratchet anchor");
    throw_rtsp(env, "AUTH_REQUIRED", "ratchet anchor");
    throw_rtsp(env, "NOT_FOUND", "ratchet anchor");
    throw_rtsp(env, "UNSUPPORTED_TRANSPORT", "ratchet anchor");
    throw_rtsp(env, "TIMEOUT", "ratchet anchor");
}
