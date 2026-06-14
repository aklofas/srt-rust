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
//! `jlong` key into a per-type [`crate::handle::HandleRegistry`] over the
//! `MuxSender<SrtTransport>`; per-call methods lease via `REGISTRY.with` (every
//! `MuxSender::send_*`/`stats`/`is_alive` takes `&self`, serialising internally
//! via its own `Mutex<Inner>`), so concurrent pushes from multiple Java threads
//! are sound. `nClose` takes + drops via `REGISTRY.close`.
//! `Socket::nIntoMuxSender` CONSUMES a `Socket` (via `REGISTRY_SOCKET.close`) and
//! returns a fresh handle.
//!
//! Error mapping mirrors tst-py's `mux_sender_error_to_pyerr`: `Mux(...)` →
//! `MuxException`, `Transport(...)` → `SrtException` per `TransportError`
//! variant, forward-compat catch-all → `SrtException(IO)`.

use std::sync::LazyLock;

use jni::JNIEnv;
use jni::objects::{JBooleanArray, JByteArray, JClass, JIntArray, JObject, JString, JValue};
use jni::sys::{jboolean, jint, jlong};

use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{
    AudioStreamHandle, DataStreamHandle, KlvStreamHandle, SubtitleStreamHandle, VideoStreamHandle,
};
use tst_pipeline::{MuxSender as RustMuxSender, MuxSenderError, MuxSenderErrorSource};
use tst_srt::{Socket, SocketConfig, SrtTransport, SrtUrl, url::Mode};

use crate::handle::HandleRegistry;
use crate::jutil::checked_u8;
use crate::mpegts::muxer::{build_muxer_config_from_arrays, throw_mux_error};

use super::errors::{connect_error, throw_srt, transport_error};
use super::stats::build_socket_stats;

type Inner = RustMuxSender<SrtTransport>;

/// Per-type leased-handle registry for `org.tstrans.srt.MuxSender`.
static REGISTRY: LazyLock<HandleRegistry<Inner>> = LazyLock::new(HandleRegistry::new);

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
        Ok(sender) => REGISTRY.insert(sender) as jlong,
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
    stream_type_codes: JIntArray<'local>,
    stream_carries_pts: JBooleanArray<'local>,
    data_desc_bytes: JByteArray<'local>,
    data_desc_lens: JIntArray<'local>,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        // Build the MuxerConfig FIRST — a pending MuxException is thrown on Err(()).
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
            Err(()) => return 0,
        };
        build_from_url(env, &url, cfg)
    })
}

/// Lease the sender and run a push op under the resource lock. A closed/absent
/// handle throws `IllegalStateException`; otherwise any `MuxSenderError` from the
/// op is mapped to the right Java exception. `send_*` take `&self`, so the
/// `&mut Inner` from the registry coerces fine.
fn with_push(
    env: &mut JNIEnv,
    handle: jlong,
    op: impl FnOnce(&Inner) -> Result<(), MuxSenderError>,
) {
    match REGISTRY.with(handle as u64, |inner| op(inner)) {
        Some(Ok(())) => {}
        Some(Err(e)) => throw_mux_sender_error(env, &e),
        None => {
            let _ = env.throw_new("java/lang/IllegalStateException", "MuxSender is closed");
        }
    }
}

/// Lease the sender and return the first handle-of-kind (`-1` if none). A closed
/// handle throws `IllegalStateException` and returns `-1`.
fn first_handle(
    env: &mut JNIEnv,
    handle: jlong,
    pick: impl FnOnce(&Inner) -> Option<u32>,
) -> jlong {
    match REGISTRY.with(handle as u64, |inner| pick(inner)) {
        Some(Some(raw)) => i64::from(raw),
        Some(None) => -1,
        None => {
            let _ = env.throw_new("java/lang/IllegalStateException", "MuxSender is closed");
            -1
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
    crate::panic::jni_catch(&mut env, (), |env| {
        let Some(buf) = read_bytes(env, &nal) else {
            return;
        };
        with_push(env, handle, |inner| {
            inner.send_video(&buf, Pts90khz::new(pts), key_frame != 0)
        });
    })
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
    crate::panic::jni_catch(&mut env, (), |env| {
        // `metadata_service_id` is `u8`; range-check before narrowing.
        let Ok(service_id) = checked_u8(env, i64::from(metadata_service_id), "metadataServiceId")
        else {
            return; // IllegalArgumentException pending
        };
        let Some(buf) = read_bytes(env, &klv) else {
            return;
        };
        with_push(env, handle, |inner| {
            inner.send_klv(&buf, Pts90khz::new(pts), service_id)
        });
    })
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
    crate::panic::jni_catch(&mut env, (), |env| {
        let Some(buf) = read_bytes(env, &frames) else {
            return;
        };
        with_push(env, handle, |inner| {
            inner.send_audio(&buf, Pts90khz::new(pts))
        });
    })
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
    crate::panic::jni_catch(&mut env, (), |env| {
        let Some(buf) = read_bytes(env, &payload) else {
            return;
        };
        with_push(env, handle, |inner| {
            inner.send_subtitle(&buf, Pts90khz::new(pts))
        });
    })
}

/// `nPushData(handle, data, pts)` — pass-through push onto the lone configured
/// data stream. No AU-cell wrap, no framing; one push = one PES on stream_id
/// `0xBD` (`private_stream_1`). `pts` is written into the PES header only for
/// `carries_pts` streams but always drives PSI/PCR pacing.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nPushData<'local>(
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
        with_push(env, handle, |inner| {
            inner.send_data(&buf, Pts90khz::new(pts))
        });
    })
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
    crate::panic::jni_catch(&mut env, (), |env| {
        let h = match VideoStreamHandle::try_from_raw(stream_handle_raw as u32) {
            Ok(h) => h,
            Err(_) => {
                throw_srt(env, "CONFIG_INVALID", "invalid stream handle");
                return;
            }
        };
        let Some(buf) = read_bytes(env, &nal) else {
            return;
        };
        with_push(env, handle, |inner| {
            inner.send_video_to(h, &buf, Pts90khz::new(pts), key_frame != 0)
        });
    })
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
    crate::panic::jni_catch(&mut env, (), |env| {
        let h = match KlvStreamHandle::try_from_raw(stream_handle_raw as u32) {
            Ok(h) => h,
            Err(_) => {
                throw_srt(env, "CONFIG_INVALID", "invalid stream handle");
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
        with_push(env, handle, |inner| {
            inner.send_klv_to(h, &buf, Pts90khz::new(pts), service_id)
        });
    })
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
    crate::panic::jni_catch(&mut env, (), |env| {
        let h = match AudioStreamHandle::try_from_raw(stream_handle_raw as u32) {
            Ok(h) => h,
            Err(_) => {
                throw_srt(env, "CONFIG_INVALID", "invalid stream handle");
                return;
            }
        };
        let Some(buf) = read_bytes(env, &frames) else {
            return;
        };
        with_push(env, handle, |inner| {
            inner.send_audio_to(h, &buf, Pts90khz::new(pts))
        });
    })
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
    crate::panic::jni_catch(&mut env, (), |env| {
        let h = match SubtitleStreamHandle::try_from_raw(stream_handle_raw as u32) {
            Ok(h) => h,
            Err(_) => {
                throw_srt(env, "CONFIG_INVALID", "invalid stream handle");
                return;
            }
        };
        let Some(buf) = read_bytes(env, &payload) else {
            return;
        };
        with_push(env, handle, |inner| {
            inner.send_subtitle_to(h, &buf, Pts90khz::new(pts))
        });
    })
}

/// `nPushDataTo(handle, streamHandleRaw, data, pts)`. The raw handle is
/// validated via the strict `u32::try_from` + `DataStreamHandle::try_from_raw`
/// chain (rejecting negative / out-of-u32 values up front rather than
/// truncating, mirroring `Muxer::nPushDataTo`). The older `*To` siblings in
/// this file still decode their handles with the truncating `as u32` cast;
/// hardening them is deliberately deferred.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nPushDataTo<'local>(
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
            throw_srt(env, "CONFIG_INVALID", "invalid stream handle");
            return;
        };
        let Some(buf) = read_bytes(env, &data) else {
            return;
        };
        with_push(env, handle, |inner| {
            inner.send_data_to(h, &buf, Pts90khz::new(pts))
        });
    })
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
    crate::panic::jni_catch(&mut env, 0, |env| {
        first_handle(env, handle, |inner| {
            inner.video_handles().into_iter().next().map(|h| h.raw())
        })
    })
}

/// `nKlvHandle(handle)` — first configured KLV stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nKlvHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        first_handle(env, handle, |inner| {
            inner.klv_handles().into_iter().next().map(|h| h.raw())
        })
    })
}

/// `nAudioHandle(handle)` — first configured audio stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nAudioHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        first_handle(env, handle, |inner| {
            inner.audio_handles().into_iter().next().map(|h| h.raw())
        })
    })
}

/// `nSubtitleHandle(handle)` — first configured subtitle stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nSubtitleHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        first_handle(env, handle, |inner| {
            inner.subtitle_handles().into_iter().next().map(|h| h.raw())
        })
    })
}

/// `nDataHandle(handle)` — first configured data stream handle, or `-1`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nDataHandle(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        first_handle(env, handle, |inner| {
            inner.data_handles().into_iter().next().map(|h| h.raw())
        })
    })
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
    crate::panic::jni_catch(&mut env, JObject::null(), |env| {
        let Some((sock, pipe)) = REGISTRY.with(handle as u64, |inner| {
            (inner.socket_stats().unwrap_or_default(), inner.stats())
        }) else {
            let _ = env.throw_new("java/lang/IllegalStateException", "MuxSender is closed");
            return JObject::null();
        };

        let sock_obj = match build_socket_stats(env, &sock) {
            Ok(o) => o,
            Err(_) => return JObject::null(),
        };
        let mux_obj = match build_muxer_stats(
            env,
            pipe.packets_sent as i64,
            pipe.bytes_sent as i64,
            i64::from(pipe.programs_configured),
        ) {
            Ok(o) => o,
            Err(_) => return JObject::null(),
        };
        match build_transport_stats(env, &sock_obj, &mux_obj) {
            Ok(o) => o,
            Err(_) => JObject::null(),
        }
    })
}

// ── Lifecycle ──────────────────────────────────────────────────────────────

/// `nClose(handle)` — drop the boxed `MuxSender` (best-effort drain + close).
/// No-op on a zero handle so a double `close()` is safe.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nClose(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    crate::panic::jni_catch(&mut env, (), |_env| {
        // Atomic + idempotent: the winning close gets the shell back for teardown.
        if let Some(inner) = REGISTRY.close(handle as u64) {
            inner.close();
        }
    })
}

/// `nIsAlive(handle)` — whether the sender owns a live transport.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_MuxSender_nIsAlive(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jboolean {
    crate::panic::jni_catch(&mut env, 0, |_env| {
        REGISTRY
            .with(handle as u64, |inner| u8::from(inner.is_alive()))
            .unwrap_or(0)
    })
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
    stream_type_codes: JIntArray<'local>,
    stream_carries_pts: JBooleanArray<'local>,
    data_desc_bytes: JByteArray<'local>,
    data_desc_lens: JIntArray<'local>,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |env| {
        // Take the Socket out of its registry (atomic; idempotent).
        let Some(socket) = super::lowlevel::REGISTRY_SOCKET.close(handle as u64) else {
            let _ = env.throw_new("java/lang/IllegalStateException", "Socket is closed");
            return 0;
        };

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
            Err(()) => {
                // Socket already consumed (dropped at scope end) — pending exception.
                drop(socket);
                return 0;
            }
        };

        match RustMuxSender::new(SrtTransport::new(socket), cfg) {
            Ok(sender) => REGISTRY.insert(sender) as jlong,
            Err(e) => {
                throw_mux_error(env, &e);
                0
            }
        }
    })
}
