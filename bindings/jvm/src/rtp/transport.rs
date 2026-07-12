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
use std::sync::LazyLock;

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JObject, JString};
use jni::sys::{jbyteArray, jint, jlong};
use tst_core::transport::{RecvTransport, Transport, TransportCancel};
use tst_rtp::builder::RtpRecvSocketBuilder;
use tst_rtp::{RtpRecvTransport, RtpSocketBuilder, RtpTransport};

use super::JniRtpCancel;
use super::errors::{connect_error_to_rtp, throw_rtp, transport_error_to_rtp};
use super::stats::build_socket_stats;
use crate::handle::HandleRegistry;

struct JniRtpSender {
    inner: RtpTransport,
    cancel: Arc<dyn TransportCancel + Send + Sync>,
}

struct JniRtpReceiver {
    inner: RtpRecvTransport,
    cancel: Arc<dyn TransportCancel + Send + Sync>,
    scratch: Vec<u8>,
}

/// Per-type leased-handle registries. Both register a cancel hook so a
/// cross-thread `close()` wakes a parked `send`/`recv` before taking the
/// resource lock (mirrors the round-1 cancel-first-then-free discipline).
static REGISTRY_SENDER: LazyLock<HandleRegistry<JniRtpSender>> = LazyLock::new(HandleRegistry::new);
static REGISTRY_RECEIVER: LazyLock<HandleRegistry<JniRtpReceiver>> =
    LazyLock::new(HandleRegistry::new);

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
    crate::panic::jni_catch(&mut env, 0, |env| {
        let url_str: String = match env.get_string(&url) {
            Ok(s) => s.into(),
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                return 0;
            }
        };
        let ssrc = match unbox_ssrc(env, &ssrc_boxed) {
            Ok(v) => v,
            Err(()) => return 0,
        };

        let mut builder = match RtpSocketBuilder::from_url(&url_str) {
            Ok(b) => b,
            Err(e) => {
                throw_rtp(env, "TRANSPORT", &e.to_string());
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
                connect_error_to_rtp(env, &e);
                return 0;
            }
        };
        let cancel = inner
            .cancel_handle()
            .expect("RtpTransport always returns Some(cancel_handle)");
        let cancel_for_hook = cancel.clone();
        REGISTRY_SENDER.insert_with_cancel(
            JniRtpSender { inner, cancel },
            Some(Box::new(move || cancel_for_hook.cancel())),
        ) as jlong
    })
}

/// Send one MPEG-TS payload chunk over RTP. Throws `RtpException` on failure.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Sender_nSend(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    data: JByteArray<'_>,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        let bytes: Vec<u8> = match env.convert_byte_array(&data) {
            Ok(b) => b,
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                return;
            }
        };
        match REGISTRY_SENDER.with_poisoning(handle as u64, |w| w.inner.send_bytes(&bytes)) {
            Some(Ok(())) => {}
            Some(Err(e)) => transport_error_to_rtp(env, &e),
            None => {
                crate::error::throw_closed(env, "Sender");
            }
        }
    })
}

/// Return a `SocketStats` record. Returns null on JNI builder error (non-fatal).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Sender_nSocketStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObject<'local> {
    crate::panic::jni_catch(&mut env, JObject::null(), |env| {
        let Some(stats) = REGISTRY_SENDER.with(handle as u64, |w| {
            w.inner.socket_stats().unwrap_or_default()
        }) else {
            return JObject::null();
        };
        build_socket_stats(env, &stats).unwrap_or_else(|_| JObject::null())
    })
}

/// Return a cancel-handle `jlong` (Box<JniRtpCancel>).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Sender_nCancelHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |_env| {
        REGISTRY_SENDER
            .with(handle as u64, |w| {
                JniRtpCancel {
                    inner: w.cancel.clone(),
                }
                .into_handle()
            })
            .unwrap_or(0)
    })
}

/// Close the Sender, freeing the native box. The cancel hook fires first (waking
/// a parked `send`) before the resource is taken.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Sender_nClose(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    // Atomic + idempotent: cancel hook wakes a parked send, then take + teardown.
    crate::panic::jni_catch(&mut env, (), |_env| {
        if let Some(mut w) = REGISTRY_SENDER.close(handle as u64) {
            w.inner.close();
        }
    })
}

// ── Receiver (org.tstrans.rtp.Receiver) ────────────────────────────────────

/// Allocate a `Receiver` bound to an `rtp://host:port` URL. Returns a `jlong`
/// handle; throws `RtpException` and returns 0 on error.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Receiver_nFromUrl(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    url: JString<'_>,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        let url_str: String = match env.get_string(&url) {
            Ok(s) => s.into(),
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                return 0;
            }
        };
        let builder = match RtpRecvSocketBuilder::from_url(&url_str) {
            Ok(b) => b,
            Err(e) => {
                throw_rtp(env, "TRANSPORT", &e.to_string());
                return 0;
            }
        };
        let inner = match builder.build() {
            Ok(t) => t,
            Err(e) => {
                connect_error_to_rtp(env, &e);
                return 0;
            }
        };
        let scratch_len = inner.max_payload();
        let cancel = inner
            .cancel_handle()
            .expect("RtpRecvTransport always returns Some(cancel_handle)");
        let cancel_for_hook = cancel.clone();
        REGISTRY_RECEIVER.insert_with_cancel(
            JniRtpReceiver {
                inner,
                cancel,
                scratch: vec![0u8; scratch_len],
            },
            Some(Box::new(move || cancel_for_hook.cancel())),
        ) as jlong
    })
}

/// Receive one MPEG-TS payload chunk (RTP header already stripped). Returns the
/// bytes as a `jbyteArray`; throws `RtpException` and returns null on failure.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Receiver_nRecv(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jbyteArray {
    crate::panic::jni_catch(&mut env, std::ptr::null_mut(), |env| {
        // `recv_bytes` may park; the closure holds the resource lock for its
        // duration. A concurrent `close()` fires the cancel hook (waking the recv)
        // before taking the lock. We copy the received bytes OUT of `scratch` inside
        // the closure so the Java array is built after the lease releases.
        let Some(res) = REGISTRY_RECEIVER.with_poisoning(handle as u64, |w| {
            let n = w.inner.recv_bytes(w.scratch.as_mut_slice())?;
            Ok::<Vec<u8>, _>(w.scratch[..n].to_vec())
        }) else {
            crate::error::throw_closed(env, "Receiver");
            return std::ptr::null_mut();
        };
        match res {
            Ok(bytes) => match env.byte_array_from_slice(&bytes) {
                Ok(arr) => arr.into_raw(),
                // Allocating the Java array failed (effectively OOM). Throw rather
                // than return null silently, so `recv()` always yields bytes or an
                // RtpException — matching tst-py's contract (it never returns None).
                Err(_) => {
                    throw_rtp(env, "TRANSPORT", "failed to allocate received packet");
                    std::ptr::null_mut()
                }
            },
            Err(e) => {
                transport_error_to_rtp(env, &e);
                std::ptr::null_mut()
            }
        }
    })
}

/// Return a `SocketStats` record. Returns null on JNI builder error (non-fatal).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Receiver_nSocketStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObject<'local> {
    crate::panic::jni_catch(&mut env, JObject::null(), |env| {
        let Some(stats) = REGISTRY_RECEIVER.with(handle as u64, |w| {
            w.inner.socket_stats().unwrap_or_default()
        }) else {
            return JObject::null();
        };
        build_socket_stats(env, &stats).unwrap_or_else(|_| JObject::null())
    })
}

/// Return a cancel-handle `jlong` (Box<JniRtpCancel>).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Receiver_nCancelHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |_env| {
        REGISTRY_RECEIVER
            .with(handle as u64, |w| {
                JniRtpCancel {
                    inner: w.cancel.clone(),
                }
                .into_handle()
            })
            .unwrap_or(0)
    })
}

/// Close the Receiver, freeing the native box. The cancel hook fires first
/// (waking a parked `recv`) before the resource is taken — mirrors tst-py
/// `PyReceiver.close`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_Receiver_nClose(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    // Atomic + idempotent: cancel hook wakes a parked recv, then take + teardown.
    crate::panic::jni_catch(&mut env, (), |_env| {
        if let Some(mut w) = REGISTRY_RECEIVER.close(handle as u64) {
            w.inner.close();
        }
    })
}
