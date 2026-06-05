//! JNI exports for `org.tstrans.srt.Sender` and `org.tstrans.srt.Receiver`.
//!
//! Each export backs one static-native method on the Java class. The handle is
//! a `jlong` storing a heap-allocated `Box<tst_pipeline::Sender<SrtTransport>>`
//! or `Box<tst_pipeline::Receiver<SrtTransport>>`. Handle lifecycle:
//! - `nFromUrl` allocates via `Box::into_raw`.
//! - Per-call methods reconstitute as `&mut *ptr` (non-consuming).
//! - `nClose` deallocates via `Box::from_raw`.
//!
//! The Java side guards all per-call methods with `ensureOpen()` and always
//! passes a non-zero handle to Rust, but zero-handle checks are retained here
//! as a safety net.
//!
//! There is no GIL analog in JNI — calls simply block on the native thread.
//! Callers that need cancellation call `nCancelHandle` and invoke `.cancel()`
//! from another thread; that wakes the libsrt socket within ~3-10 ms.

use std::sync::atomic::AtomicBool;

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JObject, JString};
use jni::sys::{jboolean, jbyteArray, jlong};
use tst_pipeline::receiver::ReceiverErrorSource;
use tst_pipeline::sender::SenderErrorSource;
use tst_pipeline::{Receiver as PlReceiver, ReceiverConfig, Sender as PlSender, SenderConfig};
use tst_srt::{Listener, ListenerConfig, Socket, SocketConfig, SrtTransport, SrtUrl, url::Mode};

use super::JniCancel;
use super::errors::{
    accept_error, bind_error, connect_error, io_error, transport_error, url_error,
};
use super::stats::{build_socket_stats, build_srt_stats};

// -----------------------------------------------------------------------
// Sender  (org.tstrans.srt.Sender)
// -----------------------------------------------------------------------

/// Allocate a `Sender` from an SRT caller-mode URL. Returns a `jlong` handle
/// on success; throws `SrtException` and returns 0 on any error.
///
/// The URL must use `mode=caller` (the default when omitted). The host is
/// bracketed for bare IPv6 literals, matching tst-py's IPv6 bracketing logic.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Sender_nFromUrl(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    url: JString<'_>,
) -> jlong {
    let url_str: String = match env.get_string(&url) {
        Ok(s) => s.into(),
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return 0;
        }
    };

    let parsed = match SrtUrl::parse(&url_str) {
        Ok(p) => p,
        Err(e) => {
            url_error(&mut env, &e);
            return 0;
        }
    };

    if parsed.mode != Mode::Caller {
        let msg = format!(
            "Sender.fromUrl requires mode=caller (default); got mode={:?}",
            parsed.mode
        );
        super::errors::throw_srt(&mut env, "CONFIG_INVALID", &msg);
        return 0;
    }

    let mut cfg = SocketConfig::default();
    parsed.overlay.apply_to_socket(&mut cfg);

    let addr = if parsed.host.contains(':') && !parsed.host.starts_with('[') {
        format!("[{}]:{}", parsed.host, parsed.port)
    } else {
        format!("{}:{}", parsed.host, parsed.port)
    };

    let socket = match Socket::connect_with(&cfg, addr.as_str()) {
        Ok(s) => s,
        Err(e) => {
            connect_error(&mut env, &e);
            return 0;
        }
    };

    let transport = SrtTransport::new(socket);
    let inner = PlSender::new(transport, SenderConfig::default());
    Box::into_raw(Box::new(inner)) as jlong
}

/// Send pre-muxed TS bytes. Throws `SrtException` on transport/framing failure.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Sender_nSendBytes(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    data: JByteArray<'_>,
) {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Sender is closed");
        return;
    }
    // SAFETY: handle is a valid Box<PlSender<SrtTransport>> kept alive by the
    // Java object. Reconstituted as a mutable borrow for this call only.
    let inner: &mut PlSender<SrtTransport> =
        unsafe { &mut *(handle as *mut PlSender<SrtTransport>) };

    let bytes: Vec<u8> = match env.convert_byte_array(&data) {
        Ok(b) => b,
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return;
        }
    };

    if let Err(e) = inner.send_ts(&bytes) {
        match e.source {
            SenderErrorSource::Transport(t) => transport_error(&mut env, &t),
            SenderErrorSource::Framing(f) => {
                super::errors::throw_srt(&mut env, "CONFIG_INVALID", &f.to_string())
            }
            _ => super::errors::throw_srt(&mut env, "IO", &e.to_string()),
        }
    }
}

/// Flush any buffered partial TS bundle. Throws `SrtException` on failure.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Sender_nFlush(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Sender is closed");
        return;
    }
    // SAFETY: handle is a valid Box<PlSender<SrtTransport>> kept alive by the
    // Java object.
    let inner: &mut PlSender<SrtTransport> =
        unsafe { &mut *(handle as *mut PlSender<SrtTransport>) };

    if let Err(e) = inner.flush() {
        match e.source {
            SenderErrorSource::Transport(t) => transport_error(&mut env, &t),
            SenderErrorSource::Framing(f) => {
                super::errors::throw_srt(&mut env, "CONFIG_INVALID", &f.to_string())
            }
            _ => super::errors::throw_srt(&mut env, "IO", &e.to_string()),
        }
    }
}

/// Obtain a cancel handle for this Sender. Returns a `jlong` handle on
/// success; returns 0 if the transport doesn't expose a cancel handle (should
/// not happen for a live SrtTransport).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Sender_nCancelHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    if handle == 0 {
        return 0;
    }
    // SAFETY: handle is a valid Box<PlSender<SrtTransport>>.
    let inner: &PlSender<SrtTransport> = unsafe { &*(handle as *const PlSender<SrtTransport>) };

    match inner.cancel_handle() {
        Some(arc) => JniCancel {
            inner: arc,
            flag: AtomicBool::new(false),
        }
        .into_handle(),
        None => {
            // A live SrtTransport always yields a cancel handle (tst-py uses
            // `.expect()` here). Treat absence as an unchecked invariant breach.
            let _ = env.throw_new(
                "java/lang/IllegalStateException",
                "SrtTransport did not return a cancel handle",
            );
            0
        }
    }
}

/// Return a `SocketStats` record for this Sender. Returns null on JNI error
/// (the underlying stats call uses `unwrap_or_default` so the Java side always
/// gets a valid snapshot or null on a builder failure — the latter is
/// considered non-fatal, hence no throw).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Sender_nSocketStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObject<'local> {
    if handle == 0 {
        return JObject::null();
    }
    // SAFETY: handle is a valid Box<PlSender<SrtTransport>>.
    let inner: &PlSender<SrtTransport> = unsafe { &*(handle as *const PlSender<SrtTransport>) };

    let stats = inner.socket_stats().unwrap_or_default();
    match build_socket_stats(&mut env, &stats) {
        Ok(obj) => obj,
        Err(_) => JObject::null(),
    }
}

/// Return an `SrtStats` record for this Sender. Throws `SrtException` (via
/// `io_error`) if the underlying `SrtTransport::stats()` call fails; returns
/// null if the JNI record-builder fails (non-fatal).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Sender_nSrtStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObject<'local> {
    if handle == 0 {
        return JObject::null();
    }
    // SAFETY: handle is a valid Box<PlSender<SrtTransport>>.
    let inner: &PlSender<SrtTransport> = unsafe { &*(handle as *const PlSender<SrtTransport>) };

    match inner.transport().stats() {
        Ok(s) => match build_srt_stats(&mut env, &s) {
            Ok(obj) => obj,
            Err(_) => JObject::null(),
        },
        Err(e) => {
            io_error(&mut env, &e);
            JObject::null()
        }
    }
}

/// Close the Sender, deallocating the native box.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Sender_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: handle is a valid Box<PlSender<SrtTransport>> written by
        // nFromUrl; nClose is called at most once (Java's close() zeroes the field).
        let mut b = unsafe { Box::from_raw(handle as *mut PlSender<SrtTransport>) };
        b.close();
        drop(b);
    }
}

/// Return whether the Sender transport is still live.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Sender_nIsAlive(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jboolean {
    if handle == 0 {
        return 0;
    }
    // SAFETY: handle is a valid Box<PlSender<SrtTransport>>.
    let inner: &PlSender<SrtTransport> = unsafe { &*(handle as *const PlSender<SrtTransport>) };
    u8::from(inner.is_alive())
}

// -----------------------------------------------------------------------
// Receiver  (org.tstrans.srt.Receiver)
// -----------------------------------------------------------------------

/// Allocate a `Receiver` from an SRT listener-mode URL. Returns a `jlong`
/// handle on success; throws `SrtException` and returns 0 on any error.
///
/// The URL must use `mode=listener`. Binds the socket, then blocks on one
/// incoming SRT handshake (one-shot accept). An empty host
/// (`srt://:7000?mode=listener`) binds to `0.0.0.0`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Receiver_nFromUrl(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    url: JString<'_>,
) -> jlong {
    let url_str: String = match env.get_string(&url) {
        Ok(s) => s.into(),
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return 0;
        }
    };

    let parsed = match SrtUrl::parse(&url_str) {
        Ok(p) => p,
        Err(e) => {
            url_error(&mut env, &e);
            return 0;
        }
    };

    if parsed.mode != Mode::Listener {
        let msg = format!(
            "Receiver.fromUrl requires mode=listener; got mode={:?}",
            parsed.mode
        );
        super::errors::throw_srt(&mut env, "CONFIG_INVALID", &msg);
        return 0;
    }

    let mut cfg = ListenerConfig::default();
    parsed.overlay.apply_to_listener(&mut cfg);

    let addr = if parsed.host.is_empty() {
        format!("0.0.0.0:{}", parsed.port)
    } else if parsed.host.contains(':') && !parsed.host.starts_with('[') {
        format!("[{}]:{}", parsed.host, parsed.port)
    } else {
        format!("{}:{}", parsed.host, parsed.port)
    };

    let mut listener = match Listener::bind_with(&cfg, addr.as_str()) {
        Ok(l) => l,
        Err(e) => {
            bind_error(&mut env, &e);
            return 0;
        }
    };

    let (socket, _peer) = match listener.accept() {
        Ok(pair) => pair,
        Err(e) => {
            accept_error(&mut env, &e);
            return 0;
        }
    };

    let transport = SrtTransport::new(socket);
    let inner = PlReceiver::new(transport, ReceiverConfig::default());
    Box::into_raw(Box::new(inner)) as jlong
}

/// Receive one TS packet (188 bytes) from the underlying transport. Returns
/// the packet as a `jbyteArray` on success; throws `SrtException` and returns
/// null on transport/error failure. `maxLen` is accepted for API symmetry with
/// tst-py but a single `next_packet` quantum is always returned.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Receiver_nRecvBytes(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    _max_len: jni::sys::jint,
) -> jbyteArray {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Receiver is closed");
        return std::ptr::null_mut();
    }
    // SAFETY: handle is a valid Box<PlReceiver<SrtTransport>>.
    let inner: &mut PlReceiver<SrtTransport> =
        unsafe { &mut *(handle as *mut PlReceiver<SrtTransport>) };

    match inner.next_packet() {
        Ok(bytes) => match env.byte_array_from_slice(&bytes) {
            Ok(arr) => arr.into_raw(),
            Err(_) => std::ptr::null_mut(),
        },
        Err(e) => {
            match e.source {
                ReceiverErrorSource::Transport(t) => transport_error(&mut env, &t),
                _ => super::errors::throw_srt(&mut env, "IO", &e.to_string()),
            }
            std::ptr::null_mut()
        }
    }
}

/// Obtain a cancel handle for this Receiver. Returns a `jlong` handle on
/// success; returns 0 if no cancel handle is available.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Receiver_nCancelHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    if handle == 0 {
        return 0;
    }
    // SAFETY: handle is a valid Box<PlReceiver<SrtTransport>>.
    let inner: &PlReceiver<SrtTransport> = unsafe { &*(handle as *const PlReceiver<SrtTransport>) };

    match inner.cancel_handle() {
        Some(arc) => JniCancel {
            inner: arc,
            flag: AtomicBool::new(false),
        }
        .into_handle(),
        None => {
            // A live SrtTransport always yields a cancel handle (tst-py uses
            // `.expect()` here). Treat absence as an unchecked invariant breach.
            let _ = env.throw_new(
                "java/lang/IllegalStateException",
                "SrtTransport did not return a cancel handle",
            );
            0
        }
    }
}

/// Return a `SocketStats` record for this Receiver. Returns null on JNI
/// builder error (non-fatal; no throw).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Receiver_nSocketStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObject<'local> {
    if handle == 0 {
        return JObject::null();
    }
    // SAFETY: handle is a valid Box<PlReceiver<SrtTransport>>.
    let inner: &PlReceiver<SrtTransport> = unsafe { &*(handle as *const PlReceiver<SrtTransport>) };

    let stats = inner.socket_stats().unwrap_or_default();
    match build_socket_stats(&mut env, &stats) {
        Ok(obj) => obj,
        Err(_) => JObject::null(),
    }
}

/// Return an `SrtStats` record for this Receiver. Throws `SrtException` (via
/// `io_error`) if the underlying `SrtTransport::stats()` call fails.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Receiver_nSrtStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObject<'local> {
    if handle == 0 {
        return JObject::null();
    }
    // SAFETY: handle is a valid Box<PlReceiver<SrtTransport>>.
    let inner: &PlReceiver<SrtTransport> = unsafe { &*(handle as *const PlReceiver<SrtTransport>) };

    match inner.transport().stats() {
        Ok(s) => match build_srt_stats(&mut env, &s) {
            Ok(obj) => obj,
            Err(_) => JObject::null(),
        },
        Err(e) => {
            io_error(&mut env, &e);
            JObject::null()
        }
    }
}

/// Close the Receiver, deallocating the native box.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Receiver_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: handle is a valid Box<PlReceiver<SrtTransport>> written by
        // nFromUrl; nClose is called at most once.
        let mut b = unsafe { Box::from_raw(handle as *mut PlReceiver<SrtTransport>) };
        b.close();
        drop(b);
    }
}

/// Return whether the Receiver transport is still live.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_Receiver_nIsAlive(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jboolean {
    if handle == 0 {
        return 0;
    }
    // SAFETY: handle is a valid Box<PlReceiver<SrtTransport>>.
    let inner: &PlReceiver<SrtTransport> = unsafe { &*(handle as *const PlReceiver<SrtTransport>) };
    u8::from(inner.is_alive())
}
