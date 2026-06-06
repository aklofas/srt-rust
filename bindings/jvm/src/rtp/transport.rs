//! JNI exports for `org.tstrans.rtp.Sender` and `org.tstrans.rtp.Receiver`.
//!
//! Each Java class is handle-backed by a `Box`:
//! - `Sender`   → `Box<JniRtpSender>`   (wraps `tst_rtp::RtpTransport`).
//! - `Receiver` → `Box<JniRtpReceiver>` (wraps `tst_rtp::RtpRecvTransport` + a
//!   reusable recv scratch buffer, mirroring tst-py's `PyReceiver.scratch`).
//!
//! Unlike the srt JVM surface (which wraps `tst_pipeline::Sender/Receiver`),
//! the rtp surface wraps the transport DIRECTLY and calls the
//! `Transport`/`RecvTransport` trait methods — exactly as tst-py's
//! `bindings/python/src/rtp/transport.rs` does.

use std::sync::Arc;

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JObject, JString};
use jni::sys::{jbyteArray, jint, jlong};
use tst_core::transport::{RecvTransport, Transport, TransportCancel};
use tst_rtp::builder::RtpRecvSocketBuilder;
use tst_rtp::{RtpRecvTransport, RtpSocketBuilder, RtpTransport};

use super::JniRtpCancel;
use super::errors::{connect_error_to_rtp, throw_rtp, transport_error_to_rtp};
use super::stats::build_socket_stats;

struct JniRtpSender {
    inner: RtpTransport,
    cancel: Arc<dyn TransportCancel + Send + Sync>,
}

struct JniRtpReceiver {
    inner: RtpRecvTransport,
    cancel: Arc<dyn TransportCancel + Send + Sync>,
    scratch: Vec<u8>,
}

/// Unbox a nullable `java.lang.Long` SSRC arg into `Option<u32>`. Returns
/// `Err(())` (after throwing IllegalArgumentException) on out-of-range values.
fn unbox_ssrc(env: &mut JNIEnv, obj: &JObject) -> Result<Option<u32>, ()> {
    if obj.is_null() {
        return Ok(None);
    }
    let v = match env.call_method(obj, "longValue", "()J", &[]) {
        Ok(jv) => jv.j().unwrap_or(-1),
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return Err(());
        }
    };
    if v < 0 || v > i64::from(u32::MAX) {
        let _ = env.throw_new(
            "java/lang/IllegalArgumentException",
            format!("ssrc out of u32 range: {v}"),
        );
        return Err(());
    }
    Ok(Some(v as u32))
}

// ── Sender (org.tstrans.rtp.Sender) ────────────────────────────────────────

/// Allocate a `Sender` from an `rtp://host:port` URL. Returns a `jlong` handle;
/// throws `RtpException` and returns 0 on error. `pktSize` is the UDP datagram
/// size; `ssrcBoxed` is a nullable `java.lang.Long` SSRC (random when null).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Sender_nFromUrl(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    url: JString<'_>,
    pkt_size: jint,
    ssrc_boxed: JObject<'_>,
) -> jlong {
    let url_str: String = match env.get_string(&url) {
        Ok(s) => s.into(),
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return 0;
        }
    };
    let ssrc = match unbox_ssrc(&mut env, &ssrc_boxed) {
        Ok(v) => v,
        Err(()) => return 0,
    };

    let mut builder = match RtpSocketBuilder::from_url(&url_str) {
        Ok(b) => b,
        Err(e) => {
            throw_rtp(&mut env, "TRANSPORT", &e.to_string());
            return 0;
        }
    };
    builder.pkt_size(pkt_size.max(0) as usize);
    if let Some(s) = ssrc {
        builder.ssrc(s);
    }
    let inner = match builder.build() {
        Ok(t) => t,
        Err(e) => {
            connect_error_to_rtp(&mut env, &e);
            return 0;
        }
    };
    let cancel = inner
        .cancel_handle()
        .expect("RtpTransport always returns Some(cancel_handle)");
    Box::into_raw(Box::new(JniRtpSender { inner, cancel })) as jlong
}

/// Send one MPEG-TS payload chunk over RTP. Throws `RtpException` on failure.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Sender_nSend(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    data: JByteArray<'_>,
) {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Sender is closed");
        return;
    }
    // SAFETY: valid Box<JniRtpSender>; send_bytes is &mut self.
    let w: &mut JniRtpSender = unsafe { &mut *(handle as *mut JniRtpSender) };
    let bytes: Vec<u8> = match env.convert_byte_array(&data) {
        Ok(b) => b,
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return;
        }
    };
    if let Err(e) = w.inner.send_bytes(&bytes) {
        transport_error_to_rtp(&mut env, &e);
    }
}

/// Return a `SocketStats` record. Returns null on JNI builder error (non-fatal).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Sender_nSocketStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObject<'local> {
    if handle == 0 {
        return JObject::null();
    }
    // SAFETY: valid Box<JniRtpSender>; socket_stats is &self.
    let w: &JniRtpSender = unsafe { &*(handle as *const JniRtpSender) };
    let stats = w.inner.socket_stats().unwrap_or_default();
    build_socket_stats(&mut env, &stats).unwrap_or_else(|_| JObject::null())
}

/// Return a cancel-handle `jlong` (Box<JniRtpCancel>).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Sender_nCancelHandle(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    if handle == 0 {
        return 0;
    }
    // SAFETY: valid Box<JniRtpSender>.
    let w: &JniRtpSender = unsafe { &*(handle as *const JniRtpSender) };
    JniRtpCancel {
        inner: w.cancel.clone(),
    }
    .into_handle()
}

/// Close the Sender, freeing the native box.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Sender_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: valid Box<JniRtpSender>; close() zeroes the Java field (runs once).
        let mut w = unsafe { Box::from_raw(handle as *mut JniRtpSender) };
        w.inner.close();
        drop(w);
    }
}

// ── Receiver (org.tstrans.rtp.Receiver) ────────────────────────────────────

/// Allocate a `Receiver` bound to an `rtp://host:port` URL. Returns a `jlong`
/// handle; throws `RtpException` and returns 0 on error.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Receiver_nFromUrl(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    url: JString<'_>,
    pkt_size: jint,
) -> jlong {
    let url_str: String = match env.get_string(&url) {
        Ok(s) => s.into(),
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return 0;
        }
    };
    let mut builder = match RtpRecvSocketBuilder::from_url(&url_str) {
        Ok(b) => b,
        Err(e) => {
            throw_rtp(&mut env, "TRANSPORT", &e.to_string());
            return 0;
        }
    };
    builder.pkt_size(pkt_size.max(0) as usize);
    let inner = match builder.build() {
        Ok(t) => t,
        Err(e) => {
            connect_error_to_rtp(&mut env, &e);
            return 0;
        }
    };
    let scratch_len = inner.max_payload();
    let cancel = inner
        .cancel_handle()
        .expect("RtpRecvTransport always returns Some(cancel_handle)");
    Box::into_raw(Box::new(JniRtpReceiver {
        inner,
        cancel,
        scratch: vec![0u8; scratch_len],
    })) as jlong
}

/// Receive one MPEG-TS payload chunk (RTP header already stripped). Returns the
/// bytes as a `jbyteArray`; throws `RtpException` and returns null on failure.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Receiver_nRecv(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jbyteArray {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Receiver is closed");
        return std::ptr::null_mut();
    }
    // SAFETY: valid Box<JniRtpReceiver>; recv_bytes is &mut self.
    let w: &mut JniRtpReceiver = unsafe { &mut *(handle as *mut JniRtpReceiver) };
    let scratch: &mut [u8] = w.scratch.as_mut_slice();
    match w.inner.recv_bytes(scratch) {
        Ok(n) => match env.byte_array_from_slice(&w.scratch[..n]) {
            Ok(arr) => arr.into_raw(),
            // Allocating the Java array failed (effectively OOM). Throw rather
            // than return null silently, so `recv()` always yields bytes or an
            // RtpException — matching tst-py's contract (it never returns None).
            Err(_) => {
                throw_rtp(&mut env, "TRANSPORT", "failed to allocate received packet");
                std::ptr::null_mut()
            }
        },
        Err(e) => {
            transport_error_to_rtp(&mut env, &e);
            std::ptr::null_mut()
        }
    }
}

/// Return a `SocketStats` record. Returns null on JNI builder error (non-fatal).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Receiver_nSocketStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObject<'local> {
    if handle == 0 {
        return JObject::null();
    }
    // SAFETY: valid Box<JniRtpReceiver>; socket_stats is &self.
    let w: &JniRtpReceiver = unsafe { &*(handle as *const JniRtpReceiver) };
    let stats = w.inner.socket_stats().unwrap_or_default();
    build_socket_stats(&mut env, &stats).unwrap_or_else(|_| JObject::null())
}

/// Return a cancel-handle `jlong` (Box<JniRtpCancel>).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Receiver_nCancelHandle(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    if handle == 0 {
        return 0;
    }
    // SAFETY: valid Box<JniRtpReceiver>.
    let w: &JniRtpReceiver = unsafe { &*(handle as *const JniRtpReceiver) };
    JniRtpCancel {
        inner: w.cancel.clone(),
    }
    .into_handle()
}

/// Close the Receiver, freeing the native box. Flips the cancel first so a
/// parked `recv` on another thread unparks (mirrors tst-py `PyReceiver.close`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Receiver_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: valid Box<JniRtpReceiver>; runs once (Java zeroes the field).
        let mut w = unsafe { Box::from_raw(handle as *mut JniRtpReceiver) };
        w.cancel.cancel();
        w.inner.close();
        drop(w);
    }
}
