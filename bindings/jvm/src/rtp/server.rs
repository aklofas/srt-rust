//! `org.tstrans.rtp` RTSP SERVER JNI surface — `RtspServer`, `MountHandle`,
//! `RtspServerCancelHandle`. Ports tst-py's `bindings/python/src/rtp/server.rs`.
//! The underlying `tst_rtp::rtsp::server::RtspServer` owns a tokio Runtime inside
//! the native Box; there is no JNI-side async handling.

use std::time::Duration;

use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::{jboolean, jint, jlong};
use secrecy::SecretString;

use tst_rtp::RtspServer as RustRtspServer;
use tst_rtp::RtspServerCancelHandle as RustServerCancel;
use tst_rtp::ServerStats as RustServerStats;
use tst_rtp::builder::RtspServerBuilder;
use tst_rtp::error::RtspServerError;

use super::errors::{server_error_to_jvm, throw_rtsp};

/// Boxed behind `org.tstrans.rtp.RtspServerCancelHandle.handle`. Wraps tst-rtp's
/// `RtspServerCancelHandle` (Clone; owns its own `Arc<AtomicBool>` flag, so it is
/// independent of the `RtspServer` box lifetime).
pub(super) struct JniRtspServerCancel {
    pub(super) inner: RustServerCancel,
}

impl JniRtspServerCancel {
    pub(super) fn into_handle(self) -> jlong {
        Box::into_raw(Box::new(self)) as jlong
    }
}

/// Fire the HARD cancel. Guards a closed (zero) handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServerCancelHandle_nCancel(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle == 0 {
        let _ = env.throw_new(
            "java/lang/IllegalStateException",
            "RtspServerCancelHandle is closed",
        );
        return;
    }
    // SAFETY: valid Box<JniRtspServerCancel> kept alive by the Java object.
    let c = unsafe { &*(handle as *const JniRtspServerCancel) };
    c.inner.cancel();
}

/// Report whether the flag was flipped. Guards a closed handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServerCancelHandle_nIsCancelled(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jboolean {
    if handle == 0 {
        let _ = env.throw_new(
            "java/lang/IllegalStateException",
            "RtspServerCancelHandle is closed",
        );
        return 0;
    }
    // SAFETY: valid Box<JniRtspServerCancel> kept alive by the Java object.
    let c = unsafe { &*(handle as *const JniRtspServerCancel) };
    // tst-rtp uses American spelling is_canceled(); the JVM method is isCancelled().
    u8::from(c.inner.is_canceled())
}

/// Free the boxed cancel handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServerCancelHandle_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: valid Box<JniRtspServerCancel>; close() zeroes the field (runs once).
        drop(unsafe { Box::from_raw(handle as *mut JniRtspServerCancel) });
    }
}

// ---------------------------------------------------------------------------
// RtspServer lifecycle.
// ---------------------------------------------------------------------------

type ServerInner = RustRtspServer;

/// Build the Java `org.tstrans.rtp.ServerStats` record. Ctor `(JJJJ)V`.
fn build_server_stats<'local>(
    env: &mut JNIEnv<'local>,
    s: &RustServerStats,
) -> jni::errors::Result<JObject<'local>> {
    env.ensure_local_capacity(4)?;
    env.new_object(
        "org/tstrans/rtp/ServerStats",
        "(JJJJ)V",
        &[
            JValue::Long(s.active_sessions as i64),
            JValue::Long(s.total_rtp_packets_sent as i64),
            JValue::Long(s.total_rtp_bytes_sent as i64),
            JValue::Long(s.mounts as i64),
        ],
    )
}

/// Read a JString, returning "" for null/error (auth fields).
fn jstring_or_empty(env: &mut JNIEnv, s: &JString) -> String {
    if s.is_null() {
        return String::new();
    }
    env.get_string(s).map(Into::into).unwrap_or_default()
}

/// `RtspServer.nStart(...)` — build the RtspServerBuilder from config primitives,
/// build()+start(), then (mirroring tst-py) raise RtspException(TLS) if either TLS
/// PEM was set. Returns a `Box<RtspServer>` handle, or 0 with a pending exception.
/// `authScheme`: -1 none / 0 basic / 1 digest-md5 / 2 digest-sha256.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServer_nStart<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    bind_addr: JString<'local>,
    max_sessions: jlong,
    session_timeout_secs: jlong,
    fanout_capacity: jlong,
    graceful_shutdown_drain_ms: jlong,
    auth_scheme: jint,
    auth_realm: JString<'local>,
    auth_user: JString<'local>,
    auth_password: JString<'local>,
    has_tls_cert: jboolean,
    has_tls_key: jboolean,
) -> jlong {
    let bind: String = match env.get_string(&bind_addr) {
        Ok(s) => s.into(),
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return 0;
        }
    };
    // Mirror ServerConfigExtract: prepend rtsp:// if no scheme present.
    let bind_url = if bind.starts_with("rtsp://") || bind.starts_with("rtsps://") {
        bind
    } else {
        format!("rtsp://{bind}")
    };

    // Read auth strings up front (avoids borrowing env inside the build closure).
    let auth = if auth_scheme >= 0 {
        let realm = jstring_or_empty(&mut env, &auth_realm);
        let user = jstring_or_empty(&mut env, &auth_user);
        let pass = jstring_or_empty(&mut env, &auth_password);
        Some((auth_scheme, realm, user, pass))
    } else {
        None
    };

    let built: Result<RustRtspServer, RtspServerError> = (|| {
        let mut builder = RtspServerBuilder::new(&bind_url)?;
        builder
            .max_sessions(max_sessions.max(0) as usize)
            .session_timeout(Duration::from_secs(session_timeout_secs.max(0) as u64))
            .fanout_capacity(fanout_capacity.max(0) as usize)
            .graceful_shutdown_drain(Duration::from_millis(
                graceful_shutdown_drain_ms.max(0) as u64
            ));
        if let Some((scheme, realm, user, pass)) = auth.as_ref() {
            let secret = SecretString::from(pass.clone());
            match scheme {
                0 => {
                    builder.auth_basic(realm, user, secret);
                }
                1 => {
                    builder.auth_digest_md5(realm, user, secret);
                }
                _ => {
                    builder.auth_digest_sha256(realm, user, secret);
                }
            }
        }
        let server = builder.build()?;
        server.start()?;
        Ok(server)
    })();

    let server = match built {
        Ok(s) => s,
        Err(e) => {
            server_error_to_jvm(&mut env, &e);
            return 0;
        }
    };

    // TLS guard (mirror tst-py order: build+start, THEN refuse). Dropping `server`
    // here fires its Drop (hard-cancel + runtime shutdown).
    if has_tls_cert != 0 || has_tls_key != 0 {
        throw_rtsp(
            &mut env,
            "TLS",
            "TLS (rtsps://) is not enabled in this build of tstrans; \
             rebuild with the tst-rtp `tls` feature wired through tst-jni",
        );
        return 0;
    }

    Box::into_raw(Box::new(server)) as jlong
}

fn checked_server(env: &mut JNIEnv, handle: jlong) -> Option<*const ServerInner> {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "RtspServer is closed");
        return None;
    }
    Some(handle as *const ServerInner)
}

/// `RtspServer.nStats(handle)` → ServerStats record, or null on a JNI builder error.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServer_nStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObject<'local> {
    let Some(ptr) = checked_server(&mut env, handle) else {
        return JObject::null();
    };
    // SAFETY: validated non-zero live Box<RtspServer>; stats() takes &self.
    let server = unsafe { &*ptr };
    let s = server.stats();
    build_server_stats(&mut env, &s).unwrap_or_else(|_| JObject::null())
}

/// `RtspServer.nLocalAddr(handle)` → "ip:port" or null.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServer_nLocalAddr<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JString<'local> {
    let Some(ptr) = checked_server(&mut env, handle) else {
        return JObject::null().into();
    };
    // SAFETY: validated non-zero live Box<RtspServer>; local_addr() takes &self.
    let server = unsafe { &*ptr };
    match server.local_addr() {
        Some(addr) => env
            .new_string(addr.to_string())
            .unwrap_or_else(|_| JObject::null().into()),
        None => JObject::null().into(),
    }
}

/// `RtspServer.nStop(handle, drainMs)` — graceful shutdown. `drainMs` is accepted
/// for API stability but ignored (the builder's graceful_shutdown_drain governs the
/// real window), mirroring tst-py.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServer_nStop(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    _drain_ms: jlong,
) {
    let Some(ptr) = checked_server(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live Box<RtspServer>; stop() takes &self.
    let server = unsafe { &*ptr };
    if let Err(e) = server.stop() {
        server_error_to_jvm(&mut env, &e);
    }
}

/// `RtspServer.nCancelHandle(handle)` → Box<JniRtspServerCancel> handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServer_nCancelHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_server(&mut env, handle) else {
        return 0;
    };
    // SAFETY: validated non-zero live Box<RtspServer>; cancel_handle() takes &self
    // and hands back an independent Clone (own Arc<AtomicBool>).
    let server = unsafe { &*ptr };
    JniRtspServerCancel {
        inner: server.cancel_handle(),
    }
    .into_handle()
}

/// `RtspServer.nClose(handle)` — best-effort graceful stop (swallow NotStarted),
/// then drop the box (Drop runs hard-cancel + runtime shutdown). Mirrors tst-py
/// `__exit__`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServer_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: handle from Box::into_raw, dropped once (Java zeroes its field).
        let b = unsafe { Box::from_raw(handle as *mut ServerInner) };
        let _ = b.stop(); // best-effort; NotStarted/Shutdown swallowed
        drop(b);
    }
}
