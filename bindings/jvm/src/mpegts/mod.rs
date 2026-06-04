//! JNI surface for `org.tstrans.mpegts.Demuxer` — the keystone vertical.
//!
//! Wraps `tst_core::mpegts::demux::Demuxer`, converting each `DemuxEvent` into
//! one of the keystone Java records (`DemuxEvent.ProgramMap` /
//! `DemuxEvent.Sample` / `DemuxEvent.Discontinuity`) and mapping `DemuxError`
//! to `org.tstrans.DemuxException` via `crate::error::throw_demux`. Mirrors
//! `bindings/python/src/mpegts.rs` (`convert_*`/`demux_error_to_pyerr`)
//! decision-for-decision.
//!
//! Handle convention: `nOpen` boxes a `Demuxer` and returns the raw pointer as
//! a `jlong`; `nClose` reconstitutes + drops the box (guarded by `handle != 0`);
//! the per-call fns deref `&mut *(handle as *mut Demuxer)`.

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JObject, JValue};
use jni::sys::{jlong, jobject};

use tst_core::error::DemuxError;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, NalUnit, SamplePayload, VideoPayload};

use crate::error::throw_demux;

/// `org.tstrans.mpegts.Demuxer.nOpen()` — allocate a `Demuxer` and hand the JVM
/// its raw pointer as a `jlong` handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Demuxer_nOpen<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jlong {
    Box::into_raw(Box::new(Demuxer::new())) as jlong
}

/// `nClose(handle)` — drop the boxed `Demuxer`. No-op on a zero (already-closed)
/// handle so a double `close()` is safe.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Demuxer_nClose<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: `handle` was produced by `Box::into_raw` in `nOpen` and is
        // dropped exactly once (Java zeroes its field after this call).
        unsafe {
            drop(Box::from_raw(handle as *mut Demuxer));
        }
    }
}

/// `nFeed(handle, bytes)` — read the Java byte array into a Rust buffer and feed
/// it to the demuxer. A `DemuxError` is mapped inline to a thrown
/// `DemuxException` (see the `match` below); the literal discriminant per arm is
/// what the error-mapping ratchet greps for.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Demuxer_nFeed<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    bytes: JByteArray<'local>,
) {
    // SAFETY: `handle` is a live pointer from `nOpen` (Java guards closed=0).
    let dx = unsafe { &mut *(handle as *mut Demuxer) };

    let buf = match env.convert_byte_array(&bytes) {
        Ok(b) => b,
        Err(_) => {
            throw_demux(&mut env, "INTERNAL", "failed to read byte[] argument");
            return;
        }
    };

    if let Err(e) = dx.feed(&buf) {
        // Mirror tst-py's `demux_error_to_pyerr` exactly. The discriminant MUST
        // be a string literal as the 2nd arg to `throw_demux` (ratchet contract).
        match &e {
            DemuxError::SyncBufExhausted { .. } => {
                throw_demux(&mut env, "SYNC_LOSS", &e.to_string())
            }
            DemuxError::MalformedPsi { .. } => throw_demux(&mut env, "BAD_PMT", &e.to_string()),
            DemuxError::MalformedPes { .. } => throw_demux(&mut env, "BAD_PES", &e.to_string()),
            DemuxError::StrictRejection(_) => {
                throw_demux(&mut env, "STRICT_REJECTION", &e.to_string())
            }
            DemuxError::Unrecoverable { .. } => throw_demux(&mut env, "INTERNAL", &e.to_string()),
            // DemuxError is #[non_exhaustive]; forward-compat catch-all.
            _ => throw_demux(&mut env, "INTERNAL", &e.to_string()),
        }
    }
}

/// `nFlush(handle)` — flush in-flight PES reassembly (call once at EOF).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Demuxer_nFlush<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) {
    // SAFETY: live pointer from `nOpen` (Java guards closed=0).
    let dx = unsafe { &mut *(handle as *mut Demuxer) };
    dx.flush();
}

/// `nNextEvent(handle)` — pull the next *mappable* event, converting it to a
/// Java `DemuxEvent` record. Events with no keystone Java mapping (`Metadata`,
/// `NonConformant`, `ReconnectDiscontinuity`) are skipped; the loop pulls the
/// next one. Returns Java `null` when the queue drains.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Demuxer_nNextEvent<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    // SAFETY: live pointer from `nOpen` (Java guards closed=0).
    let dx = unsafe { &mut *(handle as *mut Demuxer) };

    loop {
        let Some(ev) = dx.next_event() else {
            return JObject::null().into_raw();
        };
        match convert_event(&mut env, &ev) {
            Ok(Some(obj)) => return obj.into_raw(),
            // Not a keystone-mappable event — skip it and pull the next.
            Ok(None) => continue,
            Err(()) => {
                throw_demux(&mut env, "INTERNAL", "event conversion failed");
                return JObject::null().into_raw();
            }
        }
    }
}

/// Convert one `DemuxEvent` to a Java `DemuxEvent` record.
///
/// `Ok(Some(obj))` — a keystone variant was built. `Ok(None)` — the event has
/// no keystone mapping (caller should skip). `Err(())` — a JNI call failed.
fn convert_event<'local>(
    env: &mut JNIEnv<'local>,
    ev: &DemuxEvent,
) -> Result<Option<JObject<'local>>, ()> {
    match ev {
        DemuxEvent::ProgramMap(pm) => {
            let pids = build_pid_list(env, pm.streams.iter().map(|s| s.pid))?;
            let obj = env
                .new_object(
                    "org/tstrans/mpegts/DemuxEvent$ProgramMap",
                    "(IILjava/util/List;)V",
                    &[
                        JValue::Int(pm.program_number as i32),
                        JValue::Int(pm.pcr_pid as i32),
                        JValue::Object(&pids),
                    ],
                )
                .map_err(|_| ())?;
            Ok(Some(obj))
        }
        DemuxEvent::Sample {
            stream,
            pts,
            payload,
            ..
        } => {
            let kind = sample_kind(env, payload)?;
            let bytes = sample_bytes(payload);
            let buffer = wrap_byte_buffer(env, &bytes)?;
            let obj = env
                .new_object(
                    "org/tstrans/mpegts/DemuxEvent$Sample",
                    "(IJLorg/tstrans/mpegts/DemuxEvent$SampleKind;Ljava/nio/ByteBuffer;)V",
                    &[
                        JValue::Int(stream.pid as i32),
                        JValue::Long(pts.as_ticks()),
                        JValue::Object(&kind),
                        JValue::Object(&buffer),
                    ],
                )
                .map_err(|_| ())?;
            Ok(Some(obj))
        }
        DemuxEvent::Discontinuity { stream, .. } => {
            let obj = env
                .new_object(
                    "org/tstrans/mpegts/DemuxEvent$Discontinuity",
                    "(I)V",
                    &[JValue::Int(stream.pid as i32)],
                )
                .map_err(|_| ())?;
            Ok(Some(obj))
        }
        // No keystone Java mapping yet (added in the mpegts-completion wave).
        DemuxEvent::Metadata { .. }
        | DemuxEvent::NonConformant { .. }
        | DemuxEvent::ReconnectDiscontinuity => Ok(None),
    }
}

/// Build a `java.util.List<Integer>` (an `ArrayList`) of boxed PIDs.
fn build_pid_list<'local>(
    env: &mut JNIEnv<'local>,
    pids: impl Iterator<Item = u16>,
) -> Result<JObject<'local>, ()> {
    let list = env
        .new_object("java/util/ArrayList", "()V", &[])
        .map_err(|_| ())?;
    // Forward-note: this loop mints per-iteration local refs (the boxed
    // Integer + the static-method result). Fine here — PMT stream counts are
    // tiny and bounded. If this List-building shape is ever reused over an
    // unbounded per-event collection, wrap the body in `env.with_local_frame(..)`
    // (or delete refs per iteration) to avoid local-ref-table overflow.
    for pid in pids {
        let boxed = env
            .call_static_method(
                "java/lang/Integer",
                "valueOf",
                "(I)Ljava/lang/Integer;",
                &[JValue::Int(pid as i32)],
            )
            .map_err(|_| ())?
            .l()
            .map_err(|_| ())?;
        env.call_method(
            &list,
            "add",
            "(Ljava/lang/Object;)Z",
            &[JValue::Object(&boxed)],
        )
        .map_err(|_| ())?;
    }
    Ok(list)
}

/// Look up the `DemuxEvent.SampleKind` enum constant for a `SamplePayload`.
///
/// Note: the `KLV` SampleKind constant is never produced here — KLV arrives as a
/// `DemuxEvent::Metadata` event (skipped this wave), not a `Sample`. `KLV` stays
/// unused until the mpegts-completion wave wires up the Metadata mapping.
fn sample_kind<'local>(
    env: &mut JNIEnv<'local>,
    payload: &SamplePayload,
) -> Result<JObject<'local>, ()> {
    let name = match payload {
        SamplePayload::Video { .. } => "VIDEO",
        SamplePayload::Audio { .. } => "AUDIO",
        SamplePayload::Subtitle { .. } => "SUBTITLE",
        SamplePayload::Unknown { .. } => "OTHER",
    };
    env.get_static_field(
        "org/tstrans/mpegts/DemuxEvent$SampleKind",
        name,
        "Lorg/tstrans/mpegts/DemuxEvent$SampleKind;",
    )
    .map_err(|_| ())?
    .l()
    .map_err(|_| ())
}

/// Derive a contiguous payload byte vector from a `SamplePayload`. NOTE: this is
/// a COPY — Task 1.5 swaps `Sample.payload` to a direct (zero-copy) ByteBuffer.
fn sample_bytes(payload: &SamplePayload) -> Vec<u8> {
    match payload {
        SamplePayload::Video {
            payload: VideoPayload::Nals(nals),
            ..
        } => {
            let mut out = Vec::new();
            for nal in nals {
                out.extend_from_slice(nal_payload(nal));
            }
            out
        }
        SamplePayload::Video {
            payload: VideoPayload::Obus(obus),
            ..
        } => {
            let mut out = Vec::new();
            for obu in obus {
                out.extend_from_slice(&obu.payload);
            }
            out
        }
        SamplePayload::Audio { frames, .. } => frames.clone(),
        SamplePayload::Subtitle { payload, .. } => payload.clone(),
        SamplePayload::Unknown { raw, .. } => raw.clone(),
    }
}

/// The RBSP payload bytes of a `NalUnit`, regardless of codec.
fn nal_payload(nal: &NalUnit) -> &[u8] {
    match nal {
        NalUnit::H264 { payload, .. }
        | NalUnit::H265 { payload, .. }
        | NalUnit::H266 { payload, .. } => payload,
    }
}

/// Build a Java heap `ByteBuffer` over a COPY of `bytes`
/// (`ByteBuffer.wrap(byte[])`). Task 1.5 converts this to a direct buffer.
fn wrap_byte_buffer<'local>(env: &mut JNIEnv<'local>, bytes: &[u8]) -> Result<JObject<'local>, ()> {
    let arr = env.byte_array_from_slice(bytes).map_err(|_| ())?;
    env.call_static_method(
        "java/nio/ByteBuffer",
        "wrap",
        "([B)Ljava/nio/ByteBuffer;",
        &[JValue::Object(&arr)],
    )
    .map_err(|_| ())?
    .l()
    .map_err(|_| ())
}
