//! JNI surface for `org.tstrans.rtp.MuxSender` — the single-call convenience
//! wrapper that owns a `Muxer` + an RTP `RtpTransport`.
//!
//! Wraps `tst_pipeline::MuxSender<tst_rtp::RtpTransport>`. `nFromUrl` builds the
//! `MuxerConfig` from the parallel-array program description (via the shared
//! `crate::mpegts::muxer::build_muxer_config_from_arrays` helper, byte-exact with
//! `Muxer::nOpen`), then builds an `RtpTransport` from the `rtp://` URL + pkt_size
//! and hands transport + config to the pipeline shell. Ports tst-py's
//! `bindings/python/src/rtp/mux_sender.rs`.
//!
//! The handle is a `Box<MuxSender<RtpTransport>>`; per-call methods reconstitute
//! as a SHARED `&*ptr` borrow (every `MuxSender::send_*`/`stats`/`is_alive` takes
//! `&self`, serialising internally via its own `Mutex<Inner>`), so concurrent
//! pushes from multiple Java threads are sound — no aliased `&mut`. `nClose` drops
//! the box.
//!
//! Error mapping mirrors tst-py's `mux_sender_error_to_pyerr`: `Mux(...)` →
//! `MuxException`, `Transport(...)` → `RtpException` per `TransportError` variant,
//! forward-compat catch-all → `RtpException(TRANSPORT)`.

use jni::JNIEnv;
use jni::objects::{JBooleanArray, JByteArray, JClass, JIntArray, JObject, JString, JValue};
use jni::sys::{jboolean, jint, jlong};

use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{
    AudioStreamHandle, KlvStreamHandle, SubtitleStreamHandle, VideoStreamHandle,
};
use tst_pipeline::{MuxSender as RustMuxSender, MuxSenderError, MuxSenderErrorSource};
use tst_rtp::{RtpSocketBuilder, RtpTransport};

use crate::jutil::checked_u8;
use crate::mpegts::muxer::{build_muxer_config_from_arrays, throw_mux_error};

use super::errors::{connect_error_to_rtp, throw_rtp, transport_error_to_rtp};
use super::stats::build_socket_stats;

type Inner = RustMuxSender<RtpTransport>;

/// Map a `MuxSenderError` (from any `send_*`) to a thrown Java exception.
/// `Mux(...)` → `MuxException`; `Transport(...)` → `RtpException` per
/// `TransportError` variant. Mirrors tst-py's `mux_sender_error_to_pyerr`.
fn throw_mux_sender_error(env: &mut JNIEnv, e: &MuxSenderError) {
    match &e.source {
        MuxSenderErrorSource::Mux(m) => throw_mux_error(env, m),
        MuxSenderErrorSource::Transport(t) => transport_error_to_rtp(env, t),
        // `MuxSenderErrorSource` may gain variants; route any future one to a
        // generic RtpException(TRANSPORT) with the Display message preserved.
        _ => throw_rtp(env, "TRANSPORT", &e.to_string()),
    }
}

/// Build a `MuxSender<RtpTransport>` from an `rtp://` URL + pkt_size + a built
/// `MuxerConfig`. Returns the boxed handle as `jlong`, or `0` with a pending
/// exception on any failure.
fn build_from_url(
    env: &mut JNIEnv,
    url: &JString,
    cfg: tst_core::mpegts::mux::MuxerConfig,
    pkt_size: jint,
) -> jlong {
    let url_str: String = match env.get_string(url) {
        Ok(s) => s.into(),
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return 0;
        }
    };

    let mut builder = match RtpSocketBuilder::from_url(&url_str) {
        Ok(b) => b,
        Err(e) => {
            throw_rtp(env, "TRANSPORT", &e.to_string());
            return 0;
        }
    };
    builder.pkt_size(pkt_size.max(0) as usize);
    let transport = match builder.build() {
        Ok(t) => t,
        Err(e) => {
            connect_error_to_rtp(env, &e);
            return 0;
        }
    };

    match RustMuxSender::new(transport, cfg) {
        Ok(sender) => Box::into_raw(Box::new(sender)) as jlong,
        Err(e) => {
            throw_mux_error(env, &e);
            0
        }
    }
}

/// `MuxSender.nFromUrl(url, ...programConfig..., pktSize)` — build the muxer
/// config, build an RTP transport, and return a `Box<MuxSender<RtpTransport>>`.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nFromUrl<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    url: JString<'local>,
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
    pkt_size: jint,
) -> jlong {
    // Build the MuxerConfig FIRST — a pending MuxException is thrown on Err(()).
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
        Err(()) => return 0,
    };
    build_from_url(&mut env, &url, cfg, pkt_size)
}

/// Validate a native handle. Returns the live `*mut Inner`, or throws
/// `IllegalStateException` and returns `None` for a zero (closed) handle.
fn checked_sender(env: &mut JNIEnv, handle: jlong) -> Option<*mut Inner> {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "MuxSender is closed");
        return None;
    }
    Some(handle as *mut Inner)
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

// ── Push family — single-stream variants ───────────────────────────────────

/// `nPushVideo(handle, nal, pts, keyFrame)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nPushVideo<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    nal: JByteArray<'local>,
    pts: jlong,
    key_frame: jboolean,
) {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live `Box<Inner>`; send_video takes &self.
    let inner = unsafe { &*ptr };
    let Some(buf) = read_bytes(&mut env, &nal) else {
        return;
    };
    if let Err(e) = inner.send_video(&buf, Pts90khz::new(pts), key_frame != 0) {
        throw_mux_sender_error(&mut env, &e);
    }
}

/// `nPushKlv(handle, klv, pts, metadataServiceId)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nPushKlv<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    klv: JByteArray<'local>,
    pts: jlong,
    metadata_service_id: jint,
) {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live `Box<Inner>`; send_klv takes &self.
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
    if let Err(e) = inner.send_klv(&buf, Pts90khz::new(pts), service_id) {
        throw_mux_sender_error(&mut env, &e);
    }
}

/// `nPushAudio(handle, frames, pts)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nPushAudio<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    frames: JByteArray<'local>,
    pts: jlong,
) {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live `Box<Inner>`; send_audio takes &self.
    let inner = unsafe { &*ptr };
    let Some(buf) = read_bytes(&mut env, &frames) else {
        return;
    };
    if let Err(e) = inner.send_audio(&buf, Pts90khz::new(pts)) {
        throw_mux_sender_error(&mut env, &e);
    }
}

/// `nPushSubtitle(handle, pts, payload)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nPushSubtitle<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    pts: jlong,
    payload: JByteArray<'local>,
) {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live `Box<Inner>`; send_subtitle takes &self.
    let inner = unsafe { &*ptr };
    let Some(buf) = read_bytes(&mut env, &payload) else {
        return;
    };
    if let Err(e) = inner.send_subtitle(&buf, Pts90khz::new(pts)) {
        throw_mux_sender_error(&mut env, &e);
    }
}

// ── Push family — handle-targeted variants ─────────────────────────────────

/// `nPushVideoTo(handle, streamHandleRaw, nal, pts, keyFrame)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nPushVideoTo<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    stream_handle_raw: jlong,
    nal: JByteArray<'local>,
    pts: jlong,
    key_frame: jboolean,
) {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live `Box<Inner>`; send_video_to takes &self.
    let inner = unsafe { &*ptr };
    let h = match VideoStreamHandle::try_from_raw(stream_handle_raw as u32) {
        Ok(h) => h,
        Err(_) => {
            throw_rtp(&mut env, "TRANSPORT", "invalid stream handle");
            return;
        }
    };
    let Some(buf) = read_bytes(&mut env, &nal) else {
        return;
    };
    if let Err(e) = inner.send_video_to(h, &buf, Pts90khz::new(pts), key_frame != 0) {
        throw_mux_sender_error(&mut env, &e);
    }
}

/// `nPushKlvTo(handle, streamHandleRaw, klv, pts, metadataServiceId)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nPushKlvTo<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    stream_handle_raw: jlong,
    klv: JByteArray<'local>,
    pts: jlong,
    metadata_service_id: jint,
) {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live `Box<Inner>`; send_klv_to takes &self.
    let inner = unsafe { &*ptr };
    let h = match KlvStreamHandle::try_from_raw(stream_handle_raw as u32) {
        Ok(h) => h,
        Err(_) => {
            throw_rtp(&mut env, "TRANSPORT", "invalid stream handle");
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
    if let Err(e) = inner.send_klv_to(h, &buf, Pts90khz::new(pts), service_id) {
        throw_mux_sender_error(&mut env, &e);
    }
}

/// `nPushAudioTo(handle, streamHandleRaw, frames, pts)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nPushAudioTo<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    stream_handle_raw: jlong,
    frames: JByteArray<'local>,
    pts: jlong,
) {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live `Box<Inner>`; send_audio_to takes &self.
    let inner = unsafe { &*ptr };
    let h = match AudioStreamHandle::try_from_raw(stream_handle_raw as u32) {
        Ok(h) => h,
        Err(_) => {
            throw_rtp(&mut env, "TRANSPORT", "invalid stream handle");
            return;
        }
    };
    let Some(buf) = read_bytes(&mut env, &frames) else {
        return;
    };
    if let Err(e) = inner.send_audio_to(h, &buf, Pts90khz::new(pts)) {
        throw_mux_sender_error(&mut env, &e);
    }
}

/// `nPushSubtitleTo(handle, streamHandleRaw, pts, payload)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nPushSubtitleTo<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    stream_handle_raw: jlong,
    pts: jlong,
    payload: JByteArray<'local>,
) {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live `Box<Inner>`; send_subtitle_to takes &self.
    let inner = unsafe { &*ptr };
    let h = match SubtitleStreamHandle::try_from_raw(stream_handle_raw as u32) {
        Ok(h) => h,
        Err(_) => {
            throw_rtp(&mut env, "TRANSPORT", "invalid stream handle");
            return;
        }
    };
    let Some(buf) = read_bytes(&mut env, &payload) else {
        return;
    };
    if let Err(e) = inner.send_subtitle_to(h, &buf, Pts90khz::new(pts)) {
        throw_mux_sender_error(&mut env, &e);
    }
}

// ── Handle getters (first-of-kind; -1 = none) ──────────────────────────────

/// `nVideoHandle(handle)` — first configured video stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nVideoHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return -1;
    };
    // SAFETY: validated non-zero live `Box<Inner>`.
    let inner = unsafe { &*ptr };
    inner
        .video_handles()
        .into_iter()
        .next()
        .map(|h| i64::from(h.raw()))
        .unwrap_or(-1)
}

/// `nKlvHandle(handle)` — first configured KLV stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nKlvHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return -1;
    };
    // SAFETY: validated non-zero live `Box<Inner>`.
    let inner = unsafe { &*ptr };
    inner
        .klv_handles()
        .into_iter()
        .next()
        .map(|h| i64::from(h.raw()))
        .unwrap_or(-1)
}

/// `nAudioHandle(handle)` — first configured audio stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nAudioHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return -1;
    };
    // SAFETY: validated non-zero live `Box<Inner>`.
    let inner = unsafe { &*ptr };
    inner
        .audio_handles()
        .into_iter()
        .next()
        .map(|h| i64::from(h.raw()))
        .unwrap_or(-1)
}

/// `nSubtitleHandle(handle)` — first configured subtitle stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nSubtitleHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return -1;
    };
    // SAFETY: validated non-zero live `Box<Inner>`.
    let inner = unsafe { &*ptr };
    inner
        .subtitle_handles()
        .into_iter()
        .next()
        .map(|h| i64::from(h.raw()))
        .unwrap_or(-1)
}

// ── Stats ──────────────────────────────────────────────────────────────────

/// Build an `org.tstrans.mpegts.MuxerStats` record from the projected pipeline
/// counters. Ctor sig `(JJJJ)V`. `subtitle_streams_configured` is not tracked by
/// the pipeline shell — default it to 0 (mirrors tst-py). Shared with
/// `demux_receiver.rs`.
pub(crate) fn build_muxer_stats<'local>(
    env: &mut JNIEnv<'local>,
    ts_packets_emitted: i64,
    ts_bytes_emitted: i64,
    programs_configured: i64,
) -> jni::errors::Result<JObject<'local>> {
    env.ensure_local_capacity(4)?;
    env.new_object(
        "org/tstrans/mpegts/MuxerStats",
        "(JJJJ)V",
        &[
            JValue::Long(ts_packets_emitted),
            JValue::Long(ts_bytes_emitted),
            JValue::Long(programs_configured),
            JValue::Long(0),
        ],
    )
}

/// Build an `org.tstrans.rtp.TransportStats` record from an rtp `SocketStats` +
/// `MuxerStats` pair. Shared with `demux_receiver.rs`.
pub(crate) fn build_rtp_transport_stats<'local>(
    env: &mut JNIEnv<'local>,
    socket_stats: &JObject<'local>,
    muxer_stats: &JObject<'local>,
) -> jni::errors::Result<JObject<'local>> {
    env.ensure_local_capacity(4)?;
    env.new_object(
        "org/tstrans/rtp/TransportStats",
        "(Lorg/tstrans/rtp/SocketStats;Lorg/tstrans/mpegts/MuxerStats;)V",
        &[JValue::Object(socket_stats), JValue::Object(muxer_stats)],
    )
}

/// `nStats(handle)` — `(SocketStats, MuxerStats)` projection mirroring tst-py's
/// `MuxSender.stats`. Returns null on a JNI builder error (non-fatal).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObject<'local> {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return JObject::null();
    };
    // SAFETY: validated non-zero live `Box<Inner>`.
    let inner = unsafe { &*ptr };
    let sock = inner.socket_stats().unwrap_or_default();
    let pipe = inner.stats();

    let sock_obj = match build_socket_stats(&mut env, &sock) {
        Ok(o) => o,
        Err(_) => return JObject::null(),
    };
    let mux_obj = match build_muxer_stats(
        &mut env,
        pipe.packets_sent as i64,
        pipe.bytes_sent as i64,
        i64::from(pipe.programs_configured),
    ) {
        Ok(o) => o,
        Err(_) => return JObject::null(),
    };
    match build_rtp_transport_stats(&mut env, &sock_obj, &mux_obj) {
        Ok(o) => o,
        Err(_) => JObject::null(),
    }
}

// ── Lifecycle ──────────────────────────────────────────────────────────────

/// `nClose(handle)` — drop the boxed `MuxSender`. No-op on a zero handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: handle from Box::into_raw, dropped once (Java zeroes its field).
        let b = unsafe { Box::from_raw(handle as *mut Inner) };
        b.close();
        drop(b);
    }
}

/// `nIsAlive(handle)` — whether the sender owns a live transport.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_MuxSender_nIsAlive(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jboolean {
    if handle == 0 {
        return 0;
    }
    // SAFETY: validated non-zero live `Box<Inner>`.
    let inner = unsafe { &*(handle as *const Inner) };
    u8::from(inner.is_alive())
}
