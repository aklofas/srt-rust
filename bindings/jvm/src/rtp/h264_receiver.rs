//! JNI surface for `org.tstrans.rtp.H264Receiver` — the single-call convenience
//! wrapper for the RFC 6184 H.264-over-RTP receive path. Ports tst-py's
//! `bindings/python/src/rtp/h264_receiver.rs`.
//!
//! # Architecture
//!
//! `JniH264Receiver` boxes the Rust `H264Receiver` behind the standard
//! `HandleRegistry<T>`. Unlike `DemuxReceiver` there is NO byte-sink
//! registration and the inner value is NOT wrapped in `Arc<Mutex>` — the
//! single-iterator contract means one thread owns the receiver; any cross-thread
//! stop routes through the cancel handle (held separately so `close()` can fire
//! it without taking the resource lock). This matches tst-py's `PyH264Receiver`.
//!
//! `cancel` is held outside the registry slot (cloned at construction into a
//! `Box<dyn FnOnce>` cancel hook). A cross-thread `nClose` fires the hook BEFORE
//! taking the slot (waking any parked `nRecvAu`), then drops the receiver.
//!
//! # Error mapping (mirrors tst-py)
//!
//! - `TransportError::ExplicitClose` → `RtpException(CANCELLED)`
//! - `TransportError::TooLarge`      → `RtpException(MALFORMED_PACKET)`
//! - other `TransportError`          → `RtpException(TRANSPORT)`
//! - `ConnectError`                  → `RtpException(TRANSPORT)`

use std::sync::LazyLock;

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JObject, JObjectArray, JString, JValue};
use jni::sys::{jboolean, jint, jlong, jobject};

use tst_rtp::{H264DepayConfig, H264Receiver, ParameterSetInjection};

use crate::handle::HandleRegistry;

use super::errors::{connect_error_to_rtp, transport_error_to_rtp};
use super::stats::build_socket_stats;

/// Native backing for `org.tstrans.rtp.H264Receiver`. A single-iterator
/// wrapper: one thread owns the recv loop. The cancel handle is extracted at
/// construction and wired as the registry's cancel hook so `close()` wakes
/// a parked `nRecvAu` before taking the slot.
struct JniH264Receiver {
    inner: H264Receiver,
}

/// Per-type leased-handle registry for `org.tstrans.rtp.H264Receiver`. Registers
/// a cancel hook so a cross-thread `close()` wakes a parked `recv_au`.
static REGISTRY: LazyLock<HandleRegistry<JniH264Receiver>> = LazyLock::new(HandleRegistry::new);

/// Build a registry handle from an already-constructed `H264Receiver`.
/// Extracts the cancel handle, registers it as the cancel hook, and returns
/// the boxed handle as `jlong`. Used by both `nListen*` and the RTSP client's
/// `nIntoH264Receiver`.
pub(crate) fn h264_receiver_handle_from_receiver(receiver: H264Receiver) -> jlong {
    let cancel = receiver.cancel_handle();
    let slot = JniH264Receiver { inner: receiver };
    REGISTRY.insert_with_cancel(slot, Some(Box::new(move || cancel.cancel()))) as jlong
}

// ─────────────────────────────────────────────────────────────────────────────
// `H264Receiver.nListen(url)` — default H264DepayConfig
// ─────────────────────────────────────────────────────────────────────────────

/// `H264Receiver.nListen(url)` — parse an `rtp://host:port?pt=N` URL, bind a
/// UDP socket, and return a handle. Mirrors tst-py `PyH264Receiver::listen` with
/// `config=None`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_H264Receiver_nListen<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    url: JString<'local>,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        let url_str: String = match env.get_string(&url) {
            Ok(s) => s.into(),
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                return 0;
            }
        };
        match H264Receiver::listen(&url_str) {
            Ok(receiver) => h264_receiver_handle_from_receiver(receiver),
            Err(e) => {
                connect_error_to_rtp(env, &e);
                0
            }
        }
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// `H264Receiver.nListenWithConfig(...)` — explicit H264DepayConfig
// ─────────────────────────────────────────────────────────────────────────────

/// `H264Receiver.nListenWithConfig(url, payloadType, parameterSetInjection,
/// initialParameterSets, maxAuBytes)` — parse the URL, build the config from
/// caller-supplied fields, bind, and return a handle. Mirrors tst-py
/// `PyH264Receiver::listen` with an explicit config.
///
/// `initialParameterSets` is a `byte[][]` (Java `JObjectArray`); may be null
/// (treated as empty). Each element is a raw NALU byte array.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_org_tstrans_rtp_H264Receiver_nListenWithConfig<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    url: JString<'local>,
    payload_type: jint,
    parameter_set_injection: jint,
    initial_parameter_sets: JObjectArray<'local>,
    max_au_bytes: jlong,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        let url_str: String = match env.get_string(&url) {
            Ok(s) => s.into(),
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                return 0;
            }
        };

        // Build H264DepayConfig from the caller's fields (mirrors Python's kwarg path).
        let mut config = H264DepayConfig::default();
        config.payload_type = payload_type as u8;
        config.parameter_set_injection = match parameter_set_injection {
            1 => ParameterSetInjection::BeforeIdr,
            _ => ParameterSetInjection::None,
        };
        config.max_au_bytes = max_au_bytes as usize;

        // Read each NALU from the byte[][] (may be null / zero-length).
        if !initial_parameter_sets.is_null() {
            let len = match env.get_array_length(&initial_parameter_sets) {
                Ok(n) => n,
                Err(e) => {
                    let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                    return 0;
                }
            };
            for i in 0..len {
                let elem_obj = match env.get_object_array_element(&initial_parameter_sets, i) {
                    Ok(o) => o,
                    Err(e) => {
                        let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                        return 0;
                    }
                };
                if elem_obj.is_null() {
                    continue;
                }
                let arr = JByteArray::from(elem_obj);
                let bytes = match env.convert_byte_array(&arr) {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                        return 0;
                    }
                };
                config.initial_parameter_sets.push(bytes);
            }
        }

        // `listen_with` parses the URL, overrides `config.payload_type` from `?pt=`,
        // and binds. Mirrors tst-py's listen_with path.
        let parsed = match tst_rtp::url::RtpUrl::parse(&url_str) {
            Ok(p) => p,
            Err(e) => {
                super::errors::throw_rtp(env, "TRANSPORT", &e.to_string());
                return 0;
            }
        };
        match H264Receiver::listen_with(&parsed, config) {
            Ok(receiver) => h264_receiver_handle_from_receiver(receiver),
            Err(e) => {
                connect_error_to_rtp(env, &e);
                0
            }
        }
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// `H264Receiver.nRecvAu(handle)` — blocking recv
// ─────────────────────────────────────────────────────────────────────────────

/// `H264Receiver.nRecvAu(handle)` — block until the next H.264 Access Unit is
/// reassembled. Returns an `H264AccessUnit` Java object on success, `null` at EOS,
/// or throws on error. Mirrors tst-py `PyH264Receiver::recv_au`.
///
/// The registry lease holds the slot's resource lock for the duration of the
/// (possibly parked) `recv_au` call. A concurrent `close()` fires the cancel hook
/// (waking the parked recv) BEFORE it takes the resource; so `close()` is a safe
/// cross-thread stop.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_H264Receiver_nRecvAu<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    crate::panic::jni_catch(&mut env, std::ptr::null_mut(), |env| {
        // `with_poisoning` holds the resource lock while recv_au runs (which may park
        // the calling thread). A concurrent `close()` fires the cancel hook first so
        // this call returns promptly then the resource is taken.
        let Some(result) = REGISTRY.with_poisoning(handle as u64, |jdr| jdr.inner.recv_au()) else {
            // Closed/absent — clean EOS: return null (the Java side returns null
            // from recvAu(), which the caller treats as end-of-stream).
            return JObject::null().into_raw();
        };

        match result {
            // EOS (cancel / clean RTSP teardown): return null
            Ok(None) => JObject::null().into_raw(),
            Ok(Some(au)) => {
                // Build H264AccessUnit Java object. Every field is copied:
                // - annexb: byte[] heap copy (JDK<22 rule — never direct ByteBuffer over Rust memory)
                // - pts: long (i64 ticks)
                // - keyFrame: boolean
                // - rtpTimestamp: long (u32 widened to i64)
                let annexb_arr = match env.byte_array_from_slice(&au.annexb) {
                    Ok(a) => a,
                    Err(e) => {
                        let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                        return JObject::null().into_raw();
                    }
                };
                let pts = au.pts.as_ticks();
                let key_frame: jboolean = u8::from(au.key_frame);
                let rtp_timestamp = i64::from(au.rtp_timestamp);

                // Construct `new H264AccessUnit(byte[], long, boolean, long)`.
                match env.ensure_local_capacity(4) {
                    Ok(_) => {}
                    Err(e) => {
                        let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                        return JObject::null().into_raw();
                    }
                }
                let obj = match env.new_object(
                    "org/tstrans/rtp/H264AccessUnit",
                    "([BJZJ)V",
                    &[
                        JValue::Object(&annexb_arr.into()),
                        JValue::Long(pts),
                        JValue::Bool(key_frame),
                        JValue::Long(rtp_timestamp),
                    ],
                ) {
                    Ok(o) => o,
                    Err(e) => {
                        let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                        return JObject::null().into_raw();
                    }
                };
                obj.into_raw()
            }
            Err(e) => {
                transport_error_to_rtp(env, &e);
                JObject::null().into_raw()
            }
        }
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Stats natives
// ─────────────────────────────────────────────────────────────────────────────

/// `H264Receiver.nDepayStats(handle)` — snapshot the depacketizer counters.
/// Returns an `H264DepayStats` Java record. Throws `IllegalStateException` on
/// a closed handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_H264Receiver_nDepayStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    crate::panic::jni_catch(&mut env, std::ptr::null_mut(), |env| {
        let Some(s) = REGISTRY.with(handle as u64, |jdr| jdr.inner.depay_stats()) else {
            crate::error::throw_closed(env, "H264Receiver");
            return JObject::null().into_raw();
        };
        // Construct `new H264DepayStats(long x9)`.
        match env.new_object(
            "org/tstrans/rtp/H264DepayStats",
            "(JJJJJJJJJ)V",
            &[
                JValue::Long(s.aus_emitted as i64),
                JValue::Long(s.aus_dropped as i64),
                JValue::Long(s.aus_dropped_oversize as i64),
                JValue::Long(s.packets_discarded as i64),
                JValue::Long(s.nalus_discarded as i64),
                JValue::Long(s.seq_gaps as i64),
                JValue::Long(s.duplicate_packets as i64),
                JValue::Long(s.parameter_set_updates as i64),
                JValue::Long(s.ssrc_changes as i64),
            ],
        ) {
            Ok(o) => o.into_raw(),
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                JObject::null().into_raw()
            }
        }
    })
}

/// `H264Receiver.nRtpStats(handle)` — RTP protocol-level malformed-packet counter.
/// Returns an `RtpStats` Java record.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_H264Receiver_nRtpStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    crate::panic::jni_catch(&mut env, std::ptr::null_mut(), |env| {
        let Some(s) = REGISTRY.with(handle as u64, |jdr| jdr.inner.rtp_stats()) else {
            crate::error::throw_closed(env, "H264Receiver");
            return JObject::null().into_raw();
        };
        match env.new_object(
            "org/tstrans/rtp/RtpStats",
            "(J)V",
            &[JValue::Long(s.malformed_packets as i64)],
        ) {
            Ok(o) => o.into_raw(),
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                JObject::null().into_raw()
            }
        }
    })
}

/// `H264Receiver.nSocketStats(handle)` — wire-level throughput statistics.
/// Returns an `org.tstrans.rtp.SocketStats` record (direct, NOT wrapped in
/// `TransportStats` — mirrors the Rust/Python asymmetry documented in the Java
/// class's Javadoc).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_H264Receiver_nSocketStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    crate::panic::jni_catch(&mut env, std::ptr::null_mut(), |env| {
        let Some(s) = REGISTRY.with(handle as u64, |jdr| jdr.inner.socket_stats()) else {
            crate::error::throw_closed(env, "H264Receiver");
            return JObject::null().into_raw();
        };
        match build_socket_stats(env, &s) {
            Ok(o) => o.into_raw(),
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                JObject::null().into_raw()
            }
        }
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Misc natives
// ─────────────────────────────────────────────────────────────────────────────

/// `H264Receiver.nLocalAddr(handle)` — local UDP socket address as
/// `"host:port"` string, or `null` for the TCP-interleaved (RTSP) path.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_H264Receiver_nLocalAddr<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    crate::panic::jni_catch(&mut env, std::ptr::null_mut(), |env| {
        let Some(addr) = REGISTRY.with(handle as u64, |jdr| jdr.inner.local_addr()) else {
            crate::error::throw_closed(env, "H264Receiver");
            return JObject::null().into_raw();
        };
        match addr {
            None => JObject::null().into_raw(),
            Some(a) => {
                let s = a.to_string();
                match env.new_string(&s) {
                    Ok(js) => JObject::from(js).into_raw(),
                    Err(e) => {
                        let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                        JObject::null().into_raw()
                    }
                }
            }
        }
    })
}

/// `H264Receiver.nCancelHandle(handle)` — clone the cancel handle into a
/// `org.tstrans.rtp.CancelHandle` registry slot and return the slot id as
/// `jlong`. Returns 0 on a closed receiver.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_H264Receiver_nCancelHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        let Some(cancel_arc) = REGISTRY.with(handle as u64, |jdr| jdr.inner.cancel_handle()) else {
            crate::error::throw_closed(env, "H264Receiver");
            return 0;
        };
        // Coerce Arc<RtpCancelHandle> to Arc<dyn TransportCancel + Send + Sync>
        // (RtpCancelHandle implements TransportCancel).
        let erased: std::sync::Arc<dyn tst_core::transport::TransportCancel + Send + Sync> =
            cancel_arc;
        crate::rtp::JniRtpCancel { inner: erased }.into_handle()
    })
}

/// `H264Receiver.nClose(handle)` — cancel-first (wakes a parked recv), then take
/// + close the inner receiver and free the box. Atomic + idempotent.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_H264Receiver_nClose(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    crate::panic::jni_catch(&mut env, (), |_env| {
        // `REGISTRY.close` fires the cancel hook FIRST (waking any parked recv_au
        // WITHOUT taking the resource lock), THEN takes + drops the slot under the
        // lock (blocking briefly until the woken recv releases it). Atomic +
        // idempotent: a double close finds the id gone → no-op.
        if let Some(mut slot) = REGISTRY.close(handle as u64) {
            slot.inner.close();
        }
    })
}
