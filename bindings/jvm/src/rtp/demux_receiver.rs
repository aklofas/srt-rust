//! JNI surface for `org.tstrans.rtp.DemuxReceiver` — the single-call convenience
//! wrapper that owns a `Demuxer` + an RTP `RtpRecvTransport`.
//!
//! Wraps `tst_pipeline::DemuxReceiver<tst_rtp::RtpRecvTransport>`: bind a UDP RTP
//! receiver to a URL, demux the resulting MPEG-TS stream, and iterate over
//! `DemuxEvent` instances. Ports tst-py's
//! `bindings/python/src/rtp/demux_receiver.rs`.
//!
//! Concurrency (the headline rtp divergence from the srt JVM DemuxReceiver):
//! `inner` is held under `Arc<Mutex<Option<...>>>` and `cancel` is held OUTSIDE
//! the mutex. `nNext` locks `inner` inside the call, runs `recv_event`, drops the
//! guard, then drains `sink_error`. `nClose` fires `cancel` FIRST (waking a parked
//! `recv_event` within ~100 ms WITHOUT taking the lock), then locks + takes +
//! drops `inner`. This makes `close()` a safe cross-thread stop for a parked
//! iteration — necessary because the rtp convenience wrapper exposes NO public
//! cancel handle (matching tst-py) and RTP/UDP has no connection-close signal to
//! end iteration. The srt JVM DemuxReceiver's no-mutex model is deliberately NOT
//! reused (it relied on a public cancel handle + SRT connection-close).
//!
//! `add_byte_sink` discipline: the registered `Box<dyn FnMut(&[u8]) + Send>` runs
//! on the receiver's own thread inside `recv_event`, attaches to the JVM, and
//! upcalls a `Consumer<byte[]>`. It holds NO Java monitor across the upcall and
//! touches ONLY the captured `consumer` + `sink_error` — never `inner` (so it
//! cannot deadlock against the `inner` lock the parked `nNext` holds: a callback
//! that re-entered the receiver is documented as forbidden, and a concurrent
//! `close()` fires `cancel` before acquiring the lock). A callback exception is
//! captured first-wins into `sink_error` (as a `GlobalRef`) and re-thrown from the
//! next `nNext` after the `inner` guard is dropped.

use std::sync::{Arc, Mutex};

use jni::JNIEnv;
use jni::objects::{GlobalRef, JClass, JObject, JString, JThrowable, JValue};
use jni::sys::{jboolean, jint, jlong, jobject};

use tst_core::mpegts::demux::DemuxEvent;
use tst_core::transport::TransportCancel;
use tst_pipeline::{
    DemuxReceiver as RustDemuxReceiver, DemuxReceiverError, DemuxReceiverErrorSource,
};
use tst_rtp::RtpRecvTransport;
use tst_rtp::builder::RtpRecvSocketBuilder;

use crate::mpegts::{build_demux_config_from_args, convert_event, throw_demux_error};

use super::errors::{connect_error_to_rtp, throw_rtp, transport_error_to_rtp};
use super::mux_sender::{build_muxer_stats, build_rtp_transport_stats};
use super::stats::build_socket_stats;

/// Native backing for `org.tstrans.rtp.DemuxReceiver`. Faithful port of tst-py's
/// `PyDemuxReceiver`: `inner` behind a mutex so a cross-thread `close()` can free
/// it safely while `nNext` is parked; `cancel` held outside the mutex so `close()`
/// can wake a parked recv before acquiring the lock; `sink_error` for the
/// fail-loud byte-sink re-raise.
struct JniRtpDemuxReceiver {
    inner: Arc<Mutex<Option<RustDemuxReceiver<RtpRecvTransport>>>>,
    cancel: Arc<dyn TransportCancel + Send + Sync>,
    sink_error: Arc<Mutex<Option<GlobalRef>>>,
}

/// Map a `DemuxReceiverError` raised by `recv_event` onto a thrown Java exception.
/// Transport-side → `RtpException`; demux-side → `DemuxException`. Mirrors tst-py's
/// `demux_recv_error_to_pyerr`.
fn throw_demux_recv_error(env: &mut JNIEnv, e: &DemuxReceiverError) {
    match &e.source {
        DemuxReceiverErrorSource::Transport(t) => transport_error_to_rtp(env, t),
        DemuxReceiverErrorSource::Demux(d) => throw_demux_error(env, d),
        // `DemuxReceiverErrorSource` is non-exhaustive; route any future variant
        // through `RtpException(TRANSPORT)` with the Display message preserved.
        _ => throw_rtp(env, "TRANSPORT", &e.to_string()),
    }
}

/// Build a `JniRtpDemuxReceiver` from an `rtp://` URL (with or without explicit
/// demux options). Returns the boxed handle as `jlong`, or `0` with a pending
/// exception on any failure.
fn build_from_url(
    env: &mut JNIEnv,
    url: &JString,
    opts: Option<tst_core::mpegts::demux::DemuxerConfig>,
) -> jlong {
    let url_str: String = match env.get_string(url) {
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
    let transport = match builder.build() {
        Ok(t) => t,
        Err(e) => {
            connect_error_to_rtp(env, &e);
            return 0;
        }
    };

    demux_receiver_handle_from_transport(transport, opts)
}

/// Build a `JniRtpDemuxReceiver` handle from an already-constructed
/// `RtpRecvTransport` (used by the RTSP client's `intoDemuxReceiver` — it hands
/// off the SETUP-time recv transport). Mirrors tst-py's
/// `PyDemuxReceiver::from_recv_transport[_with_config]`. Infallible (box alloc).
pub(crate) fn demux_receiver_handle_from_transport(
    transport: RtpRecvTransport,
    opts: Option<tst_core::mpegts::demux::DemuxerConfig>,
) -> jlong {
    let receiver = match opts {
        None => RustDemuxReceiver::new(transport),
        Some(opts) => RustDemuxReceiver::with_demux_options(transport, opts),
    };
    let cancel = receiver
        .cancel_handle()
        .expect("RtpRecvTransport always returns Some(cancel_handle)");
    let jdr = JniRtpDemuxReceiver {
        inner: Arc::new(Mutex::new(Some(receiver))),
        cancel,
        sink_error: Arc::new(Mutex::new(None)),
    };
    Box::into_raw(Box::new(jdr)) as jlong
}

/// `DemuxReceiver.nFromUrl(url)` — default demux options.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_DemuxReceiver_nFromUrl<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    url: JString<'local>,
) -> jlong {
    build_from_url(&mut env, &url, None)
}

/// `DemuxReceiver.nFromUrlWithConfig(url, ...)` — explicit `DemuxerConfig`.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_org_tstrans_rtp_DemuxReceiver_nFromUrlWithConfig<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    url: JString<'local>,
    strict: jint,
    pes_cap_per_pid: jlong,
    pes_cap_total: jlong,
    cfi: jboolean,
    av1: jint,
    au_cell_cap: jlong,
    lenient_psi: jboolean,
) -> jlong {
    let opts = build_demux_config_from_args(
        strict,
        pes_cap_per_pid,
        pes_cap_total,
        cfi,
        av1,
        au_cell_cap,
        lenient_psi,
    );
    build_from_url(&mut env, &url, Some(opts))
}

/// Validate a handle. Returns the live `*const JniRtpDemuxReceiver`, or throws
/// `IllegalStateException` + returns `None` for a zero (closed) handle.
fn checked_receiver(env: &mut JNIEnv, handle: jlong) -> Option<*const JniRtpDemuxReceiver> {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "DemuxReceiver is closed");
        return None;
    }
    Some(handle as *const JniRtpDemuxReceiver)
}

/// `nNext(handle)` — block until the next `DemuxEvent`. Java `null` on clean end
/// (receiver closed / drained → iterator `hasNext` returns false). Throws
/// `RtpException` / `DemuxException` on a recv-side error, or re-raises a captured
/// byte-sink exception fail-loud.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_DemuxReceiver_nNext<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    let Some(ptr) = checked_receiver(&mut env, handle) else {
        return JObject::null().into_raw();
    };
    // SAFETY: validated non-zero live `Box<JniRtpDemuxReceiver>`. We only touch
    // the Arc fields (clone-and-lock), never a `&mut` to the box itself. These
    // top-of-call field reads are sound under the single-iterator + parked-close
    // contract: `nClose` frees the box only AFTER its cancel wakes a recv that is
    // already PARKED past this point holding the `inner` guard. A second concurrent
    // `nNext`, or a `close()` racing a fresh `nNext` ENTRY here, is UB — the same
    // accepted contract as the srt DemuxReceiver / wave-A Receiver in this binding.
    let jdr = unsafe { &*ptr };
    let inner = jdr.inner.clone();
    let sink_error = jdr.sink_error.clone();

    // Lock `inner` for the recv, then drop the guard BEFORE draining sink_error.
    // The block is the `res` initialiser so the `MutexGuard` is dropped at its
    // end — no `&mut env` JNI call below runs while the guard is held.
    let res: Result<Option<DemuxEvent>, DemuxReceiverError> = {
        let mut guard = match inner.lock() {
            Ok(g) => g,
            Err(_) => {
                throw_rtp(&mut env, "TRANSPORT", "DemuxReceiver lock poisoned");
                return JObject::null().into_raw();
            }
        };
        match guard.as_mut() {
            Some(rx) => rx.recv_event(),
            None => {
                // Closed concurrently — clean end of iteration.
                return JObject::null().into_raw();
            }
        }
        // guard dropped here at end of block
    };

    // Fail-loud: surface any byte-sink exception captured during this recv_event
    // BEFORE inspecting `res`. `take()` so a resumed iteration isn't poisoned.
    let captured = sink_error.lock().ok().and_then(|mut s| s.take());
    if let Some(global) = captured {
        if let Ok(local) = env.new_local_ref(&global) {
            let _ = env.throw(JThrowable::from(local));
        }
        return JObject::null().into_raw();
    }

    match res {
        Ok(None) => JObject::null().into_raw(),
        Ok(Some(ev)) => match convert_event(&mut env, &ev) {
            Ok(Some(obj)) => obj.into_raw(),
            // Forward-compat guard (all current variants build a record).
            Ok(None) => JObject::null().into_raw(),
            Err(()) => {
                crate::error::throw_demux(&mut env, "INTERNAL", "event conversion failed");
                JObject::null().into_raw()
            }
        },
        Err(e) => {
            throw_demux_recv_error(&mut env, &e);
            JObject::null().into_raw()
        }
    }
}

/// `nAddByteSink(handle, consumer)` — register a per-188-byte `Consumer<byte[]>`.
/// The closure attaches to the JVM and upcalls with NO Java monitor held; it
/// touches only `consumer` + `sink_error`. See the module doc.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_DemuxReceiver_nAddByteSink<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    consumer: JObject<'local>,
) {
    let Some(ptr) = checked_receiver(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live `Box<JniRtpDemuxReceiver>`.
    let jdr = unsafe { &*ptr };

    let vm = match env.get_java_vm() {
        Ok(vm) => vm,
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return;
        }
    };
    let consumer = match env.new_global_ref(&consumer) {
        Ok(g) => g,
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return;
        }
    };
    let slot = jdr.sink_error.clone();
    let inner = jdr.inner.clone();

    // Lock `inner` to register the sink. If a concurrent `nNext` holds the lock,
    // this blocks until it yields (registration is append-only, no re-entry into
    // the JVM here). Throw if closed/poisoned.
    let mut guard = match inner.lock() {
        Ok(g) => g,
        Err(_) => {
            throw_rtp(&mut env, "TRANSPORT", "DemuxReceiver lock poisoned");
            return;
        }
    };
    let Some(rx) = guard.as_mut() else {
        let _ = env.throw_new("java/lang/IllegalStateException", "DemuxReceiver is closed");
        return;
    };

    rx.add_byte_sink(Box::new(move |pkt: &[u8]| {
        // Runs on the receiver's own thread inside recv_event. NO Java monitor is
        // held across this upcall; touches ONLY `consumer` + `slot`, never `inner`.
        let Ok(mut env) = vm.attach_current_thread() else {
            return;
        };
        let arr = match env.byte_array_from_slice(pkt) {
            Ok(a) => a,
            // OOM-only: clear the pending exception before bailing so the next
            // packet's fanout doesn't run JNI calls under a stale exception.
            Err(_) => {
                let _ = env.exception_clear();
                return;
            }
        };
        let _ = env.call_method(
            &consumer,
            "accept",
            "(Ljava/lang/Object;)V",
            &[JValue::Object(&arr.into())],
        );
        // Fail-loud: capture the first callback exception as a GlobalRef; later
        // per-packet errors are dropped. `nNext` drains + re-throws it.
        if env.exception_check().unwrap_or(false) {
            if let Ok(exc) = env.exception_occurred() {
                let _ = env.exception_clear();
                if let Ok(mut s) = slot.lock() {
                    if s.is_none() {
                        if let Ok(g) = env.new_global_ref(&exc) {
                            *s = Some(g);
                        }
                    }
                }
            }
        }
    }));
}

/// `nStats(handle)` — `(SocketStats, MuxerStats)` projection mirroring tst-py's
/// `DemuxReceiver.stats`. Returns null on a JNI builder error (non-fatal).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_DemuxReceiver_nStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObject<'local> {
    let Some(ptr) = checked_receiver(&mut env, handle) else {
        return JObject::null();
    };
    // SAFETY: validated non-zero live `Box<JniRtpDemuxReceiver>`.
    let jdr = unsafe { &*ptr };
    let combined = {
        let guard = match jdr.inner.lock() {
            Ok(g) => g,
            Err(_) => {
                throw_rtp(&mut env, "TRANSPORT", "DemuxReceiver lock poisoned");
                return JObject::null();
            }
        };
        match guard.as_ref() {
            Some(rx) => rx.stats(),
            None => {
                let _ = env.throw_new("java/lang/IllegalStateException", "DemuxReceiver is closed");
                return JObject::null();
            }
        }
        // guard dropped here at end of block, before any &mut env JNI calls
    };

    // Partial SocketStats from the pipeline-tracked recv counters (full
    // SocketStats via the transport accessor isn't surfaced through the shell).
    // `SocketStats` is non-exhaustive; populate via mut spread.
    let mut sock = tst_core::transport::SocketStats::default();
    sock.bytes_received = combined.bytes_received;
    sock.packets_received = combined.packets_received;

    let sock_obj = match build_socket_stats(&mut env, &sock) {
        Ok(o) => o,
        Err(_) => return JObject::null(),
    };
    // Re-shape the demux side as a MuxerStats projection (mirrors tst-py).
    let mux_obj = match build_muxer_stats(
        &mut env,
        combined.packets_received as i64,
        combined.bytes_received as i64,
        combined.program_maps_seen as i64,
    ) {
        Ok(o) => o,
        Err(_) => return JObject::null(),
    };
    match build_rtp_transport_stats(&mut env, &sock_obj, &mux_obj) {
        Ok(o) => o,
        Err(_) => JObject::null(),
    }
}

/// `nClose(handle)` — cancel-first, then take + drop the inner receiver and free
/// the box. Cancelling before locking wakes a parked `nNext` so this never
/// deadlocks against it. No-op on a zero handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_DemuxReceiver_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: handle from Box::into_raw, dropped once (Java zeroes its field).
        let b = unsafe { Box::from_raw(handle as *mut JniRtpDemuxReceiver) };
        // Step 1: wake any parked recv WITHOUT taking the lock.
        b.cancel.cancel();
        // Step 2: take + close the inner under the lock (waits briefly for the
        // parked recv to release). Lock-poisoned → best-effort no-op (the cancel
        // already woke any parked recv; the box drop frees everything).
        if let Ok(mut guard) = b.inner.lock() {
            if let Some(mut rx) = guard.take() {
                rx.close();
            }
        }
        drop(b);
    }
}

/// `nIsAlive(handle)` — whether the receiver owns a live transport. Uses
/// `try_lock` so a concurrent parked `nNext` reports "alive" rather than blocking.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_DemuxReceiver_nIsAlive(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jboolean {
    if handle == 0 {
        return 0;
    }
    // SAFETY: validated non-zero live `Box<JniRtpDemuxReceiver>`.
    let jdr = unsafe { &*(handle as *const JniRtpDemuxReceiver) };
    match jdr.inner.try_lock() {
        Ok(guard) => match guard.as_ref() {
            Some(rx) => u8::from(rx.is_alive()),
            None => 0,
        },
        // Locked by a parked recv → the receiver is live.
        Err(_) => 1,
    }
}
