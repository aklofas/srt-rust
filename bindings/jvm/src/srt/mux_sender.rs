//! JNI surface for `org.tstrans.srt.MuxSender` — the single-call convenience
//! wrapper that owns a `Muxer` + `SrtTransport`.
//!
//! Wraps `tst_pipeline::MuxSender<tst_srt::SrtTransport>`. `nFromUrl` builds the
//! `MuxerConfig` from the parallel-array program description (via the shared
//! [`crate::mpegts::muxer::build_muxer_config_from_arrays`] helper, byte-exact
//! with `Muxer::nOpen`), then parses the SRT caller-mode URL, connects, and
//! hands the transport + config to the pipeline shell. The push family pushes
//! elementary streams; each call ends in an `SrtTransport::send_bytes` flush.
//!
//! Ports tst-py's `bindings/python/src/srt/mux_sender.rs`. The handle is a
//! `Box<MuxSender<SrtTransport>>` (`Box::into_raw`/`from_raw`); per-call methods
//! reconstitute as `&mut *ptr`; `nClose` drops the box. `Socket::nIntoMuxSender`
//! CONSUMES a `Box<Socket>` (`*Box::from_raw`) and returns a fresh handle.
//!
//! Error mapping mirrors tst-py's `mux_sender_error_to_pyerr`: `Mux(...)` →
//! `MuxException`, `Transport(...)` → `SrtException` per `TransportError`
//! variant, forward-compat catch-all → `SrtException(IO)`.

use jni::JNIEnv;
use jni::objects::{JBooleanArray, JByteArray, JClass, JIntArray, JObject, JString, JValue};
use jni::sys::{jboolean, jint, jlong};

use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{
    AudioStreamHandle, KlvStreamHandle, SubtitleStreamHandle, VideoStreamHandle,
};
use tst_pipeline::{MuxSender as RustMuxSender, MuxSenderError, MuxSenderErrorSource};
use tst_srt::{Socket, SocketConfig, SrtTransport, SrtUrl, url::Mode};

use crate::jutil::checked_u8;
use crate::mpegts::muxer::{build_muxer_config_from_arrays, throw_mux_error};

use super::errors::{connect_error, throw_srt, transport_error};
use super::stats::build_socket_stats;

type Inner = RustMuxSender<SrtTransport>;

/// Map a `MuxSenderError` (from any `send_*`) to a thrown Java exception.
/// `Mux(...)` → `MuxException`; `Transport(...)` → `SrtException` per
/// `TransportError` variant. Mirrors tst-py's `mux_sender_error_to_pyerr`.
fn throw_mux_sender_error(env: &mut JNIEnv, e: &MuxSenderError) {
    match &e.source {
        MuxSenderErrorSource::Mux(m) => throw_mux_error(env, m),
        MuxSenderErrorSource::Transport(t) => transport_error(env, t),
        // `MuxSenderErrorSource` may gain variants; route any future one to a
        // generic SrtException(IO) with the Display message preserved.
        _ => throw_srt(env, "IO", &e.to_string()),
    }
}

/// Join `host:port`, bracketing bare IPv6 literals. Mirror of the
/// low-level helper in `srt/lowlevel.rs`.
fn join_host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Build a `MuxSender<SrtTransport>` from a parsed caller-mode URL + a built
/// `MuxerConfig`: parse the URL, reject non-Caller mode, connect, wrap. Returns
/// the boxed handle as `jlong`, or `0` with a pending exception on any failure.
/// Shared by `nFromUrl` (where the socket is created here).
fn build_from_url(
    env: &mut JNIEnv,
    url: &JString,
    cfg: tst_core::mpegts::mux::MuxerConfig,
) -> jlong {
    let url_str: String = match env.get_string(url) {
        Ok(s) => s.into(),
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return 0;
        }
    };

    let parsed = match SrtUrl::parse(&url_str) {
        Ok(p) => p,
        Err(e) => {
            super::errors::url_error(env, &e);
            return 0;
        }
    };

    if parsed.mode != Mode::Caller {
        let msg = format!(
            "MuxSender.fromUrl requires mode=caller (default); got mode={:?}",
            parsed.mode
        );
        throw_srt(env, "CONFIG_INVALID", &msg);
        return 0;
    }

    let mut sock_cfg = SocketConfig::default();
    parsed.overlay.apply_to_socket(&mut sock_cfg);
    let addr = join_host_port(&parsed.host, parsed.port);

    let socket = match Socket::connect_with(&sock_cfg, addr.as_str()) {
        Ok(s) => s,
        Err(e) => {
            connect_error(env, &e);
            return 0;
        }
    };

    match RustMuxSender::new(SrtTransport::new(socket), cfg) {
        Ok(sender) => Box::into_raw(Box::new(sender)) as jlong,
        Err(e) => {
            throw_mux_error(env, &e);
            0
        }
    }
}

/// `MuxSender.nFromUrl(url, ...programConfig...)` — build the muxer config,
/// connect a caller-mode SRT socket, and return a `Box<MuxSender>` handle.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nFromUrl<'local>(
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
    build_from_url(&mut env, &url, cfg)
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

/// `nPushVideo(handle, nal, pts, keyFrame)` — push one video access unit.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nPushVideo<'local>(
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
    // SAFETY: validated non-zero live `Box<Inner>` pointer from nFromUrl.
    let inner = unsafe { &mut *ptr };
    let Some(buf) = read_bytes(&mut env, &nal) else {
        return;
    };
    if let Err(e) = inner.send_video(&buf, Pts90khz::new(pts), key_frame != 0) {
        throw_mux_sender_error(&mut env, &e);
    }
}

/// `nPushKlv(handle, klv, pts, metadataServiceId)` — push one KLV blob. The
/// muxer auto-wraps the AU-cell header for synchronous-metadata streams; the
/// caller passes raw KLV LS bytes.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nPushKlv<'local>(
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
    // SAFETY: validated non-zero live `Box<Inner>` pointer from nFromUrl.
    let inner = unsafe { &mut *ptr };
    // `metadata_service_id` is `u8`; range-check before narrowing.
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

/// `nPushAudio(handle, frames, pts)` — push one encoded audio frame.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nPushAudio<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    frames: JByteArray<'local>,
    pts: jlong,
) {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live `Box<Inner>` pointer from nFromUrl.
    let inner = unsafe { &mut *ptr };
    let Some(buf) = read_bytes(&mut env, &frames) else {
        return;
    };
    if let Err(e) = inner.send_audio(&buf, Pts90khz::new(pts)) {
        throw_mux_sender_error(&mut env, &e);
    }
}

/// `nPushSubtitle(handle, pts, payload)` — push one subtitle access unit.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nPushSubtitle<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    pts: jlong,
    payload: JByteArray<'local>,
) {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live `Box<Inner>` pointer from nFromUrl.
    let inner = unsafe { &mut *ptr };
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
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nPushVideoTo<'local>(
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
    // SAFETY: validated non-zero live `Box<Inner>` pointer from nFromUrl.
    let inner = unsafe { &mut *ptr };
    let h = match VideoStreamHandle::try_from_raw(stream_handle_raw as u32) {
        Ok(h) => h,
        Err(_) => {
            throw_srt(&mut env, "CONFIG_INVALID", "invalid stream handle");
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
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nPushKlvTo<'local>(
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
    // SAFETY: validated non-zero live `Box<Inner>` pointer from nFromUrl.
    let inner = unsafe { &mut *ptr };
    let h = match KlvStreamHandle::try_from_raw(stream_handle_raw as u32) {
        Ok(h) => h,
        Err(_) => {
            throw_srt(&mut env, "CONFIG_INVALID", "invalid stream handle");
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
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nPushAudioTo<'local>(
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
    // SAFETY: validated non-zero live `Box<Inner>` pointer from nFromUrl.
    let inner = unsafe { &mut *ptr };
    let h = match AudioStreamHandle::try_from_raw(stream_handle_raw as u32) {
        Ok(h) => h,
        Err(_) => {
            throw_srt(&mut env, "CONFIG_INVALID", "invalid stream handle");
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
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nPushSubtitleTo<'local>(
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
    // SAFETY: validated non-zero live `Box<Inner>` pointer from nFromUrl.
    let inner = unsafe { &mut *ptr };
    let h = match SubtitleStreamHandle::try_from_raw(stream_handle_raw as u32) {
        Ok(h) => h,
        Err(_) => {
            throw_srt(&mut env, "CONFIG_INVALID", "invalid stream handle");
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

// ── Handle getters ─────────────────────────────────────────────────────────
//
// Return the first configured handle of each kind across all programs (which
// for the single-program ctor is also the only program). `-1` = none, which
// the Java side maps to `Optional.empty()`.

/// `nVideoHandle(handle)` — first configured video stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nVideoHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return -1;
    };
    // SAFETY: validated non-zero live `Box<Inner>` pointer from nFromUrl.
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
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nKlvHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return -1;
    };
    // SAFETY: validated non-zero live `Box<Inner>` pointer from nFromUrl.
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
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nAudioHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return -1;
    };
    // SAFETY: validated non-zero live `Box<Inner>` pointer from nFromUrl.
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
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nSubtitleHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return -1;
    };
    // SAFETY: validated non-zero live `Box<Inner>` pointer from nFromUrl.
    let inner = unsafe { &*ptr };
    inner
        .subtitle_handles()
        .into_iter()
        .next()
        .map(|h| i64::from(h.raw()))
        .unwrap_or(-1)
}

// ── Stats ──────────────────────────────────────────────────────────────────

/// Build an `org.tstrans.mpegts.MuxerStats` record from the projected
/// pipeline counters. Ctor sig `(JJJJ)V`. `subtitle_streams_configured` is not
/// tracked by the pipeline shell — default it to 0 (mirrors tst-py).
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

/// Build an `org.tstrans.srt.TransportStats` record from a `SocketStats` +
/// `MuxerStats` pair.
pub(crate) fn build_transport_stats<'local>(
    env: &mut JNIEnv<'local>,
    socket_stats: &JObject<'local>,
    muxer_stats: &JObject<'local>,
) -> jni::errors::Result<JObject<'local>> {
    env.ensure_local_capacity(4)?;
    env.new_object(
        "org/tstrans/srt/TransportStats",
        "(Lorg/tstrans/srt/SocketStats;Lorg/tstrans/mpegts/MuxerStats;)V",
        &[JValue::Object(socket_stats), JValue::Object(muxer_stats)],
    )
}

/// `nStats(handle)` — return a `TransportStats` projecting the SRT socket
/// counters + the muxer's program/packet totals. Returns null on a JNI builder
/// error (non-fatal; mirrors the stats-builder convention in `transport.rs`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> JObject<'local> {
    let Some(ptr) = checked_sender(&mut env, handle) else {
        return JObject::null();
    };
    // SAFETY: validated non-zero live `Box<Inner>` pointer from nFromUrl.
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
    match build_transport_stats(&mut env, &sock_obj, &mux_obj) {
        Ok(o) => o,
        Err(_) => JObject::null(),
    }
}

// ── Lifecycle ──────────────────────────────────────────────────────────────

/// `nClose(handle)` — drop the boxed `MuxSender` (best-effort drain + close).
/// No-op on a zero handle so a double `close()` is safe.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: handle was produced by Box::into_raw and is dropped exactly
        // once (Java zeroes its field after this call).
        let b = unsafe { Box::from_raw(handle as *mut Inner) };
        b.close();
        drop(b);
    }
}

/// `nIsAlive(handle)` — whether the sender owns a live transport.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nIsAlive(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jboolean {
    if handle == 0 {
        return 0;
    }
    // SAFETY: validated non-zero live `Box<Inner>` pointer from nFromUrl.
    let inner = unsafe { &*(handle as *const Inner) };
    u8::from(inner.is_alive())
}

// ---------------------------------------------------------------------------
// Socket — nIntoMuxSender (CONSUMES the Box<Socket>)
// ---------------------------------------------------------------------------

/// `Socket.nIntoMuxSender(handle, ...programConfig...)` — consume a
/// `Box<Socket>` and produce a `Box<MuxSender<SrtTransport>>`. The Java caller
/// zeroes its own socket handle unconditionally after this returns (the socket
/// is consumed even on a config/new error → return 0 with the pending
/// exception).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_org_tstrans_srt_Socket_nIntoMuxSender<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
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
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Socket is closed");
        return 0;
    }
    // SAFETY: handle is a valid Box<Socket> from nConnect/nAccept; consumed once
    // here. The Java caller zeroes its own field so no double-free occurs.
    let socket: Socket = *unsafe { Box::from_raw(handle as *mut Socket) };

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
        Err(()) => {
            // Socket already consumed (dropped at scope end) — pending exception.
            drop(socket);
            return 0;
        }
    };

    match RustMuxSender::new(SrtTransport::new(socket), cfg) {
        Ok(sender) => Box::into_raw(Box::new(sender)) as jlong,
        Err(e) => {
            throw_mux_error(&mut env, &e);
            0
        }
    }
}
