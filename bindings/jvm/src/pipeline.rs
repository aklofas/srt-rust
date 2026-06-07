//! JNI bindings for `tst_pipeline::ext::pairing::PairingDemuxer` —
//! the byte-feeding `org.tstrans.pipeline.Pairer`.
//!
//! Wraps the core `PairingDemuxer` (a `Demuxer` + `Pairer` composite):
//! feed raw TS bytes, collect `PairerOutput`s. The owned demuxer means
//! `DemuxEvent`s never round-trip across the boundary — only the
//! projected `VideoSample` / `KlvSample` shapes (and a pass-through
//! `DemuxEvent` for off-PID events) cross. Mirrors
//! `bindings/python/src/pipeline.rs` decision-for-decision, reusing the
//! `crate::mpegts` projection helpers so a `VideoSample` / `KlvSample`
//! is byte-identical to the corresponding `mpegts.Demuxer` projection.
//!
//! Handle convention matches `mod mpegts`: `nOpen`/`nOpenWithConfig` box
//! a [`PairingDemuxer`] and return the raw pointer as a `jlong`; `nClose`
//! reconstitutes + drops the box (guarded on a non-zero handle); the
//! per-call fns validate the handle via [`checked_pairer_handle`] before
//! dereferencing.

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JObject, JValue};
use jni::sys::{jboolean, jint, jlong, jobject};
use std::time::Duration;

use tst_core::mpegts::demux::DemuxerConfig;
use tst_pipeline::ext::pairing::{
    KlvSample, PairerConfig, PairerMode, PairerOutput, PairingDemuxer, PairingDemuxerConfig,
    VideoSample,
};

use crate::error::throw_demux;
use crate::mpegts::{
    build_demux_config_from_args, build_stream_id, build_video_units, codec_enum, convert_event,
    metadata_kind, opt_long, throw_demux_error, video_codec_name, wrap_heap_byte_buffer,
};

/// `nOpen(videoPid, klvPid)` — allocate a default-config [`PairingDemuxer`]
/// and hand the JVM its raw pointer as a `jlong` handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_pipeline_Pairer_nOpen<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    video_pid: jint,
    klv_pid: jint,
) -> jlong {
    let p = Box::new(PairingDemuxer::new(video_pid as u16, klv_pid as u16));
    Box::into_raw(p) as jlong
}

/// `nOpenWithConfig(...)` — build a configured [`PairingDemuxer`]. The
/// `strict`/`av1` ints are the Java enum ORDINALS (same contract as the
/// `mpegts.Demuxer.nOpenWithConfig` path — see
/// [`build_demux_config_from_args`]). When `has_demuxer_config` is false
/// the demuxer half keeps its `DemuxerConfig::default()`.
///
/// `PairingDemuxerConfig` / `PairerConfig` are non-exhaustive in the
/// external `tst-pipeline` crate, so they cannot be struct-literal'd here
/// — they are assembled via `Default::default()` + field assignment,
/// mirroring tst-py's `build_pairing_demuxer`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_pipeline_Pairer_nOpenWithConfig<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    video_pid: jint,
    klv_pid: jint,
    buffered: jboolean,
    max_lag_nanos: jlong,
    tolerance_nanos: jlong,
    max_buffered_klv: jlong,
    max_buffered_video: jlong,
    link_klv_to_video: jboolean,
    has_demuxer_config: jboolean,
    strict: jint,
    pes_cap_per_pid: jlong,
    pes_cap_total: jlong,
    cfi: jboolean,
    av1: jint,
    au_cell_cap: jlong,
    lenient_psi: jboolean,
) -> jlong {
    let mode = if buffered != 0 {
        PairerMode::Buffered {
            max_lag: Duration::from_nanos(max_lag_nanos as u64),
        }
    } else {
        PairerMode::Realtime
    };

    let mut pairer = PairerConfig::default();
    pairer.mode = mode;
    pairer.tolerance = Duration::from_nanos(tolerance_nanos as u64);
    pairer.max_buffered_klv = max_buffered_klv as u64;
    pairer.max_buffered_video = max_buffered_video as u64;
    pairer.link_klv_to_video = link_klv_to_video != 0;

    let demuxer = if has_demuxer_config != 0 {
        build_demux_config_from_args(
            strict,
            pes_cap_per_pid,
            pes_cap_total,
            cfi,
            av1,
            au_cell_cap,
            lenient_psi,
        )
    } else {
        DemuxerConfig::default()
    };

    let mut cfg = PairingDemuxerConfig::default();
    cfg.pairer = pairer;
    cfg.demuxer = demuxer;

    let p = Box::new(PairingDemuxer::with_config(
        video_pid as u16,
        klv_pid as u16,
        cfg,
    ));
    Box::into_raw(p) as jlong
}

/// `nFeed(handle, bytes)` — read the Java byte array, feed it, and return
/// the produced `PairerOutput`s as a `java.util.ArrayList`. A `DemuxError`
/// is mapped to a thrown `org.tstrans.DemuxException` via
/// [`throw_demux_error`].
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_pipeline_Pairer_nFeed<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    bytes: JByteArray<'local>,
) -> jobject {
    let Some(ptr) = checked_pairer_handle(&mut env, handle) else {
        return JObject::null().into_raw();
    };
    // SAFETY: `checked_pairer_handle` rejected 0; the pointer is a live
    // `Box<PairingDemuxer>` from `nOpen`/`nOpenWithConfig` (single-threaded
    // use per the Demuxer contract).
    let pd = unsafe { &mut *ptr };

    let buf = match env.convert_byte_array(&bytes) {
        Ok(b) => b,
        Err(_) => {
            throw_demux(&mut env, "INTERNAL", "failed to read byte[] argument");
            return JObject::null().into_raw();
        }
    };

    let outputs = match pd.feed(&buf) {
        Ok(v) => v,
        Err(e) => {
            throw_demux_error(&mut env, &e);
            return JObject::null().into_raw();
        }
    };

    match build_output_list(&mut env, &outputs) {
        Ok(list) => list.into_raw(),
        Err(()) => JObject::null().into_raw(),
    }
}

/// `nFlush(handle)` — drain end-of-stream state and return the trailing
/// `PairerOutput`s. No-op (empty list) in `Realtime` mode.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_pipeline_Pairer_nFlush<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    let Some(ptr) = checked_pairer_handle(&mut env, handle) else {
        return JObject::null().into_raw();
    };
    // SAFETY: validated non-zero live pointer from `nOpen`/`nOpenWithConfig`.
    let pd = unsafe { &mut *ptr };
    let outputs = pd.flush();
    match build_output_list(&mut env, &outputs) {
        Ok(list) => list.into_raw(),
        Err(()) => JObject::null().into_raw(),
    }
}

/// Build a `java.util.ArrayList<PairerOutput>` from a slice of outputs.
fn build_output_list<'local>(
    env: &mut JNIEnv<'local>,
    outs: &[PairerOutput],
) -> Result<JObject<'local>, ()> {
    let list = env
        .new_object(
            "java/util/ArrayList",
            "(I)V",
            &[JValue::Int(outs.len() as i32)],
        )
        .map_err(|_| ())?;
    for o in outs {
        // `Ok(None)` = skip this output (a forward-compat unmapped event;
        // unreachable today). Mirrors `mpegts::nNextEvent`'s skip behavior so a
        // future skip-worthy variant is dropped from the batch, not failed.
        if let Some(obj) = convert_output(env, o)? {
            env.call_method(
                &list,
                "add",
                "(Ljava/lang/Object;)Z",
                &[JValue::Object(&obj)],
            )
            .map_err(|_| ())?;
        }
    }
    Ok(list)
}

/// Convert one `PairerOutput` to its `org.tstrans.pipeline.PairerOutput.*`
/// sealed-record variant (nested binary names use `$`). `Ok(None)` = skip this
/// output (a forward-compat unmapped `DemuxEvent` in a `PassThrough`; not
/// produced today); `Err(())` = a JNI conversion failed (exception pending).
fn convert_output<'local>(
    env: &mut JNIEnv<'local>,
    o: &PairerOutput,
) -> Result<Option<JObject<'local>>, ()> {
    let obj = match o {
        PairerOutput::Paired { video, klv } => {
            let v = convert_video_sample(env, video)?;
            let k = convert_klv_sample(env, klv)?;
            env.new_object(
                "org/tstrans/pipeline/PairerOutput$Paired",
                "(Lorg/tstrans/pipeline/VideoSample;Lorg/tstrans/pipeline/KlvSample;)V",
                &[JValue::Object(&v), JValue::Object(&k)],
            )
            .map_err(|_| ())?
        }
        PairerOutput::UnpairedVideo(video) => {
            let v = convert_video_sample(env, video)?;
            env.new_object(
                "org/tstrans/pipeline/PairerOutput$UnpairedVideo",
                "(Lorg/tstrans/pipeline/VideoSample;)V",
                &[JValue::Object(&v)],
            )
            .map_err(|_| ())?
        }
        PairerOutput::UnpairedKlv(klv) => {
            let k = convert_klv_sample(env, klv)?;
            env.new_object(
                "org/tstrans/pipeline/PairerOutput$UnpairedKlv",
                "(Lorg/tstrans/pipeline/KlvSample;)V",
                &[JValue::Object(&k)],
            )
            .map_err(|_| ())?
        }
        PairerOutput::PassThrough(ev) => {
            // `convert_event` can in principle signal "skip this event"
            // (`Ok(None)`) — a forward-compat channel no current variant
            // exercises. Propagate the skip so the unmapped event is dropped
            // from the batch (matching `mpegts::nNextEvent`), not failed.
            let Some(ev_obj) = convert_event(env, ev)? else {
                return Ok(None);
            };
            env.new_object(
                "org/tstrans/pipeline/PairerOutput$PassThrough",
                "(Lorg/tstrans/mpegts/DemuxEvent;)V",
                &[JValue::Object(&ev_obj)],
            )
            .map_err(|_| ())?
        }
    };
    Ok(Some(obj))
}

/// Project a `VideoSample` to a `org.tstrans.pipeline.VideoSample` record.
/// Field order matches the Java canonical ctor:
/// `(StreamId, long pts, Long dts, VideoCodec, List<VideoUnit>)`.
fn convert_video_sample<'local>(
    env: &mut JNIEnv<'local>,
    vs: &VideoSample,
) -> Result<JObject<'local>, ()> {
    let stream = build_stream_id(env, &vs.stream)?;
    let dts = opt_long(env, vs.dts)?;
    let codec = codec_enum(env, "VideoCodec", video_codec_name(vs.codec))?;
    let payload = build_video_units(env, &vs.payload)?;
    env.new_object(
        "org/tstrans/pipeline/VideoSample",
        "(Lorg/tstrans/mpegts/StreamId;JLjava/lang/Long;Lorg/tstrans/mpegts/VideoCodec;Ljava/util/List;)V",
        &[
            JValue::Object(&stream),
            JValue::Long(vs.pts.as_ticks()),
            JValue::Object(&dts),
            JValue::Object(&codec),
            JValue::Object(&payload),
        ],
    )
    .map_err(|_| ())
}

/// Project a `KlvSample` to a `org.tstrans.pipeline.KlvSample` record.
/// Field order matches the Java canonical ctor:
/// `(StreamId, long pts, MetadataKind, ByteBuffer)`. Only the
/// [`metadata_kind`] discriminator is used — the `(was_reassembled,
/// cell_count)` pair it also returns is carried on `DemuxEvent.Metadata`,
/// not on the flat `KlvSample` projection.
fn convert_klv_sample<'local>(
    env: &mut JNIEnv<'local>,
    ks: &KlvSample,
) -> Result<JObject<'local>, ()> {
    let stream = build_stream_id(env, &ks.stream)?;
    let (kind, _was_reassembled, _cell_count) = metadata_kind(env, &ks.kind)?;
    let payload = wrap_heap_byte_buffer(env, &ks.payload)?;
    env.new_object(
        "org/tstrans/pipeline/KlvSample",
        "(Lorg/tstrans/mpegts/StreamId;JLorg/tstrans/mpegts/MetadataKind;Ljava/nio/ByteBuffer;)V",
        &[
            JValue::Object(&stream),
            JValue::Long(ks.pts.as_ticks()),
            JValue::Object(&kind),
            JValue::Object(&payload),
        ],
    )
    .map_err(|_| ())
}

/// `nStats(handle)` — snapshot the pairing counters as a
/// `org.tstrans.pipeline.PairerStats` record.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_pipeline_Pairer_nStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    let Some(ptr) = checked_pairer_handle(&mut env, handle) else {
        return JObject::null().into_raw();
    };
    // SAFETY: validated non-zero live pointer; `stats` is `&self`.
    let s = unsafe { &*ptr }.stats();
    match env.new_object(
        "org/tstrans/pipeline/PairerStats",
        "(JJJJ)V",
        &[
            JValue::Long(s.paired as i64),
            JValue::Long(s.unpaired_video as i64),
            JValue::Long(s.unpaired_klv as i64),
            JValue::Long(s.pass_through as i64),
        ],
    ) {
        Ok(o) => o.into_raw(),
        Err(_) => JObject::null().into_raw(),
    }
}

/// `nDemuxerStats(handle)` — snapshot the underlying demuxer counters as a
/// `org.tstrans.mpegts.DemuxerStats` record. The two `u32` fields
/// (`programs_seen`, `subtitle_streams_seen`) widen to `long`, matching
/// the Java record's all-`long` shape.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_pipeline_Pairer_nDemuxerStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    let Some(ptr) = checked_pairer_handle(&mut env, handle) else {
        return JObject::null().into_raw();
    };
    // SAFETY: validated non-zero live pointer; `demuxer_stats` is `&self`.
    let s = unsafe { &*ptr }.demuxer_stats();
    match env.new_object(
        "org/tstrans/mpegts/DemuxerStats",
        "(JJJJJJ)V",
        &[
            JValue::Long(s.program_maps_seen as i64),
            JValue::Long(s.pmt_versions_seen as i64),
            JValue::Long(s.discontinuities as i64),
            JValue::Long(s.nonconformant as i64),
            JValue::Long(s.programs_seen as i64),
            JValue::Long(s.subtitle_streams_seen as i64),
        ],
    ) {
        Ok(o) => o.into_raw(),
        Err(_) => JObject::null().into_raw(),
    }
}

/// `nResetStats(handle)` — zero the pairing counters (not demuxer stats).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_pipeline_Pairer_nResetStats<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) {
    let Some(ptr) = checked_pairer_handle(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live pointer; `reset_stats` is `&mut self`.
    unsafe { &mut *ptr }.reset_stats();
}

/// `nClose(handle)` — drop the boxed [`PairingDemuxer`]. No-op on a zero
/// (already-closed) handle so a double `close()` is safe.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_pipeline_Pairer_nClose<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: produced by `Box::into_raw` in `nOpen`/`nOpenWithConfig`;
        // dropped exactly once (Java zeroes the handle field after).
        unsafe {
            drop(Box::from_raw(handle as *mut PairingDemuxer));
        }
    }
}

/// Validate a native handle. Returns the live `*mut PairingDemuxer`, or
/// throws `IllegalStateException` and returns `None` for a zero (closed)
/// handle — the native-side mirror of the Java `ensureOpen()` check.
fn checked_pairer_handle(env: &mut JNIEnv, handle: jlong) -> Option<*mut PairingDemuxer> {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Pairer is closed");
        return None;
    }
    Some(handle as *mut PairingDemuxer)
}
