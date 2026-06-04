//! JNI surface for `org.tstrans.mpegts.Muxer` — the offline byte-feeding muxer.
//!
//! Wraps `tst_core::mpegts::mux::Muxer`. `nOpen` marshals the parallel-array
//! config description (one entry per elementary stream, decoded by Java enum
//! ORDINAL) into a [`MuxerConfig`] via the `tst_core` builders, boxes the
//! [`Muxer`], and returns the raw pointer as a `jlong` handle. The push family
//! (`nPushVideo` / `nPushKlv` / `nPushAudio` / `nPushSubtitle`) reads the Java
//! `byte[]`, calls the matching `Muxer::push_*`, and maps any [`MuxError`] to a
//! thrown `org.tstrans.MuxException` via [`throw_mux_error`]. `nPull` drains TS
//! packets into the caller's `byte[]`; `nPending` / `nCapacity` report queue
//! depth; `nClose` reconstitutes + drops the box.
//!
//! The `MuxError` → `MuxException.Kind` mapping mirrors tst-py's
//! `mux_error_to_pyerr` decision-for-decision: it routes via the 5-variant
//! [`MuxSenderErrorKind`] (`MuxError::kind()`), NOT a per-variant inline match.
//!
//! Handle convention + handle-validation (`checked_muxer` throws
//! `IllegalStateException` on a zero handle before dereferencing) mirror the
//! Demuxer JNI in [`super`]. All JNI array-read failures fail closed (a thrown
//! `MuxException`/`RuntimeException` + a Rust default), never an `.unwrap()`
//! panic across the FFI boundary.

use jni::JNIEnv;
use jni::objects::{JBooleanArray, JByteArray, JClass, JIntArray};
use jni::sys::{jboolean, jint, jlong};

use tst_core::error::MuxError;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{
    AudioCodec, Av1CarriageMode, KlvStreamType, Muxer, MuxerConfig, MuxerProgramConfigBuilder,
    SubtitleCodec, VideoCodec,
};

use crate::error::throw_mux;

/// `org.tstrans.mpegts.Muxer.nOpen(...)` — build a configured [`Muxer`] from the
/// parallel-array config description and hand the JVM its raw pointer as a
/// `jlong` handle. Returns `0` (with a pending `MuxException`) on any config or
/// marshalling failure.
///
/// The per-stream arrays (`stream_pids` / `stream_kinds` / `stream_codecs` /
/// `klv_stream_types` / `klv_carries_pts`) are decoded by Java enum ORDINAL —
/// see the `*_codec` / `klv_type` / `av1_mode` mappers below for the exact
/// ordinal → Rust-enum contract.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_org_tstrans_mpegts_Muxer_nOpen<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
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
    // Read the parallel arrays. On ANY JNI read error, fail closed (throw
    // INTERNAL, return 0) rather than panic across the FFI boundary. `n` is the
    // stream count, taken from `stream_pids`.
    let pids = match read_int_array(&mut env, &stream_pids) {
        Some(v) => v,
        None => return 0,
    };
    let n = pids.len();
    let kinds = match read_int_array(&mut env, &stream_kinds) {
        Some(v) => v,
        None => return 0,
    };
    let codecs = match read_int_array(&mut env, &stream_codecs) {
        Some(v) => v,
        None => return 0,
    };
    let klv_types = match read_int_array(&mut env, &klv_stream_types) {
        Some(v) => v,
        None => return 0,
    };
    let carries = match read_boolean_array(&mut env, &klv_carries_pts) {
        Some(v) => v,
        None => return 0,
    };

    let mut prog = MuxerProgramConfigBuilder::new(program_number as u16, pmt_pid as u16);
    for i in 0..n {
        let pid = pids[i] as u16;
        // stream kind code: 0=video, 1=klv, 2=audio, 3=subtitle.
        match kinds[i] {
            0 => {
                prog.add_video(pid, video_codec(codecs[i]));
            }
            1 => {
                prog.add_klv(pid, klv_type(klv_types[i]), carries[i] != 0);
            }
            2 => {
                prog.add_audio(pid, audio_codec(codecs[i]));
            }
            3 => {
                // Subtitle ordinals 2/3 only (CEA-708 / WebVTT). Ordinals 0/1 are
                // the DVB codecs the Java builder already rejects (they need config
                // not exposed in the JVM binding); reject defensively if one slips
                // through.
                match codecs[i] {
                    2 => {
                        prog.add_subtitle(pid, SubtitleCodec::Cea708Standalone);
                    }
                    3 => {
                        prog.add_subtitle(pid, SubtitleCodec::WebVttInTs);
                    }
                    _ => {
                        throw_mux(
                            &mut env,
                            "CONFIG_INVALID",
                            "DVB subtitle codecs need config not exposed in the JVM binding",
                        );
                        return 0;
                    }
                }
            }
            _ => {
                throw_mux(&mut env, "INTERNAL", "unknown stream kind ordinal");
                return 0;
            }
        }
    }

    // `pcr_pid < 0` means "auto-resolve" — leave the builder default; `>= 0`
    // pins the PCR PID explicitly.
    if pcr_pid >= 0 {
        prog.pcr_pid(pcr_pid as u16);
    }

    let mut b = MuxerConfig::builder();
    b.add_program(prog.build());
    b.pcr_interval_ms(pcr_interval_ms as u32);
    b.psi_interval_ms(psi_interval_ms as u32);
    b.buffer_packets(buffer_packets as usize);
    b.av1_carriage(av1_mode(av1_carriage));
    let cfg = match b.build() {
        Ok(c) => c,
        Err(e) => {
            throw_mux_error(&mut env, &e);
            return 0;
        }
    };
    let mux = match Muxer::new(cfg) {
        Ok(m) => m,
        Err(e) => {
            throw_mux_error(&mut env, &e);
            return 0;
        }
    };
    Box::into_raw(Box::new(mux)) as jlong
}

/// `nPushVideo(handle, nal, pts, keyFrame)` — push one Annex-B access unit.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Muxer_nPushVideo<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    nal: JByteArray<'local>,
    pts: jlong,
    key_frame: jboolean,
) {
    let Some(ptr) = checked_muxer(&mut env, handle) else {
        return;
    };
    // SAFETY: `checked_muxer` rejected 0; the pointer is a live `Box<Muxer>`
    // from `nOpen` (single-threaded use per the JNI handle contract).
    let mux = unsafe { &mut *ptr };
    let buf = match env.convert_byte_array(&nal) {
        Ok(b) => b,
        Err(_) => {
            throw_mux(&mut env, "INTERNAL", "failed to read byte[] argument");
            return;
        }
    };
    if let Err(e) = mux.push_video(&buf, Pts90khz::new(pts), key_frame != 0) {
        throw_mux_error(&mut env, &e);
    }
}

/// `nPushKlv(handle, klv, pts, metadataServiceId)` — push one KLV blob. The
/// muxer auto-wraps the AU-cell header for synchronous-metadata streams; the
/// caller passes raw KLV LS bytes.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Muxer_nPushKlv<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    klv: JByteArray<'local>,
    pts: jlong,
    metadata_service_id: jint,
) {
    let Some(ptr) = checked_muxer(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live pointer from `nOpen`.
    let mux = unsafe { &mut *ptr };
    let buf = match env.convert_byte_array(&klv) {
        Ok(b) => b,
        Err(_) => {
            throw_mux(&mut env, "INTERNAL", "failed to read byte[] argument");
            return;
        }
    };
    // `metadata_service_id` is `u8` in `Muxer::push_klv` (spec default 0x00).
    if let Err(e) = mux.push_klv(&buf, Pts90khz::new(pts), metadata_service_id as u8) {
        throw_mux_error(&mut env, &e);
    }
}

/// `nPushAudio(handle, frames, pts)` — push one audio access unit.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Muxer_nPushAudio<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    frames: JByteArray<'local>,
    pts: jlong,
) {
    let Some(ptr) = checked_muxer(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live pointer from `nOpen`.
    let mux = unsafe { &mut *ptr };
    let buf = match env.convert_byte_array(&frames) {
        Ok(b) => b,
        Err(_) => {
            throw_mux(&mut env, "INTERNAL", "failed to read byte[] argument");
            return;
        }
    };
    if let Err(e) = mux.push_audio(&buf, Pts90khz::new(pts)) {
        throw_mux_error(&mut env, &e);
    }
}

/// `nPushSubtitle(handle, pts, payload)` — push one subtitle access unit. Note
/// the `(pts, payload)` arg order on `Muxer::push_subtitle`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Muxer_nPushSubtitle<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    pts: jlong,
    payload: JByteArray<'local>,
) {
    let Some(ptr) = checked_muxer(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live pointer from `nOpen`.
    let mux = unsafe { &mut *ptr };
    let buf = match env.convert_byte_array(&payload) {
        Ok(b) => b,
        Err(_) => {
            throw_mux(&mut env, "INTERNAL", "failed to read byte[] argument");
            return;
        }
    };
    if let Err(e) = mux.push_subtitle(Pts90khz::new(pts), &buf) {
        throw_mux_error(&mut env, &e);
    }
}

/// `nPull(handle, out)` — drain whole TS packets into the caller's `byte[]`,
/// returning the number of bytes written (a multiple of 188; `0` if `out` is
/// under 188 or the queue is empty).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Muxer_nPull<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    out: JByteArray<'local>,
) -> jint {
    let Some(ptr) = checked_muxer(&mut env, handle) else {
        return 0;
    };
    // SAFETY: validated non-zero live pointer from `nOpen`.
    let mux = unsafe { &mut *ptr };

    let out_len = match env.get_array_length(&out) {
        Ok(l) => l as usize,
        Err(_) => {
            throw_mux(&mut env, "INTERNAL", "failed to read byte[] length");
            return 0;
        }
    };
    let mut scratch = vec![0u8; out_len];
    let n = mux.pull(&mut scratch);
    if n == 0 {
        return 0;
    }
    // jbyte = i8; the byte slice has identical layout. Write the first `n`
    // bytes back into the caller's array.
    // SAFETY: `scratch[..n]` is a live, initialized `[u8]` of length `n`; the
    // i8 view aliases the same bytes (identical size/alignment) and is only
    // read by `set_byte_array_region`.
    let i8_view = unsafe { core::slice::from_raw_parts(scratch.as_ptr() as *const i8, n) };
    if env.set_byte_array_region(&out, 0, i8_view).is_err() {
        throw_mux(&mut env, "INTERNAL", "failed to write byte[] result");
        return 0;
    }
    n as jint
}

/// `nPending(handle)` — TS packets currently queued.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Muxer_nPending<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_muxer(&mut env, handle) else {
        return 0;
    };
    // SAFETY: validated non-zero live pointer from `nOpen`.
    let mux = unsafe { &*ptr };
    mux.pending_packets() as jlong
}

/// `nCapacity(handle)` — configured queue capacity in TS packets.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Muxer_nCapacity<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jlong {
    let Some(ptr) = checked_muxer(&mut env, handle) else {
        return 0;
    };
    // SAFETY: validated non-zero live pointer from `nOpen`.
    let mux = unsafe { &*ptr };
    mux.capacity_packets() as jlong
}

/// `nClose(handle)` — drop the boxed [`Muxer`]. No-op on a zero
/// (already-closed) handle so a double `close()` is safe.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Muxer_nClose<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: `handle` was produced by `Box::into_raw` in `nOpen` and is
        // dropped exactly once (Java zeroes its field after this call).
        unsafe {
            drop(Box::from_raw(handle as *mut Muxer));
        }
    }
}

/// Map a `MuxError` to a thrown `org.tstrans.MuxException`, mirroring tst-py's
/// `mux_error_to_pyerr` (route via the 5-variant `MuxSenderErrorKind`). Each
/// inline literal is what the error-mapping ratchet greps for.
fn throw_mux_error(env: &mut JNIEnv, e: &MuxError) {
    use tst_core::error::MuxSenderErrorKind::*;
    let msg = e.to_string();
    match e.kind() {
        InputMalformed => throw_mux(env, "INPUT_MALFORMED", &msg),
        ConfigInvalid => throw_mux(env, "CONFIG_INVALID", &msg),
        InvalidUsage => throw_mux(env, "INVALID_USAGE", &msg),
        Backpressure => throw_mux(env, "BACKPRESSURE", &msg),
        Internal => throw_mux(env, "INTERNAL", &msg),
        // MuxSenderErrorKind is non-exhaustive; forward-compat catch-all.
        _ => throw_mux(env, "INTERNAL", &msg),
    }
}

/// Validate a native handle. Returns the live `*mut Muxer`, or throws
/// `IllegalStateException` and returns `None` for a zero (closed) handle —
/// the native-side enforcement of the Java `ensureOpen()` contract, so the JNI
/// boundary fails closed even if a private native method is reached by
/// reflection.
fn checked_muxer(env: &mut JNIEnv, handle: jlong) -> Option<*mut Muxer> {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Muxer is closed");
        return None;
    }
    Some(handle as *mut Muxer)
}

/// Read a Java `int[]` into a `Vec<i32>`, or throw INTERNAL + return `None` on a
/// JNI read failure.
fn read_int_array(env: &mut JNIEnv, arr: &JIntArray) -> Option<Vec<i32>> {
    let len = match env.get_array_length(arr) {
        Ok(l) => l as usize,
        Err(_) => {
            throw_mux(env, "INTERNAL", "failed to read int[] length");
            return None;
        }
    };
    let mut v = vec![0i32; len];
    if env.get_int_array_region(arr, 0, &mut v).is_err() {
        throw_mux(env, "INTERNAL", "failed to read int[] region");
        return None;
    }
    Some(v)
}

/// Read a Java `boolean[]` into a `Vec<u8>` (`jboolean` = `u8`; `!= 0` is
/// true), or throw INTERNAL + return `None` on a JNI read failure.
fn read_boolean_array(env: &mut JNIEnv, arr: &JBooleanArray) -> Option<Vec<u8>> {
    let len = match env.get_array_length(arr) {
        Ok(l) => l as usize,
        Err(_) => {
            throw_mux(env, "INTERNAL", "failed to read boolean[] length");
            return None;
        }
    };
    let mut v = vec![0u8; len];
    if env.get_boolean_array_region(arr, 0, &mut v).is_err() {
        throw_mux(env, "INTERNAL", "failed to read boolean[] region");
        return None;
    }
    Some(v)
}

/// Video codec ordinal (`stream_codecs[i]` when kind=video) → [`VideoCodec`].
/// Matches the Java `VideoCodec` enum declaration order.
fn video_codec(ordinal: i32) -> VideoCodec {
    match ordinal {
        0 => VideoCodec::H264,
        1 => VideoCodec::H265,
        2 => VideoCodec::H266,
        3 => VideoCodec::Av1,
        // Unreachable: a Java enum ordinal is always in range.
        _ => VideoCodec::H264,
    }
}

/// Audio codec ordinal (`stream_codecs[i]` when kind=audio) → [`AudioCodec`].
/// Matches the Java `AudioCodec` enum declaration order.
fn audio_codec(ordinal: i32) -> AudioCodec {
    match ordinal {
        0 => AudioCodec::Mp2,
        1 => AudioCodec::Aac,
        2 => AudioCodec::AacLatm,
        3 => AudioCodec::Ac3,
        // Unreachable: a Java enum ordinal is always in range.
        _ => AudioCodec::Mp2,
    }
}

/// KLV stream-type ordinal (`klv_stream_types[i]` when kind=klv) →
/// [`KlvStreamType`]. Matches the Java `KlvStreamType` enum declaration order.
fn klv_type(ordinal: i32) -> KlvStreamType {
    match ordinal {
        0 => KlvStreamType::SynchronousMetadata,
        _ => KlvStreamType::PrivateData, // 1 (and any out-of-range → PrivateData).
    }
}

/// `av1Carriage` scalar ordinal → [`Av1CarriageMode`]. Mirrors the Demuxer
/// `nOpenWithConfig` mapping: 1 → InteropRawObu, else Mpeg2TsBinding.
fn av1_mode(ordinal: i32) -> Av1CarriageMode {
    match ordinal {
        1 => Av1CarriageMode::InteropRawObu,
        _ => Av1CarriageMode::Mpeg2TsBinding,
    }
}
