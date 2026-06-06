//! `org.tstrans.rtp` RTSP SERVER JNI surface — `RtspServer`, `MountHandle`,
//! `RtspServerCancelHandle`. Ports tst-py's `bindings/python/src/rtp/server.rs`.
//! The underlying `tst_rtp::rtsp::server::RtspServer` owns a tokio Runtime inside
//! the native Box; there is no JNI-side async handling.

use std::time::Duration;

use jni::JNIEnv;
use jni::objects::{
    JBooleanArray, JByteArray, JClass, JIntArray, JLongArray, JObject, JString, JValue,
};
use jni::sys::{jboolean, jint, jlong};
use secrecy::SecretString;

use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{
    AudioStreamHandle, KlvStreamHandle, SubtitleStreamHandle, VideoStreamHandle,
};
use tst_rtp::RtspServer as RustRtspServer;
use tst_rtp::RtspServerCancelHandle as RustServerCancel;
use tst_rtp::ServerStats as RustServerStats;
use tst_rtp::builder::RtspServerBuilder;
use tst_rtp::error::RtspServerError;
use tst_rtp::rtsp::server::mount::{MountHandle as RustMountHandle, MountKind};

use crate::jutil::checked_u8;
use crate::mpegts::muxer::build_muxer_config_from_arrays;

use super::errors::{mount_error_to_jvm, server_error_to_jvm, throw_rtsp};

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

// ---------------------------------------------------------------------------
// Mount factories (on RtspServer) + MountHandle push surface.
// ---------------------------------------------------------------------------

type MountInner = RustMountHandle;

fn checked_mount(env: &mut JNIEnv, handle: jlong) -> Option<*const MountInner> {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "MountHandle is closed");
        return None;
    }
    Some(handle as *const MountInner)
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
    klv_stream_types: JIntArray<'local>,
    klv_carries_pts: JBooleanArray<'local>,
) -> jlong {
    let Some(ptr) = checked_server(&mut env, server_handle) else {
        return 0;
    };
    let cfg = match build_muxer_config_from_arrays(
        &mut env,
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
        &klv_stream_types,
        &klv_carries_pts,
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
    // SAFETY: validated non-zero live Box<RtspServer>; add_mount takes &self.
    let server = unsafe { &*ptr };
    match server.add_mount(&path_str, cfg) {
        Ok(mh) => Box::into_raw(Box::new(mh)) as jlong,
        Err(e) => {
            server_error_to_jvm(&mut env, &e);
            0
        }
    }
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
    klv_stream_types: JIntArray<'local>,
    klv_carries_pts: JBooleanArray<'local>,
) -> jlong {
    let Some(ptr) = checked_server(&mut env, server_handle) else {
        return 0;
    };
    let cfg = match build_muxer_config_from_arrays(
        &mut env,
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
        &klv_stream_types,
        &klv_carries_pts,
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
    // SAFETY: validated non-zero live Box<RtspServer>; add_multicast_mount takes &self.
    let server = unsafe { &*ptr };
    match server.add_multicast_mount(&path_str, cfg, &url) {
        Ok(mh) => Box::into_raw(Box::new(mh)) as jlong,
        Err(e) => {
            server_error_to_jvm(&mut env, &e);
            0
        }
    }
}

// ── MountHandle: identity / introspection ──────────────────────────────────

/// `MountHandle.nMountPath(handle)` → registered mount path string.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nMountPath<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JString<'local> {
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return JObject::null().into();
    };
    // SAFETY: validated non-zero live Box<MountHandle>; mount_path takes &self.
    let inner = unsafe { &*ptr };
    env.new_string(inner.mount_path())
        .unwrap_or_else(|_| JObject::null().into())
}

/// `MountHandle.nPeerCount(handle)` → live subscriber count.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nPeerCount(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return 0;
    };
    // SAFETY: validated non-zero live Box<MountHandle>; peer_count takes &self.
    let inner = unsafe { &*ptr };
    inner.peer_count() as jlong
}

/// `MountHandle.nMountKind(handle)` → "unicast" / "multicast" / "unknown".
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nMountKind<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JString<'local> {
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return JObject::null().into();
    };
    // SAFETY: validated non-zero live Box<MountHandle>; mount_kind takes &self.
    let inner = unsafe { &*ptr };
    let kind = match inner.mount_kind() {
        MountKind::Unicast => "unicast",
        MountKind::Multicast { .. } => "multicast",
        _ => "unknown",
    };
    env.new_string(kind)
        .unwrap_or_else(|_| JObject::null().into())
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
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return JObject::null();
    };
    // SAFETY: validated non-zero live Box<MountHandle>; stats takes &self.
    let inner = unsafe { &*ptr };
    let s = inner.stats();
    build_mount_stats(&mut env, &s).unwrap_or_else(|_| JObject::null())
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
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live Box<MountHandle>; push_video takes &self, and
    // RustMountHandle is Arc-shared so concurrent &*ptr pushes are sound (no &mut).
    let inner = unsafe { &*ptr };
    let Some(buf) = read_bytes(&mut env, &nal) else {
        return;
    };
    if let Err(e) = inner.push_video(&buf, Pts90khz::new(pts), key_frame != 0) {
        mount_error_to_jvm(&mut env, &e);
    }
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
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return;
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    let Ok(service_id) = checked_u8(
        &mut env,
        i64::from(metadata_service_id),
        "metadataServiceId",
    ) else {
        return; // IllegalArgumentException pending
    };
    let Some(buf) = read_bytes(&mut env, &klv) else {
        return;
    };
    if let Err(e) = inner.push_klv(&buf, Pts90khz::new(pts), service_id) {
        mount_error_to_jvm(&mut env, &e);
    }
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
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return;
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    let Some(buf) = read_bytes(&mut env, &frames) else {
        return;
    };
    if let Err(e) = inner.push_audio(&buf, Pts90khz::new(pts)) {
        mount_error_to_jvm(&mut env, &e);
    }
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
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return;
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    let Some(buf) = read_bytes(&mut env, &payload) else {
        return;
    };
    if let Err(e) = inner.push_subtitle(&buf, Pts90khz::new(pts)) {
        mount_error_to_jvm(&mut env, &e);
    }
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
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return;
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    let h = match VideoStreamHandle::try_from_raw(stream_handle_raw as u32) {
        Ok(h) => h,
        Err(_) => {
            // MountHandle has no transport concept — a forged handle is a MOUNT error
            // (DIFFERS from MuxSender, which maps forged handles to RtpException(TRANSPORT)).
            throw_rtsp(&mut env, "MOUNT", "invalid stream handle");
            return;
        }
    };
    let Some(buf) = read_bytes(&mut env, &nal) else {
        return;
    };
    if let Err(e) = inner.push_video_to(h, &buf, Pts90khz::new(pts), key_frame != 0) {
        mount_error_to_jvm(&mut env, &e);
    }
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
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return;
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    let h = match KlvStreamHandle::try_from_raw(stream_handle_raw as u32) {
        Ok(h) => h,
        Err(_) => {
            throw_rtsp(&mut env, "MOUNT", "invalid stream handle");
            return;
        }
    };
    let Ok(service_id) = checked_u8(
        &mut env,
        i64::from(metadata_service_id),
        "metadataServiceId",
    ) else {
        return;
    };
    let Some(buf) = read_bytes(&mut env, &klv) else {
        return;
    };
    if let Err(e) = inner.push_klv_to(h, &buf, Pts90khz::new(pts), service_id) {
        mount_error_to_jvm(&mut env, &e);
    }
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
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return;
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    let h = match AudioStreamHandle::try_from_raw(stream_handle_raw as u32) {
        Ok(h) => h,
        Err(_) => {
            throw_rtsp(&mut env, "MOUNT", "invalid stream handle");
            return;
        }
    };
    let Some(buf) = read_bytes(&mut env, &frames) else {
        return;
    };
    if let Err(e) = inner.push_audio_to(h, &buf, Pts90khz::new(pts)) {
        mount_error_to_jvm(&mut env, &e);
    }
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
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return;
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    let h = match SubtitleStreamHandle::try_from_raw(stream_handle_raw as u32) {
        Ok(h) => h,
        Err(_) => {
            throw_rtsp(&mut env, "MOUNT", "invalid stream handle");
            return;
        }
    };
    let Some(buf) = read_bytes(&mut env, &payload) else {
        return;
    };
    if let Err(e) = inner.push_subtitle_to(h, &buf, Pts90khz::new(pts)) {
        mount_error_to_jvm(&mut env, &e);
    }
}

// ── MountHandle: stream-handle accessors (first-of-kind; -1 = none) ──────────

/// `MountHandle.nVideoHandle(handle)` — first configured video stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nVideoHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return -1;
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    inner
        .video_handles()
        .into_iter()
        .next()
        .map(|h| i64::from(h.raw()))
        .unwrap_or(-1)
}

/// `MountHandle.nKlvHandle(handle)` — first configured KLV stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nKlvHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return -1;
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    inner
        .klv_handles()
        .into_iter()
        .next()
        .map(|h| i64::from(h.raw()))
        .unwrap_or(-1)
}

/// `MountHandle.nAudioHandle(handle)` — first configured audio stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nAudioHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return -1;
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    inner
        .audio_handles()
        .into_iter()
        .next()
        .map(|h| i64::from(h.raw()))
        .unwrap_or(-1)
}

/// `MountHandle.nSubtitleHandle(handle)` — first configured subtitle stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nSubtitleHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return -1;
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    inner
        .subtitle_handles()
        .into_iter()
        .next()
        .map(|h| i64::from(h.raw()))
        .unwrap_or(-1)
}

// ── MountHandle: stream-handle accessors (all-of-kind; long[]) ──────────────

/// `MountHandle.nVideoHandles(handle)` → `long[]` of all video stream handles.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nVideoHandles<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JLongArray<'local> {
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return JObject::null().into();
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    let raws: Vec<i64> = inner
        .video_handles()
        .into_iter()
        .map(|h| i64::from(h.raw()))
        .collect();
    let arr = match env.new_long_array(raws.len() as i32) {
        Ok(a) => a,
        Err(_) => return JObject::null().into(),
    };
    let _ = env.set_long_array_region(&arr, 0, &raws);
    arr
}

/// `MountHandle.nKlvHandles(handle)` → `long[]` of all KLV stream handles.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nKlvHandles<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JLongArray<'local> {
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return JObject::null().into();
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    let raws: Vec<i64> = inner
        .klv_handles()
        .into_iter()
        .map(|h| i64::from(h.raw()))
        .collect();
    let arr = match env.new_long_array(raws.len() as i32) {
        Ok(a) => a,
        Err(_) => return JObject::null().into(),
    };
    let _ = env.set_long_array_region(&arr, 0, &raws);
    arr
}

/// `MountHandle.nAudioHandles(handle)` → `long[]` of all audio stream handles.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nAudioHandles<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JLongArray<'local> {
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return JObject::null().into();
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    let raws: Vec<i64> = inner
        .audio_handles()
        .into_iter()
        .map(|h| i64::from(h.raw()))
        .collect();
    let arr = match env.new_long_array(raws.len() as i32) {
        Ok(a) => a,
        Err(_) => return JObject::null().into(),
    };
    let _ = env.set_long_array_region(&arr, 0, &raws);
    arr
}

/// `MountHandle.nSubtitleHandles(handle)` → `long[]` of all subtitle stream handles.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nSubtitleHandles<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JLongArray<'local> {
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return JObject::null().into();
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    let raws: Vec<i64> = inner
        .subtitle_handles()
        .into_iter()
        .map(|h| i64::from(h.raw()))
        .collect();
    let arr = match env.new_long_array(raws.len() as i32) {
        Ok(a) => a,
        Err(_) => return JObject::null().into(),
    };
    let _ = env.set_long_array_region(&arr, 0, &raws);
    arr
}

// ── MountHandle: lifecycle ──────────────────────────────────────────────────

/// `MountHandle.nFlush(handle)` — drain + broadcast buffered TS.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nFlush(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return;
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    inner.flush();
}

/// `MountHandle.nResetStats(handle)` — zero all flow counters.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nResetStats(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    let Some(ptr) = checked_mount(&mut env, handle) else {
        return;
    };
    // SAFETY: as nPushVideo.
    let inner = unsafe { &*ptr };
    inner.reset_stats();
}

/// `MountHandle.nClose(handle)` — free the handle-wrapper box (the mount itself
/// persists in the server until stop()/close()). No-op on a zero handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MountHandle_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: handle from Box::into_raw, dropped once (Java zeroes its field).
        drop(unsafe { Box::from_raw(handle as *mut MountInner) });
    }
}
