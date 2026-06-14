//! `org.tstrans.rtp` RTSP SERVER JNI surface — `RtspServer`, `MountHandle`,
//! `RtspServerCancelHandle`. Ports tst-py's `bindings/python/src/rtp/server.rs`.
//! The underlying `tst_rtp::rtsp::server::RtspServer` owns a tokio Runtime; it is
//! held in a per-type leased [`HandleRegistry`] (not a raw box), and there is no
//! JNI-side async handling.

use std::sync::LazyLock;
use std::time::Duration;

use jni::JNIEnv;
use jni::objects::{
    JBooleanArray, JByteArray, JClass, JIntArray, JLongArray, JObject, JString, JValue,
};
use jni::sys::{jboolean, jint, jlong};
use secrecy::SecretString;

use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{
    AudioStreamHandle, DataStreamHandle, KlvStreamHandle, SubtitleStreamHandle, VideoStreamHandle,
};
use tst_rtp::RtspServer as RustRtspServer;
use tst_rtp::RtspServerCancelHandle as RustServerCancel;
use tst_rtp::ServerStats as RustServerStats;
use tst_rtp::builder::RtspServerBuilder;
use tst_rtp::error::RtspServerError;
use tst_rtp::rtsp::server::mount::{MountHandle as RustMountHandle, MountKind};

use crate::handle::HandleRegistry;
use crate::jutil::checked_u8;
use crate::mpegts::muxer::build_muxer_config_from_arrays;

use super::errors::{mount_error_to_jvm, server_error_to_jvm, throw_rtsp};

/// Boxed behind `org.tstrans.rtp.RtspServerCancelHandle.handle`. Wraps tst-rtp's
/// `RtspServerCancelHandle` (Clone; owns its own `Arc<AtomicBool>` flag, so it is
/// independent of the `RtspServer` lifetime).
pub(super) struct JniRtspServerCancel {
    pub(super) inner: RustServerCancel,
}

/// Per-type leased-handle registry for `org.tstrans.rtp.RtspServerCancelHandle`.
/// A cancel target (no parked op to wake) — register with `insert` (cancel = None).
static REGISTRY_CANCEL: LazyLock<HandleRegistry<JniRtspServerCancel>> =
    LazyLock::new(HandleRegistry::new);

impl JniRtspServerCancel {
    pub(super) fn into_handle(self) -> jlong {
        REGISTRY_CANCEL.insert(self) as jlong
    }
}

/// Fire the HARD cancel. Guards a closed (zero) handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServerCancelHandle_nCancel(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        if REGISTRY_CANCEL
            .with(handle as u64, |c| c.inner.cancel())
            .is_none()
        {
            let _ = env.throw_new(
                "java/lang/IllegalStateException",
                "RtspServerCancelHandle is closed",
            );
        }
    })
}

/// Report whether the flag was flipped. Guards a closed handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServerCancelHandle_nIsCancelled(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jboolean {
    // tst-rtp uses American spelling is_canceled(); the JVM method is isCancelled().
    // A cancel target is never "parked", so `try_with` never reports `Locked`;
    // treat `Locked`/`Taken` as closed.
    crate::panic::jni_catch(&mut env, 0, |env| {
        match REGISTRY_CANCEL.try_with(handle as u64, |c| u8::from(c.inner.is_canceled())) {
            crate::handle::TryWith::Ran(v) => v,
            _ => {
                let _ = env.throw_new(
                    "java/lang/IllegalStateException",
                    "RtspServerCancelHandle is closed",
                );
                0
            }
        }
    })
}

/// Free the boxed cancel handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServerCancelHandle_nClose(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    // Atomic + idempotent drop.
    crate::panic::jni_catch(&mut env, (), |_env| {
        let _ = REGISTRY_CANCEL.close(handle as u64);
    })
}

// ---------------------------------------------------------------------------
// RtspServer lifecycle.
// ---------------------------------------------------------------------------

type ServerInner = RustRtspServer;

/// Per-type leased-handle registry for `org.tstrans.rtp.RtspServer`. Registered
/// with a cancel hook (a HARD cancel via the server's own independent
/// `RtspServerCancelHandle`) so a `close` racing any server op wakes it before
/// the resource is taken for teardown.
static REGISTRY_SERVER: LazyLock<HandleRegistry<ServerInner>> = LazyLock::new(HandleRegistry::new);

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
    crate::panic::jni_catch(&mut env, 0, |env| {
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
            let realm = jstring_or_empty(env, &auth_realm);
            let user = jstring_or_empty(env, &auth_user);
            let pass = jstring_or_empty(env, &auth_password);
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
                server_error_to_jvm(env, &e);
                return 0;
            }
        };

        // TLS guard (mirror tst-py order: build+start, THEN refuse). Dropping `server`
        // here fires its Drop (hard-cancel + runtime shutdown).
        if has_tls_cert != 0 || has_tls_key != 0 {
            throw_rtsp(
                env,
                "TLS",
                "TLS (rtsps://) is not enabled in this build of tstrans; \
                 rebuild with the tst-rtp `tls` feature wired through tst-jni",
            );
            return 0;
        }

        // The cancel hook drives a HARD cancel via the server's own independent
        // `RtspServerCancelHandle` (own Arc<AtomicBool>), wiring `close` to wake any
        // racing server op before the resource is taken for teardown.
        let cancel = server.cancel_handle();
        REGISTRY_SERVER.insert_with_cancel(server, Some(Box::new(move || cancel.cancel()))) as jlong
    })
}

/// Lease the server for `handle` and run `f` on it under the entry's resource
/// lock. `None` (absent/closed) → throw `IllegalStateException` (the caller maps
/// `None` to its own default return). Every server method takes `&self`.
fn with_server<R>(env: &mut JNIEnv, handle: jlong, f: impl FnOnce(&ServerInner) -> R) -> Option<R> {
    match REGISTRY_SERVER.with(handle as u64, |s| f(s)) {
        Some(r) => Some(r),
        None => {
            let _ = env.throw_new("java/lang/IllegalStateException", "RtspServer is closed");
            None
        }
    }
}

/// `RtspServer.nStats(handle)` → ServerStats record, or null on a JNI builder error.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServer_nStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObject<'local> {
    crate::panic::jni_catch(&mut env, JObject::null(), |env| {
        let Some(s) = with_server(env, handle, |server| server.stats()) else {
            return JObject::null();
        };
        build_server_stats(env, &s).unwrap_or_else(|_| JObject::null())
    })
}

/// `RtspServer.nLocalAddr(handle)` → "ip:port" or null.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServer_nLocalAddr<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JString<'local> {
    crate::panic::jni_catch(&mut env, JObject::null().into(), |env| {
        let Some(addr) = with_server(env, handle, |server| server.local_addr()) else {
            return JObject::null().into();
        };
        match addr {
            Some(addr) => env
                .new_string(addr.to_string())
                .unwrap_or_else(|_| JObject::null().into()),
            None => JObject::null().into(),
        }
    })
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
    crate::panic::jni_catch(&mut env, (), |env| {
        let Some(res) = with_server(env, handle, |server| server.stop()) else {
            return;
        };
        if let Err(e) = res {
            server_error_to_jvm(env, &e);
        }
    })
}

/// `RtspServer.nCancelHandle(handle)` → Box<JniRtspServerCancel> handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServer_nCancelHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    // cancel_handle() takes &self and hands back an independent Clone (own
    // Arc<AtomicBool>), so the returned handle survives the server's lifetime.
    crate::panic::jni_catch(&mut env, 0, |env| {
        with_server(env, handle, |server| {
            JniRtspServerCancel {
                inner: server.cancel_handle(),
            }
            .into_handle()
        })
        .unwrap_or(0)
    })
}

/// `RtspServer.nClose(handle)` — best-effort graceful stop (swallow NotStarted),
/// then drop the box (Drop runs hard-cancel + runtime shutdown). Mirrors tst-py
/// `__exit__`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServer_nClose(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    // Atomic + idempotent: the cancel hook fires a HARD cancel first (waking any
    // racing server op), then only the winning close gets the server back for a
    // best-effort graceful stop (NotStarted/Shutdown swallowed). Dropping it fires
    // its Drop (runtime shutdown). A second close finds the id gone → no-op.
    crate::panic::jni_catch(&mut env, (), |_env| {
        if let Some(server) = REGISTRY_SERVER.close(handle as u64) {
            let _ = server.stop(); // best-effort
            drop(server);
        }
    })
}

// ---------------------------------------------------------------------------
// Mount factories (on RtspServer) + MountHandle push surface.
// ---------------------------------------------------------------------------

type MountInner = RustMountHandle;

/// Per-type leased-handle registry for `org.tstrans.rtp.MountHandle`. No cancel
/// hook (push/flush calls don't park indefinitely); `close` just frees the
/// handle wrapper — the mount itself persists in the server until stop()/close().
static REGISTRY_MOUNT: LazyLock<HandleRegistry<MountInner>> = LazyLock::new(HandleRegistry::new);

/// Lease the mount for `handle` and run `f` on it under the entry's resource
/// lock. `None` (absent/closed) → throw `IllegalStateException` (the caller maps
/// `None` to its own default return). Every `MountHandle` method takes `&self`
/// (Arc-shared internally), so concurrent leases from multiple Java threads are
/// sound.
fn with_mount<R>(env: &mut JNIEnv, handle: jlong, f: impl FnOnce(&MountInner) -> R) -> Option<R> {
    match REGISTRY_MOUNT.with(handle as u64, |m| f(m)) {
        Some(r) => Some(r),
        None => {
            let _ = env.throw_new("java/lang/IllegalStateException", "MountHandle is closed");
            None
        }
    }
}

/// Read a Java `byte[]` argument, or throw `RuntimeException` + return `None`.
fn read_bytes(env: &mut JNIEnv, arr: &JByteArray) -> Option<Vec<u8>> {
    match env.convert_byte_array(arr) {
        Ok(b) => Some(b),
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            None
        }
    }
}

/// `RtspServer.nAddUnicastMount(serverHandle, path, ...muxerConfig arrays...)` →
/// Box<MountHandle> handle, or 0 with a pending RtspException.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServer_nAddUnicastMount<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    server_handle: jlong,
    path: JString<'local>,
    program_number: jint,
    pmt_pid: jint,
    pcr_pid: jint,
    pcr_interval_ms: jint,
    psi_interval_ms: jint,
    buffer_packets: jint,
    av1_carriage: jint,
    stream_pids: JIntArray<'local>,
    stream_kinds: JIntArray<'local>,
    stream_codecs: JIntArray<'local>,
    stream_type_codes: JIntArray<'local>,
    stream_carries_pts: JBooleanArray<'local>,
    data_desc_bytes: JByteArray<'local>,
    data_desc_lens: JIntArray<'local>,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        // Validate the server handle up front (throws if closed); the cfg/path build
        // touches `env`, so the actual `add_mount` is leased separately below.
        if with_server(env, server_handle, |_| ()).is_none() {
            return 0;
        }
        let cfg = match build_muxer_config_from_arrays(
            env,
            program_number,
            pmt_pid,
            pcr_pid,
            pcr_interval_ms,
            psi_interval_ms,
            buffer_packets,
            av1_carriage,
            &stream_pids,
            &stream_kinds,
            &stream_codecs,
            &stream_type_codes,
            &stream_carries_pts,
            &data_desc_bytes,
            &data_desc_lens,
        ) {
            Ok(c) => c,
            Err(()) => return 0, // pending MuxException
        };
        let path_str: String = match env.get_string(&path) {
            Ok(s) => s.into(),
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                return 0;
            }
        };
        // add_mount takes &self; lease the server again and run it under the lock. A
        // server closed between the two leases yields None → IllegalStateException.
        let Some(res) = with_server(env, server_handle, |server| {
            server.add_mount(&path_str, cfg)
        }) else {
            return 0;
        };
        match res {
            Ok(mh) => REGISTRY_MOUNT.insert(mh) as jlong,
            Err(e) => {
                server_error_to_jvm(env, &e);
                0
            }
        }
    })
}

/// `RtspServer.nAddMulticastMount(serverHandle, path, group, port, ttl, iface,
/// ...muxerConfig arrays...)` → Box<MountHandle> handle, or 0 with a pending
/// RtspException. Builds the `rtp://group:port?ttl=N[&iface=...]` URL the Rust
/// `add_multicast_mount` expects (mirror `PyRtspServer::add_multicast_mount`).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServer_nAddMulticastMount<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    server_handle: jlong,
    path: JString<'local>,
    group: JString<'local>,
    port: jint,
    ttl: jint,
    iface: JString<'local>,
    program_number: jint,
    pmt_pid: jint,
    pcr_pid: jint,
    pcr_interval_ms: jint,
    psi_interval_ms: jint,
    buffer_packets: jint,
    av1_carriage: jint,
    stream_pids: JIntArray<'local>,
    stream_kinds: JIntArray<'local>,
    stream_codecs: JIntArray<'local>,
    stream_type_codes: JIntArray<'local>,
    stream_carries_pts: JBooleanArray<'local>,
    data_desc_bytes: JByteArray<'local>,
    data_desc_lens: JIntArray<'local>,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        // Validate the server handle up front (throws if closed); the URL/cfg build
        // touches `env`, so the actual `add_multicast_mount` is leased separately.
        if with_server(env, server_handle, |_| ()).is_none() {
            return 0;
        }
        let cfg = match build_muxer_config_from_arrays(
            env,
            program_number,
            pmt_pid,
            pcr_pid,
            pcr_interval_ms,
            psi_interval_ms,
            buffer_packets,
            av1_carriage,
            &stream_pids,
            &stream_kinds,
            &stream_codecs,
            &stream_type_codes,
            &stream_carries_pts,
            &data_desc_bytes,
            &data_desc_lens,
        ) {
            Ok(c) => c,
            Err(()) => return 0, // pending MuxException
        };
        let path_str: String = match env.get_string(&path) {
            Ok(s) => s.into(),
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                return 0;
            }
        };
        let group_str: String = match env.get_string(&group) {
            Ok(s) => s.into(),
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                return 0;
            }
        };
        // Build the `rtp://<group>:<port>?ttl=N[&iface=...]` URL (mirror tst-py).
        let mut url = format!("rtp://{group_str}:{port}?ttl={ttl}");
        if !iface.is_null() {
            if let Ok(i) = env.get_string(&iface) {
                let i: String = i.into();
                url.push_str("&iface=");
                url.push_str(&i);
            }
        }
        // add_multicast_mount takes &self; lease the server again and run it under the
        // lock. A server closed between leases yields None → IllegalStateException.
        let Some(res) = with_server(env, server_handle, |server| {
            server.add_multicast_mount(&path_str, cfg, &url)
        }) else {
            return 0;
        };
        match res {
            Ok(mh) => REGISTRY_MOUNT.insert(mh) as jlong,
            Err(e) => {
                server_error_to_jvm(env, &e);
                0
            }
        }
    })
}

// ── MountHandle: identity / introspection ──────────────────────────────────

/// `MountHandle.nMountPath(handle)` → registered mount path string.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nMountPath<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JString<'local> {
    crate::panic::jni_catch(&mut env, JObject::null().into(), |env| {
        let Some(path) = with_mount(env, handle, |inner| inner.mount_path().to_owned()) else {
            return JObject::null().into();
        };
        env.new_string(path)
            .unwrap_or_else(|_| JObject::null().into())
    })
}

/// `MountHandle.nPeerCount(handle)` → live subscriber count.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nPeerCount(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        with_mount(env, handle, |inner| inner.peer_count() as jlong).unwrap_or(0)
    })
}

/// `MountHandle.nMountKind(handle)` → "unicast" / "multicast" / "unknown".
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nMountKind<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JString<'local> {
    crate::panic::jni_catch(&mut env, JObject::null().into(), |env| {
        let Some(kind) = with_mount(env, handle, |inner| match inner.mount_kind() {
            MountKind::Unicast => "unicast",
            MountKind::Multicast { .. } => "multicast",
            _ => "unknown",
        }) else {
            return JObject::null().into();
        };
        env.new_string(kind)
            .unwrap_or_else(|_| JObject::null().into())
    })
}

/// Build the Java `org.tstrans.rtp.MountStats` record. Ctor `(JJJJ)V`.
fn build_mount_stats<'local>(
    env: &mut JNIEnv<'local>,
    s: &tst_rtp::rtsp::server::mount::MountStats,
) -> jni::errors::Result<JObject<'local>> {
    env.ensure_local_capacity(4)?;
    env.new_object(
        "org/tstrans/rtp/MountStats",
        "(JJJJ)V",
        &[
            JValue::Long(s.bytes_pushed as i64),
            JValue::Long(s.packets_pushed as i64),
            JValue::Long(s.peer_count as i64),
            JValue::Long(s.frames_dropped_total as i64),
        ],
    )
}

/// `MountHandle.nStats(handle)` → MountStats record, or null on a JNI builder error.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObject<'local> {
    crate::panic::jni_catch(&mut env, JObject::null(), |env| {
        let Some(s) = with_mount(env, handle, |inner| inner.stats()) else {
            return JObject::null();
        };
        build_mount_stats(env, &s).unwrap_or_else(|_| JObject::null())
    })
}

// ── MountHandle: push family — single-stream variants ───────────────────────

/// `MountHandle.nPushVideo(handle, nal, pts, keyFrame)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nPushVideo<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    nal: JByteArray<'local>,
    pts: jlong,
    key_frame: jboolean,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        let Some(buf) = read_bytes(env, &nal) else {
            return;
        };
        let Some(res) = with_mount(env, handle, |inner| {
            inner.push_video(&buf, Pts90khz::new(pts), key_frame != 0)
        }) else {
            return;
        };
        if let Err(e) = res {
            mount_error_to_jvm(env, &e);
        }
    })
}

/// `MountHandle.nPushKlv(handle, klv, pts, metadataServiceId)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nPushKlv<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    klv: JByteArray<'local>,
    pts: jlong,
    metadata_service_id: jint,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        let Ok(service_id) = checked_u8(env, i64::from(metadata_service_id), "metadataServiceId")
        else {
            return; // IllegalArgumentException pending
        };
        let Some(buf) = read_bytes(env, &klv) else {
            return;
        };
        let Some(res) = with_mount(env, handle, |inner| {
            inner.push_klv(&buf, Pts90khz::new(pts), service_id)
        }) else {
            return;
        };
        if let Err(e) = res {
            mount_error_to_jvm(env, &e);
        }
    })
}

/// `MountHandle.nPushAudio(handle, frames, pts)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nPushAudio<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    frames: JByteArray<'local>,
    pts: jlong,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        let Some(buf) = read_bytes(env, &frames) else {
            return;
        };
        let Some(res) = with_mount(env, handle, |inner| {
            inner.push_audio(&buf, Pts90khz::new(pts))
        }) else {
            return;
        };
        if let Err(e) = res {
            mount_error_to_jvm(env, &e);
        }
    })
}

/// `MountHandle.nPushSubtitle(handle, pts, payload)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nPushSubtitle<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    pts: jlong,
    payload: JByteArray<'local>,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        let Some(buf) = read_bytes(env, &payload) else {
            return;
        };
        let Some(res) = with_mount(env, handle, |inner| {
            inner.push_subtitle(&buf, Pts90khz::new(pts))
        }) else {
            return;
        };
        if let Err(e) = res {
            mount_error_to_jvm(env, &e);
        }
    })
}

/// `MountHandle.nPushData(handle, data, pts)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nPushData<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    data: JByteArray<'local>,
    pts: jlong,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        let Some(buf) = read_bytes(env, &data) else {
            return;
        };
        let Some(res) = with_mount(env, handle, |inner| {
            inner.push_data(&buf, Pts90khz::new(pts))
        }) else {
            return;
        };
        if let Err(e) = res {
            mount_error_to_jvm(env, &e);
        }
    })
}

// ── MountHandle: push family — handle-targeted variants ─────────────────────

/// `MountHandle.nPushVideoTo(handle, streamHandleRaw, nal, pts, keyFrame)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nPushVideoTo<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    stream_handle_raw: jlong,
    nal: JByteArray<'local>,
    pts: jlong,
    key_frame: jboolean,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        let h = match VideoStreamHandle::try_from_raw(stream_handle_raw as u32) {
            Ok(h) => h,
            Err(_) => {
                // MountHandle has no transport concept — a forged handle is a MOUNT error
                // (DIFFERS from MuxSender, which maps forged handles to RtpException(TRANSPORT)).
                throw_rtsp(env, "MOUNT", "invalid stream handle");
                return;
            }
        };
        let Some(buf) = read_bytes(env, &nal) else {
            return;
        };
        let Some(res) = with_mount(env, handle, |inner| {
            inner.push_video_to(h, &buf, Pts90khz::new(pts), key_frame != 0)
        }) else {
            return;
        };
        if let Err(e) = res {
            mount_error_to_jvm(env, &e);
        }
    })
}

/// `MountHandle.nPushKlvTo(handle, streamHandleRaw, klv, pts, metadataServiceId)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nPushKlvTo<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    stream_handle_raw: jlong,
    klv: JByteArray<'local>,
    pts: jlong,
    metadata_service_id: jint,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        let h = match KlvStreamHandle::try_from_raw(stream_handle_raw as u32) {
            Ok(h) => h,
            Err(_) => {
                throw_rtsp(env, "MOUNT", "invalid stream handle");
                return;
            }
        };
        let Ok(service_id) = checked_u8(env, i64::from(metadata_service_id), "metadataServiceId")
        else {
            return;
        };
        let Some(buf) = read_bytes(env, &klv) else {
            return;
        };
        let Some(res) = with_mount(env, handle, |inner| {
            inner.push_klv_to(h, &buf, Pts90khz::new(pts), service_id)
        }) else {
            return;
        };
        if let Err(e) = res {
            mount_error_to_jvm(env, &e);
        }
    })
}

/// `MountHandle.nPushAudioTo(handle, streamHandleRaw, frames, pts)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nPushAudioTo<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    stream_handle_raw: jlong,
    frames: JByteArray<'local>,
    pts: jlong,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        let h = match AudioStreamHandle::try_from_raw(stream_handle_raw as u32) {
            Ok(h) => h,
            Err(_) => {
                throw_rtsp(env, "MOUNT", "invalid stream handle");
                return;
            }
        };
        let Some(buf) = read_bytes(env, &frames) else {
            return;
        };
        let Some(res) = with_mount(env, handle, |inner| {
            inner.push_audio_to(h, &buf, Pts90khz::new(pts))
        }) else {
            return;
        };
        if let Err(e) = res {
            mount_error_to_jvm(env, &e);
        }
    })
}

/// `MountHandle.nPushSubtitleTo(handle, streamHandleRaw, pts, payload)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nPushSubtitleTo<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    stream_handle_raw: jlong,
    pts: jlong,
    payload: JByteArray<'local>,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        let h = match SubtitleStreamHandle::try_from_raw(stream_handle_raw as u32) {
            Ok(h) => h,
            Err(_) => {
                throw_rtsp(env, "MOUNT", "invalid stream handle");
                return;
            }
        };
        let Some(buf) = read_bytes(env, &payload) else {
            return;
        };
        let Some(res) = with_mount(env, handle, |inner| {
            inner.push_subtitle_to(h, &buf, Pts90khz::new(pts))
        }) else {
            return;
        };
        if let Err(e) = res {
            mount_error_to_jvm(env, &e);
        }
    })
}

/// `MountHandle.nPushDataTo(handle, streamHandleRaw, data, pts)`. The raw handle
/// is validated via the strict `u32::try_from` + `DataStreamHandle::try_from_raw`
/// chain (rejecting negative / out-of-u32 values up front rather than
/// truncating). A forged handle is a `MOUNT` error (DIFFERS from MuxSender, which
/// maps forged handles to RtpException(TRANSPORT)).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nPushDataTo<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    stream_handle_raw: jlong,
    data: JByteArray<'local>,
    pts: jlong,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        let Some(h) = u32::try_from(stream_handle_raw)
            .ok()
            .and_then(|r| DataStreamHandle::try_from_raw(r).ok())
        else {
            throw_rtsp(env, "MOUNT", "invalid stream handle");
            return;
        };
        let Some(buf) = read_bytes(env, &data) else {
            return;
        };
        let Some(res) = with_mount(env, handle, |inner| {
            inner.push_data_to(h, &buf, Pts90khz::new(pts))
        }) else {
            return;
        };
        if let Err(e) = res {
            mount_error_to_jvm(env, &e);
        }
    })
}

// ── MountHandle: stream-handle accessors (first-of-kind; -1 = none) ──────────

/// `MountHandle.nVideoHandle(handle)` — first configured video stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nVideoHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        with_mount(env, handle, |inner| {
            inner
                .video_handles()
                .into_iter()
                .next()
                .map(|h| i64::from(h.raw()))
                .unwrap_or(-1)
        })
        .unwrap_or(-1)
    })
}

/// `MountHandle.nKlvHandle(handle)` — first configured KLV stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nKlvHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        with_mount(env, handle, |inner| {
            inner
                .klv_handles()
                .into_iter()
                .next()
                .map(|h| i64::from(h.raw()))
                .unwrap_or(-1)
        })
        .unwrap_or(-1)
    })
}

/// `MountHandle.nAudioHandle(handle)` — first configured audio stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nAudioHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        with_mount(env, handle, |inner| {
            inner
                .audio_handles()
                .into_iter()
                .next()
                .map(|h| i64::from(h.raw()))
                .unwrap_or(-1)
        })
        .unwrap_or(-1)
    })
}

/// `MountHandle.nSubtitleHandle(handle)` — first configured subtitle stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nSubtitleHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        with_mount(env, handle, |inner| {
            inner
                .subtitle_handles()
                .into_iter()
                .next()
                .map(|h| i64::from(h.raw()))
                .unwrap_or(-1)
        })
        .unwrap_or(-1)
    })
}

/// `MountHandle.nDataHandle(handle)` — first configured data stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nDataHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        with_mount(env, handle, |inner| {
            inner
                .data_handles()
                .into_iter()
                .next()
                .map(|h| i64::from(h.raw()))
                .unwrap_or(-1)
        })
        .unwrap_or(-1)
    })
}

// ── MountHandle: stream-handle accessors (all-of-kind; long[]) ──────────────

/// `MountHandle.nVideoHandles(handle)` → `long[]` of all video stream handles.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nVideoHandles<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JLongArray<'local> {
    crate::panic::jni_catch(&mut env, JObject::null().into(), |env| {
        let Some(raws) = with_mount(env, handle, |inner| {
            inner
                .video_handles()
                .into_iter()
                .map(|h| i64::from(h.raw()))
                .collect::<Vec<i64>>()
        }) else {
            return JObject::null().into();
        };
        let arr = match env.new_long_array(raws.len() as i32) {
            Ok(a) => a,
            Err(_) => return JObject::null().into(),
        };
        let _ = env.set_long_array_region(&arr, 0, &raws);
        arr
    })
}

/// `MountHandle.nKlvHandles(handle)` → `long[]` of all KLV stream handles.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nKlvHandles<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JLongArray<'local> {
    crate::panic::jni_catch(&mut env, JObject::null().into(), |env| {
        let Some(raws) = with_mount(env, handle, |inner| {
            inner
                .klv_handles()
                .into_iter()
                .map(|h| i64::from(h.raw()))
                .collect::<Vec<i64>>()
        }) else {
            return JObject::null().into();
        };
        let arr = match env.new_long_array(raws.len() as i32) {
            Ok(a) => a,
            Err(_) => return JObject::null().into(),
        };
        let _ = env.set_long_array_region(&arr, 0, &raws);
        arr
    })
}

/// `MountHandle.nAudioHandles(handle)` → `long[]` of all audio stream handles.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nAudioHandles<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JLongArray<'local> {
    crate::panic::jni_catch(&mut env, JObject::null().into(), |env| {
        let Some(raws) = with_mount(env, handle, |inner| {
            inner
                .audio_handles()
                .into_iter()
                .map(|h| i64::from(h.raw()))
                .collect::<Vec<i64>>()
        }) else {
            return JObject::null().into();
        };
        let arr = match env.new_long_array(raws.len() as i32) {
            Ok(a) => a,
            Err(_) => return JObject::null().into(),
        };
        let _ = env.set_long_array_region(&arr, 0, &raws);
        arr
    })
}

/// `MountHandle.nSubtitleHandles(handle)` → `long[]` of all subtitle stream handles.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nSubtitleHandles<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JLongArray<'local> {
    crate::panic::jni_catch(&mut env, JObject::null().into(), |env| {
        let Some(raws) = with_mount(env, handle, |inner| {
            inner
                .subtitle_handles()
                .into_iter()
                .map(|h| i64::from(h.raw()))
                .collect::<Vec<i64>>()
        }) else {
            return JObject::null().into();
        };
        let arr = match env.new_long_array(raws.len() as i32) {
            Ok(a) => a,
            Err(_) => return JObject::null().into(),
        };
        let _ = env.set_long_array_region(&arr, 0, &raws);
        arr
    })
}

/// `MountHandle.nDataHandles(handle)` → `long[]` of all data stream handles.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nDataHandles<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JLongArray<'local> {
    crate::panic::jni_catch(&mut env, JObject::null().into(), |env| {
        let Some(raws) = with_mount(env, handle, |inner| {
            inner
                .data_handles()
                .into_iter()
                .map(|h| i64::from(h.raw()))
                .collect::<Vec<i64>>()
        }) else {
            return JObject::null().into();
        };
        let arr = match env.new_long_array(raws.len() as i32) {
            Ok(a) => a,
            Err(_) => return JObject::null().into(),
        };
        let _ = env.set_long_array_region(&arr, 0, &raws);
        arr
    })
}

// ── MountHandle: lifecycle ──────────────────────────────────────────────────

/// `MountHandle.nFlush(handle)` — drain + broadcast buffered TS.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nFlush(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        with_mount(env, handle, |inner| inner.flush());
    })
}

/// `MountHandle.nResetStats(handle)` — zero all flow counters.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nResetStats(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        with_mount(env, handle, |inner| inner.reset_stats());
    })
}

/// `MountHandle.nClose(handle)` — free the handle-wrapper box (the mount itself
/// persists in the server until stop()/close()). No-op on a zero handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nClose(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    // Atomic + idempotent: only the winning close gets the handle wrapper back to
    // drop. No cancel hook — the mount persists in the server until stop()/close().
    crate::panic::jni_catch(&mut env, (), |_env| {
        let _ = REGISTRY_MOUNT.close(handle as u64);
    })
}
