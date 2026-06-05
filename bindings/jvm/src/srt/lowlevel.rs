//! JNI exports for `org.tstrans.srt.Builder`, `org.tstrans.srt.Socket`, and
//! `org.tstrans.srt.Listener` — the low-level SRT primitives.
//!
//! Handle lifecycle:
//! - `nConnect` / `nListen` allocate via `Box::into_raw`.
//! - Non-consuming methods reconstitute as `&mut *ptr`.
//! - `nIntoSender` / `nIntoReceiver` CONSUME the `Box<Socket>` via `Box::from_raw`
//!   and return a new `Box<Sender>` / `Box<Receiver>` handle. The Java caller
//!   zeros its own socket handle on success — the `handle = 0` assignment is the
//!   next statement after the native call.
//! - `nClose` deallocates via `Box::from_raw`.
//!
//! Error mapping keeps each KIND literal on the same line as `throw_srt` so the
//! T4 grep ratchet can verify coverage.

#![allow(clippy::too_many_arguments)]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::{jlong, jobject};
use tst_core::transport::TransportCancel;
use tst_pipeline::{Receiver as PlReceiver, ReceiverConfig, Sender as PlSender, SenderConfig};
use tst_srt::{
    Listener as SrtListener, ListenerConfig, Socket as SrtSocket, SocketConfig, SrtTransport,
    SrtUrl,
    options::{Congestion, MaxBandwidth, Passphrase, StreamId},
    url::Mode,
};

use super::JniCancel;
use super::errors::{accept_error, bind_error, connect_error, io_error, throw_srt, url_error};
use crate::jutil::checked_u16;

// ---------------------------------------------------------------------------
// LowLevelSrtCancel adapter
// ---------------------------------------------------------------------------
//
// `Listener::cancel_handle()` returns a concrete `tst_core::SrtCancelHandle`,
// not an `Arc<dyn TransportCancel>`. We need a thin adapter to fit it into
// `JniCancel::inner: Arc<dyn TransportCancel + Send + Sync>`. This mirrors
// tst-py's `LowLevelSrtCancel` in `bindings/python/src/srt/transport.rs`.

struct LowLevelSrtCancel(tst_core::SrtCancelHandle);

impl TransportCancel for LowLevelSrtCancel {
    fn cancel(&self) {
        self.0.cancel();
    }
}

// ---------------------------------------------------------------------------
// Helper: build a HostPort JObject from a std::net::SocketAddr
// ---------------------------------------------------------------------------

fn build_host_port<'local>(
    env: &mut JNIEnv<'local>,
    addr: std::net::SocketAddr,
) -> jni::errors::Result<JObject<'local>> {
    // Ensure capacity for: HostPort ctor arg (String) + local refs.
    env.ensure_local_capacity(8)?;
    let host_str = addr.ip().to_string();
    let host = env.new_string(&host_str)?;
    env.new_object(
        "org/tstrans/srt/HostPort",
        "(Ljava/lang/String;I)V",
        &[JValue::Object(&host), JValue::Int(addr.port() as i32)],
    )
}

// ---------------------------------------------------------------------------
// Helper: unbox nullable Integer / Long JObject args
// ---------------------------------------------------------------------------

/// Unbox a nullable `java.lang.Integer` JNI argument. Returns `None` for null.
fn unbox_nullable_int(env: &mut JNIEnv, obj: &JObject) -> jni::errors::Result<Option<i32>> {
    if obj.is_null() {
        return Ok(None);
    }
    let v = env.call_method(obj, "intValue", "()I", &[])?.i()?;
    Ok(Some(v))
}

/// Unbox a nullable `java.lang.Long` JNI argument. Returns `None` for null.
fn unbox_nullable_long(env: &mut JNIEnv, obj: &JObject) -> jni::errors::Result<Option<i64>> {
    if obj.is_null() {
        return Ok(None);
    }
    let v = env.call_method(obj, "longValue", "()J", &[])?.j()?;
    Ok(Some(v))
}

// ---------------------------------------------------------------------------
// Helper: join host:port, bracketing bare IPv6 literals
// ---------------------------------------------------------------------------

fn join_host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

// ---------------------------------------------------------------------------
// Helper: build SocketConfig from knob args
// ---------------------------------------------------------------------------
//
// Mode ordinals (the Java Builder.Mode enum, passed as mode.ordinal()):
//   URL_CHOICE=0, CALLER=1, LISTENER=2, RENDEZVOUS=3.
// nConnect rejects LISTENER(2) and RENDEZVOUS(3); nListen rejects CALLER(1)
// and RENDEZVOUS(3). URL_CHOICE(0) defers to the URL's ?mode= parameter.

/// Apply the nullable knob args into a `SocketConfig`. Returns `Err` if any
/// validated knob (passphrase, stream_id, congestion, mss, payload_size) fails.
/// Does NOT apply the URL overlay — the caller does that after this returns.
#[allow(clippy::cast_sign_loss)]
fn build_socket_config(
    env: &mut JNIEnv,
    latency_ms: &JObject,
    passphrase: &JString,
    stream_id: &JString,
    congestion: &JString,
    connect_timeout_ms: &JObject,
    recv_timeout_ms: &JObject,
    send_timeout_ms: &JObject,
    peer_latency_ms: &JObject,
    recv_latency_ms: &JObject,
    max_bandwidth_bps: &JObject,
    mss: &JObject,
    payload_size: &JObject,
) -> jni::errors::Result<SocketConfig> {
    let mut cfg = SocketConfig::default();

    // latency_ms — applied to both SRTO_LATENCY on socket configs
    if let Some(ms) = unbox_nullable_int(env, latency_ms)? {
        cfg.latency = Some(Duration::from_millis(ms as u64));
    }

    // passphrase
    if !passphrase.is_null() {
        let s: String = env.get_string(passphrase).map(Into::into)?;
        match Passphrase::new(s) {
            Ok(pp) => cfg.passphrase = Some(pp),
            Err(e) => {
                throw_srt(env, "CONFIG_INVALID", &e.to_string());
                return Err(jni::errors::Error::JavaException);
            }
        }
    }

    // stream_id (caller-side only; not in ListenerConfig)
    if !stream_id.is_null() {
        let s: String = env.get_string(stream_id).map(Into::into)?;
        match StreamId::new(s) {
            Ok(id) => cfg.stream_id = Some(id),
            Err(e) => {
                throw_srt(env, "CONFIG_INVALID", &e.to_string());
                return Err(jni::errors::Error::JavaException);
            }
        }
    }

    // congestion
    if !congestion.is_null() {
        let s: String = env.get_string(congestion).map(Into::into)?;
        match Congestion::from_str_strict(&s) {
            Ok(c) => cfg.congestion = Some(c),
            Err(e) => {
                throw_srt(env, "CONFIG_INVALID", &e.to_string());
                return Err(jni::errors::Error::JavaException);
            }
        }
    }

    // connect_timeout_ms
    if let Some(ms) = unbox_nullable_int(env, connect_timeout_ms)? {
        cfg.connect_timeout = Some(Duration::from_millis(ms as u64));
    }

    // recv_timeout_ms
    if let Some(ms) = unbox_nullable_int(env, recv_timeout_ms)? {
        cfg.recv_timeout = Some(Duration::from_millis(ms as u64));
    }

    // send_timeout_ms
    if let Some(ms) = unbox_nullable_int(env, send_timeout_ms)? {
        cfg.send_timeout = Some(Duration::from_millis(ms as u64));
    }

    // peer_latency_ms (caller-only; not in ListenerConfig)
    if let Some(ms) = unbox_nullable_int(env, peer_latency_ms)? {
        cfg.peer_latency = Some(Duration::from_millis(ms as u64));
    }

    // recv_latency_ms
    if let Some(ms) = unbox_nullable_int(env, recv_latency_ms)? {
        cfg.recv_latency = Some(Duration::from_millis(ms as u64));
    }

    // max_bandwidth_bps
    if let Some(bps) = unbox_nullable_long(env, max_bandwidth_bps)? {
        cfg.max_bandwidth = Some(MaxBandwidth::Limited(bps as u64));
    }

    // mss — u16; checked_u16 throws IllegalArgumentException on overflow
    if let Some(v) = unbox_nullable_int(env, mss)? {
        cfg.mss = Some(checked_u16(env, v as i64, "mss")?);
    }

    // payload_size — u16
    if let Some(v) = unbox_nullable_int(env, payload_size)? {
        cfg.payload_size = Some(checked_u16(env, v as i64, "payloadSize")?);
    }

    Ok(cfg)
}

/// Apply the nullable knob args into a `ListenerConfig`. Shares most knobs with
/// `build_socket_config` but omits `stream_id`, `peer_latency_ms`, and
/// `connect_timeout_ms` (not present on `ListenerConfig`).
#[allow(clippy::cast_sign_loss)]
fn build_listener_config(
    env: &mut JNIEnv,
    latency_ms: &JObject,
    passphrase: &JString,
    congestion: &JString,
    recv_timeout_ms: &JObject,
    send_timeout_ms: &JObject,
    recv_latency_ms: &JObject,
    max_bandwidth_bps: &JObject,
    mss: &JObject,
    payload_size: &JObject,
) -> jni::errors::Result<ListenerConfig> {
    let mut cfg = ListenerConfig::default();

    if let Some(ms) = unbox_nullable_int(env, latency_ms)? {
        cfg.latency = Some(Duration::from_millis(ms as u64));
    }

    if !passphrase.is_null() {
        let s: String = env.get_string(passphrase).map(Into::into)?;
        match Passphrase::new(s) {
            Ok(pp) => cfg.passphrase = Some(pp),
            Err(e) => {
                throw_srt(env, "CONFIG_INVALID", &e.to_string());
                return Err(jni::errors::Error::JavaException);
            }
        }
    }

    if !congestion.is_null() {
        let s: String = env.get_string(congestion).map(Into::into)?;
        match Congestion::from_str_strict(&s) {
            Ok(c) => cfg.congestion = Some(c),
            Err(e) => {
                throw_srt(env, "CONFIG_INVALID", &e.to_string());
                return Err(jni::errors::Error::JavaException);
            }
        }
    }

    if let Some(ms) = unbox_nullable_int(env, recv_timeout_ms)? {
        cfg.recv_timeout = Some(Duration::from_millis(ms as u64));
    }

    if let Some(ms) = unbox_nullable_int(env, send_timeout_ms)? {
        cfg.send_timeout = Some(Duration::from_millis(ms as u64));
    }

    if let Some(ms) = unbox_nullable_int(env, recv_latency_ms)? {
        cfg.recv_latency = Some(Duration::from_millis(ms as u64));
    }

    if let Some(bps) = unbox_nullable_long(env, max_bandwidth_bps)? {
        cfg.max_bandwidth = Some(MaxBandwidth::Limited(bps as u64));
    }

    if let Some(v) = unbox_nullable_int(env, mss)? {
        cfg.mss = Some(checked_u16(env, v as i64, "mss")?);
    }

    if let Some(v) = unbox_nullable_int(env, payload_size)? {
        cfg.payload_size = Some(checked_u16(env, v as i64, "payloadSize")?);
    }

    Ok(cfg)
}

// ---------------------------------------------------------------------------
// Builder — nConnect
// ---------------------------------------------------------------------------

/// Open a caller-mode SRT socket. Returns `Box<Socket>` as `jlong`.
///
/// Mode ordinal mapping (Builder.Mode enum):
/// - 0 = URL_CHOICE: defer to the URL's ?mode= (default is caller)
/// - 1 = CALLER: assert URL mode must be caller
/// - 2 = LISTENER: reject from nConnect (wrong finalizer)
/// - 3 = RENDEZVOUS: reject (not supported)
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Builder_nConnect<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    url: JString<'local>,
    mode: jni::sys::jint,
    latency_ms: JObject<'local>,
    passphrase: JString<'local>,
    stream_id: JString<'local>,
    congestion: JString<'local>,
    connect_timeout_ms: JObject<'local>,
    recv_timeout_ms: JObject<'local>,
    send_timeout_ms: JObject<'local>,
    peer_latency_ms: JObject<'local>,
    recv_latency_ms: JObject<'local>,
    max_bandwidth_bps: JObject<'local>,
    mss: JObject<'local>,
    payload_size: JObject<'local>,
) -> jlong {
    // Ordinal 3 = RENDEZVOUS, ordinal 2 = LISTENER (wrong for connect).
    if mode == 3 {
        throw_srt(
            &mut env,
            "CONFIG_INVALID",
            "rendezvous mode is not yet supported by tst-srt",
        );
        return 0;
    }
    if mode == 2 {
        throw_srt(
            &mut env,
            "CONFIG_INVALID",
            "Builder.connect() requires caller mode (mode is LISTENER)",
        );
        return 0;
    }

    let url_str: String = match env.get_string(&url) {
        Ok(s) => s.into(),
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return 0;
        }
    };

    // knob overflow checks happen BEFORE the network call (checked_u16 in build_socket_config)
    let mut cfg = match build_socket_config(
        &mut env,
        &latency_ms,
        &passphrase,
        &stream_id,
        &congestion,
        &connect_timeout_ms,
        &recv_timeout_ms,
        &send_timeout_ms,
        &peer_latency_ms,
        &recv_latency_ms,
        &max_bandwidth_bps,
        &mss,
        &payload_size,
    ) {
        Ok(c) => c,
        Err(_) => return 0, // exception already thrown (IllegalArgumentException or SrtException)
    };

    let parsed = match SrtUrl::parse(&url_str) {
        Ok(p) => p,
        Err(e) => {
            url_error(&mut env, &e);
            return 0;
        }
    };

    // Validate that the URL mode is caller (or default) — mode=1 (CALLER) asserts this.
    // mode=0 (URL_CHOICE) also requires URL to be caller-default.
    if parsed.mode != Mode::Caller {
        let msg = format!(
            "Builder.connect() requires URL mode=caller (default); got mode={:?}",
            parsed.mode
        );
        throw_srt(&mut env, "CONFIG_INVALID", &msg);
        return 0;
    }

    // URL overlay AFTER kwargs — URL wins on conflict (Q4-A).
    parsed.overlay.apply_to_socket(&mut cfg);

    let addr = join_host_port(&parsed.host, parsed.port);
    let socket = match SrtSocket::connect_with(&cfg, addr.as_str()) {
        Ok(s) => s,
        Err(e) => {
            connect_error(&mut env, &e);
            return 0;
        }
    };

    Box::into_raw(Box::new(socket)) as jlong
}

// ---------------------------------------------------------------------------
// Builder — nListen
// ---------------------------------------------------------------------------

/// Bind a listener-mode SRT socket. Returns `Box<Listener>` as `jlong`.
///
/// Mode ordinal mapping (same as nConnect):
/// - 0 = URL_CHOICE: defer to URL (must have ?mode=listener)
/// - 1 = CALLER: reject from nListen (wrong finalizer)
/// - 2 = LISTENER: assert URL mode must be listener
/// - 3 = RENDEZVOUS: reject (not supported)
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Builder_nListen<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    url: JString<'local>,
    mode: jni::sys::jint,
    latency_ms: JObject<'local>,
    passphrase: JString<'local>,
    stream_id: JString<'local>, // accepted but ignored for listener (no ListenerConfig.stream_id)
    congestion: JString<'local>,
    connect_timeout_ms: JObject<'local>, // accepted but ignored (no ListenerConfig.connect_timeout)
    recv_timeout_ms: JObject<'local>,
    send_timeout_ms: JObject<'local>,
    peer_latency_ms: JObject<'local>, // accepted but ignored (no ListenerConfig.peer_latency)
    recv_latency_ms: JObject<'local>,
    max_bandwidth_bps: JObject<'local>,
    mss: JObject<'local>,
    payload_size: JObject<'local>,
) -> jlong {
    if mode == 3 {
        throw_srt(
            &mut env,
            "CONFIG_INVALID",
            "rendezvous mode is not yet supported by tst-srt",
        );
        return 0;
    }
    if mode == 1 {
        throw_srt(
            &mut env,
            "CONFIG_INVALID",
            "Builder.listen() requires listener mode (mode is CALLER)",
        );
        return 0;
    }

    let url_str: String = match env.get_string(&url) {
        Ok(s) => s.into(),
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return 0;
        }
    };

    // knob overflow checks happen BEFORE the network call
    let mut cfg = match build_listener_config(
        &mut env,
        &latency_ms,
        &passphrase,
        &congestion,
        &recv_timeout_ms,
        &send_timeout_ms,
        &recv_latency_ms,
        &max_bandwidth_bps,
        &mss,
        &payload_size,
    ) {
        Ok(c) => c,
        Err(_) => return 0,
    };

    // Silently ignore: stream_id (no ListenerConfig field), connect_timeout_ms
    // (no ListenerConfig field), peer_latency_ms (no ListenerConfig field).
    // This matches tst-py: apply_stream_id goes to socket_cfg only; peer_latency
    // goes to socket_cfg only; connect_timeout goes to socket_cfg only.
    let _ = (&stream_id, &connect_timeout_ms, &peer_latency_ms); // suppress unused warnings

    let parsed = match SrtUrl::parse(&url_str) {
        Ok(p) => p,
        Err(e) => {
            url_error(&mut env, &e);
            return 0;
        }
    };

    if parsed.mode != Mode::Listener {
        let msg = format!(
            "Builder.listen() requires URL ?mode=listener; got mode={:?}",
            parsed.mode
        );
        throw_srt(&mut env, "CONFIG_INVALID", &msg);
        return 0;
    }

    // URL overlay AFTER kwargs.
    parsed.overlay.apply_to_listener(&mut cfg);

    let addr = if parsed.host.is_empty() {
        format!("0.0.0.0:{}", parsed.port)
    } else {
        join_host_port(&parsed.host, parsed.port)
    };

    let listener = match SrtListener::bind_with(&cfg, addr.as_str()) {
        Ok(l) => l,
        Err(e) => {
            bind_error(&mut env, &e);
            return 0;
        }
    };

    Box::into_raw(Box::new(listener)) as jlong
}

// ---------------------------------------------------------------------------
// Socket — nIntoSender
// ---------------------------------------------------------------------------

/// Consume a `Box<Socket>` and produce a `Box<PlSender<SrtTransport>>`.
/// The Java caller MUST zero its socket handle field after this returns.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Socket_nIntoSender(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Socket is closed");
        return 0;
    }
    // SAFETY: handle is a valid Box<SrtSocket> allocated by nConnect or nAccept.
    // We consume it here (Box::from_raw + dereference) — the Java caller must
    // zero its own field so no double-free occurs.
    let socket: SrtSocket = *unsafe { Box::from_raw(handle as *mut SrtSocket) };
    let transport = SrtTransport::new(socket);
    let sender = PlSender::new(transport, SenderConfig::default());
    Box::into_raw(Box::new(sender)) as jlong
}

// ---------------------------------------------------------------------------
// Socket — nIntoReceiver
// ---------------------------------------------------------------------------

/// Consume a `Box<Socket>` and produce a `Box<PlReceiver<SrtTransport>>`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Socket_nIntoReceiver(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Socket is closed");
        return 0;
    }
    // SAFETY: handle is a valid Box<SrtSocket>; consumed once here.
    let socket: SrtSocket = *unsafe { Box::from_raw(handle as *mut SrtSocket) };
    let transport = SrtTransport::new(socket);
    let receiver = PlReceiver::new(transport, ReceiverConfig::default());
    Box::into_raw(Box::new(receiver)) as jlong
}

// ---------------------------------------------------------------------------
// Socket — nLocalAddr / nPeerAddr / nStreamId / nClose
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Socket_nLocalAddr<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Socket is closed");
        return std::ptr::null_mut();
    }
    // SAFETY: handle is a valid Box<SrtSocket>; reconstituted as &SrtSocket (non-consuming).
    let socket: &SrtSocket = unsafe { &*(handle as *const SrtSocket) };
    match socket.local_addr() {
        Ok(addr) => match build_host_port(&mut env, addr) {
            Ok(obj) => obj.into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
        Err(e) => {
            io_error(&mut env, &e);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Socket_nPeerAddr<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Socket is closed");
        return std::ptr::null_mut();
    }
    // SAFETY: handle is a valid Box<SrtSocket>; reconstituted as &SrtSocket (non-consuming).
    let socket: &SrtSocket = unsafe { &*(handle as *const SrtSocket) };
    match socket.peer_addr() {
        Ok(addr) => match build_host_port(&mut env, addr) {
            Ok(obj) => obj.into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
        Err(e) => {
            io_error(&mut env, &e);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Socket_nStreamId<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    if handle == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: handle is a valid Box<SrtSocket>.
    let socket: &SrtSocket = unsafe { &*(handle as *const SrtSocket) };
    match socket.stream_id() {
        Some(id) => match env.new_string(id) {
            Ok(s) => s.into_raw() as jobject,
            Err(_) => std::ptr::null_mut(),
        },
        None => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Socket_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: handle is a valid Box<SrtSocket>; close() is called at most
        // once (Java's close() zeroes the field; consumed-handles also zero it).
        let socket: SrtSocket = *unsafe { Box::from_raw(handle as *mut SrtSocket) };
        let _ = socket.close();
    }
}

// ---------------------------------------------------------------------------
// Listener — nAccept
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Listener_nAccept(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    timeout_ms: jlong,
) -> jlong {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Listener is closed");
        return 0;
    }
    // SAFETY: handle is a valid Box<SrtListener>; reconstituted as &mut SrtListener.
    let listener: &mut SrtListener = unsafe { &mut *(handle as *mut SrtListener) };
    let result = if timeout_ms < 0 {
        listener.accept()
    } else {
        listener.accept_timeout(Duration::from_millis(timeout_ms as u64))
    };
    match result {
        Ok((socket, _peer)) => Box::into_raw(Box::new(socket)) as jlong,
        Err(e) => {
            accept_error(&mut env, &e);
            0
        }
    }
}

// ---------------------------------------------------------------------------
// Listener — nCancelHandle
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Listener_nCancelHandle(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    if handle == 0 {
        return 0;
    }
    // SAFETY: handle is a valid Box<SrtListener>.
    let listener: &SrtListener = unsafe { &*(handle as *const SrtListener) };
    // Wrap the concrete SrtCancelHandle in a LowLevelSrtCancel adapter so it
    // fits the JniCancel::inner: Arc<dyn TransportCancel + Send + Sync> slot.
    // This mirrors tst-py's PyCancelHandle::from_concrete (transport.rs).
    let adapter: Arc<dyn TransportCancel + Send + Sync> =
        Arc::new(LowLevelSrtCancel(listener.cancel_handle()));
    JniCancel {
        inner: adapter,
        flag: AtomicBool::new(false),
    }
    .into_handle()
}

// ---------------------------------------------------------------------------
// Listener — nLocalAddr / nClose
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Listener_nLocalAddr<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Listener is closed");
        return std::ptr::null_mut();
    }
    // SAFETY: handle is a valid Box<SrtListener>.
    let listener: &SrtListener = unsafe { &*(handle as *const SrtListener) };
    match listener.local_addr() {
        Ok(addr) => match build_host_port(&mut env, addr) {
            Ok(obj) => obj.into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
        Err(e) => {
            io_error(&mut env, &e);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Listener_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: handle is a valid Box<SrtListener>; close() is called at most
        // once (Java's close() zeroes the field).
        let listener: SrtListener = *unsafe { Box::from_raw(handle as *mut SrtListener) };
        let _ = listener.close();
    }
}
