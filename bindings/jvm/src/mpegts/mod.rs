//! JNI surface for `org.tstrans.mpegts.Demuxer` — the keystone vertical.
//!
//! Wraps `tst_core::mpegts::demux::Demuxer`, converting each `DemuxEvent` into
//! one of the Java records (`DemuxEvent.ProgramMap` /
//! `DemuxEvent.Video` / `DemuxEvent.Audio` / `DemuxEvent.Subtitle` /
//! `DemuxEvent.UnknownSample` / `DemuxEvent.Metadata` /
//! `DemuxEvent.NonConformant` / `DemuxEvent.Discontinuity` /
//! `DemuxEvent.ReconnectDiscontinuity`) and mapping
//! `DemuxError` to `org.tstrans.DemuxException` via `crate::error::throw_demux`.
//! Mirrors `bindings/python/src/mpegts.rs` (`convert_*`/`demux_error_to_pyerr`)
//! decision-for-decision.
//!
//! Handle convention: the `jlong` is an opaque key into a per-type
//! [`crate::handle::HandleRegistry`] over the [`Demuxer`]; `nOpen`/`nOpenWithConfig`
//! register via `REGISTRY.insert`; per-call fns lease via `REGISTRY.with`
//! (mapping a closed/absent handle to a thrown `IllegalStateException`); `nClose`
//! takes + drops via `REGISTRY.close` (atomic + idempotent, so a double
//! `close()` is UAF/double-free-safe).
//!
//! The sample-record `payload` is a COPIED, Java-owned heap `ByteBuffer` (`ByteBuffer.wrap`
//! over a fresh `byte[]`). The earlier zero-copy direct-buffer over Rust-owned
//! memory was a use-after-free hazard once a consumer retained the buffer past
//! the next pull, and a JDK-17-stable primitive for *defined*-on-stale-read
//! zero-copy does not exist (the real one is FFM `Arena`/`MemorySegment`, stable
//! only in JDK 22+). Real zero-copy is therefore deferred to a JDK-22+ FFM path;
//! the keystone copies, which is unconditionally safe. See the design spec §5.4.

pub mod muxer;

use std::sync::LazyLock;

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JObject, JValue};
use jni::sys::{jboolean, jint, jlong, jobject};

use tst_core::error::DemuxError;
use tst_core::mpegts::au_cell::CellFragmentIndication;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::demux::event::MultiCellAuReason;
use tst_core::mpegts::demux::{
    AudioCodec, DemuxEvent, Demuxer, DiscontinuityKind, MetadataKind, NonConformantIssue,
    SamplePayload, StreamId, StreamKind, SubtitleCodec, VideoCodec, VideoPayload, split_video,
};
use tst_core::mpegts::mux::Av1CarriageMode;

use crate::codec::aac::build_adts_frame;
use crate::codec::mpegaudio::build_mpeg2_audio_frame;
use crate::codec::shared::{build_nal_unit, build_obu};
use crate::error::{build_codec_exception, throw_demux};
use crate::handle::HandleRegistry;

/// Per-type leased-handle registry for `org.tstrans.mpegts.Demuxer`.
static REGISTRY: LazyLock<HandleRegistry<Demuxer>> = LazyLock::new(HandleRegistry::new);

/// `org.tstrans.mpegts.Demuxer.nOpen()` — allocate a [`Demuxer`] and hand the JVM
/// its raw pointer as a `jlong` handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Demuxer_nOpen<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |_env| REGISTRY.insert(Demuxer::new()) as jlong)
}

/// `nOpenWithConfig(...)` — build a configured [`Demuxer`]. The `strict`/`av1`
/// ints are the Java enum ORDINALS (contract: must mirror the Java enum
/// declaration order — `StrictMode`: 0=OFF,1=TIMING_ONLY,2=PSI_ONLY,3=FULL;
/// `Av1CarriageMode`: 0=MPEG2_TS_BINDING,1=INTEROP_RAW_OBU). A `0` cap means
/// "use the Rust default" (mapped to `None`). Mirrors tst-py's
/// `build_demuxer_config` field-by-field.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Demuxer_nOpenWithConfig<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    strict: jint,
    pes_cap_per_pid: jlong,
    pes_cap_total: jlong,
    cfi: jboolean,
    av1: jint,
    au_cell_cap: jlong,
    lenient_psi: jboolean,
) -> jlong {
    crate::panic::jni_catch(&mut env, 0, |_env| {
        let opts = build_demux_config_from_args(
            strict,
            pes_cap_per_pid,
            pes_cap_total,
            cfi,
            av1,
            au_cell_cap,
            lenient_psi,
        );
        REGISTRY.insert(Demuxer::with_config(opts)) as jlong
    })
}

/// Assemble a `tst_core` [`DemuxerConfig`] from the 7 marshalled JNI primitives
/// (the `nOpenWithConfig` arg shape). The `strict`/`av1` ints are the Java enum
/// ORDINALS (contract: must mirror the Java enum declaration order —
/// `StrictMode`: 0=OFF,1=TIMING_ONLY,2=PSI_ONLY,3=FULL; `Av1CarriageMode`:
/// 0=MPEG2_TS_BINDING,1=INTEROP_RAW_OBU). A `0` cap means "use the Rust default"
/// (mapped to `None`). Mirrors tst-py's `build_demuxer_config` field-by-field.
///
/// Shared by `nOpenWithConfig` and the srt `DemuxReceiver.nFromUrlWithConfig` /
/// `Socket.nIntoDemuxReceiverWithConfig` paths so the config assembly is DRY.
pub(crate) fn build_demux_config_from_args(
    strict: jint,
    pes_cap_per_pid: jlong,
    pes_cap_total: jlong,
    cfi: jboolean,
    av1: jint,
    au_cell_cap: jlong,
    lenient_psi: jboolean,
) -> tst_core::mpegts::demux::DemuxerConfig {
    use tst_core::mpegts::demux::{DemuxerConfig, StrictMode};

    // `DemuxerConfig` is non-exhaustive in `tst_core`, so it can't be built with
    // struct-expression syntax from this crate — assemble it field-by-field on
    // top of `default()`, mirroring tst-py's `build_demuxer_config`. The Rust-only
    // `klv_link_overrides`/`stream_kind_overrides` keep their defaults (deferred).
    let mut opts = DemuxerConfig::default();
    opts.strict = match strict {
        0 => StrictMode::Off,
        1 => StrictMode::TimingOnly,
        2 => StrictMode::DescriptorsOnly, // Java PSI_ONLY
        _ => StrictMode::Full,            // 3 (and any out-of-range → strictest, safe)
    };
    opts.av1_carriage = match av1 {
        1 => Av1CarriageMode::InteropRawObu,
        _ => Av1CarriageMode::Mpeg2TsBinding,
    };
    if pes_cap_per_pid > 0 {
        opts.pes_cap_per_pid = Some(pes_cap_per_pid as usize);
    }
    if pes_cap_total > 0 {
        opts.pes_cap_total = Some(pes_cap_total as usize);
    }
    if au_cell_cap > 0 {
        opts.au_cell_cap_per_pid = Some(au_cell_cap as usize);
    }
    opts.cfi_tolerance = cfi != 0;
    opts.lenient_psi_reassembly = lenient_psi != 0;
    opts
}

/// `nClose(handle)` — take + drop the registered [`Demuxer`]. Atomic +
/// idempotent via `REGISTRY.close`, so a double `close()` is
/// UAF/double-free-safe. The demuxer's teardown is a plain drop (no
/// flush/finalize).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Demuxer_nClose<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) {
    crate::panic::jni_catch(&mut env, (), |_env| {
        // The winning close gets the demuxer back; it has no extra teardown, so just
        // let it drop here.
        let _ = REGISTRY.close(handle as u64);
    })
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
    crate::panic::jni_catch(&mut env, (), |env| {
        let buf = match env.convert_byte_array(&bytes) {
            Ok(b) => b,
            Err(_) => {
                throw_demux(env, "INTERNAL", "failed to read byte[] argument");
                return;
            }
        };

        match REGISTRY.with(handle as u64, |dx| dx.feed(&buf)) {
            Some(Ok(())) => {}
            Some(Err(e)) => throw_demux_error(env, &e),
            None => closed(env),
        }
    })
}

/// Map a `tst_core` [`DemuxError`] to a thrown `org.tstrans.DemuxException`,
/// mirroring tst-py's `demux_error_to_pyerr` exactly. The discriminant MUST be a
/// string literal as the 2nd arg to `throw_demux` (ratchet contract — the `java
/// demux` error-mapping rail greps the whole tree, so these literals living here
/// rather than at the `nFeed` call site keeps coverage intact).
///
/// Shared by `nFeed` and the srt `DemuxReceiver.nNext` demux-error arm.
pub(crate) fn throw_demux_error(env: &mut JNIEnv, e: &DemuxError) {
    match e {
        DemuxError::SyncBufExhausted { .. } => throw_demux(env, "SYNC_LOSS", &e.to_string()),
        DemuxError::MalformedPsi { .. } => throw_demux(env, "BAD_PMT", &e.to_string()),
        DemuxError::MalformedPes { .. } => throw_demux(env, "BAD_PES", &e.to_string()),
        DemuxError::StrictRejection(_) => throw_demux(env, "STRICT_REJECTION", &e.to_string()),
        DemuxError::Unrecoverable { .. } => throw_demux(env, "INTERNAL", &e.to_string()),
        // DemuxError is marked non-exhaustive; forward-compat catch-all.
        _ => throw_demux(env, "INTERNAL", &e.to_string()),
    }
}

/// `nFlush(handle)` — flush in-flight PES reassembly (call once at EOF).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Demuxer_nFlush<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) {
    crate::panic::jni_catch(&mut env, (), |env| {
        if REGISTRY.with(handle as u64, |dx| dx.flush()).is_none() {
            closed(env);
        }
    })
}

/// `nNextEvent(handle)` — pull the next event, converting it to a Java
/// `DemuxEvent` record. Every current `DemuxEvent` variant maps to a record, so
/// the loop returns the first event pulled; the `Ok(None) => continue` arm is a
/// retained forward-compat guard (currently unreachable). Returns Java `null`
/// when the queue drains.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Demuxer_nNextEvent<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) -> jobject {
    crate::panic::jni_catch(&mut env, std::ptr::null_mut(), |env| {
        // Lease + drive the pull loop under the resource lock. `with` runs the
        // closure synchronously, so capturing `env` (`&mut JNIEnv`) to build the
        // Java record in-place is sound. `None` (closed/absent) → IllegalStateException.
        let result = REGISTRY.with(handle as u64, |dx| {
            loop {
                let Some(ev) = dx.next_event() else {
                    return JObject::null().into_raw();
                };
                match convert_event(env, &ev) {
                    Ok(Some(obj)) => return obj.into_raw(),
                    // All current `DemuxEvent` variants map to `Ok(Some(..))`, so this
                    // branch is currently unreachable; retained as a forward-compat
                    // guard should a future skip-worthy variant appear.
                    Ok(None) => continue,
                    Err(()) => {
                        throw_demux(env, "INTERNAL", "event conversion failed");
                        return JObject::null().into_raw();
                    }
                }
            }
        });
        match result {
            Some(obj) => obj,
            None => {
                closed(env);
                JObject::null().into_raw()
            }
        }
    })
}

/// Throw `IllegalStateException` for a leased call that found a closed/absent
/// handle — the native-side enforcement of the same closed-handle contract the
/// Java `ensureOpen()` already checks, so the JNI boundary fails closed even if a
/// private native method is reached by reflection.
fn closed(env: &mut JNIEnv) {
    let _ = env.throw_new("java/lang/IllegalStateException", "Demuxer is closed");
}

/// Convert one `DemuxEvent` to a Java `DemuxEvent` record.
///
/// `DemuxEvent` is not marked non-exhaustive, so this match is exhaustive and
/// every variant currently builds a record: `Ok(Some(obj))`. The `Ok(None)`
/// "skip this event" channel is retained in the return type as a forward-compat
/// guard (see `nNextEvent`) but is not produced today. `Err(())` — a JNI call
/// failed.
pub(crate) fn convert_event<'local>(
    env: &mut JNIEnv<'local>,
    ev: &DemuxEvent,
) -> Result<Option<JObject<'local>>, ()> {
    match ev {
        DemuxEvent::ProgramMap(pm) => {
            let pids = build_pid_list(env, pm.streams.iter().map(|s| s.pid))?;
            let obj = env
                .new_object(
                    "org/tstrans/mpegts/DemuxEvent$ProgramMap",
                    "(IIILjava/util/List;)V",
                    &[
                        JValue::Int(pm.program_number as i32),
                        JValue::Int(pm.pcr_pid as i32),
                        JValue::Int(pm.pmt_pid as i32),
                        JValue::Object(&pids),
                    ],
                )
                .map_err(|_| ())?;
            Ok(Some(obj))
        }
        DemuxEvent::Sample {
            stream,
            pts,
            dts,
            payload,
        } => {
            // Typed-payload sample records. Mirrors tst-py's `convert_sample_event`
            // decision-for-decision: video → typed NAL/OBU lists; audio → typed
            // frame lists on a clean parse, raw bytes + a `CodecParseException`
            // on a mid-stream parse failure, raw bytes (silent) for deferred
            // codecs; subtitle/unknown → raw heap `ByteBuffer`.
            let stream_obj = build_stream_id(env, stream)?;
            let pts_ticks = pts.as_ticks();
            let dts_obj = opt_long(env, *dts)?;
            let obj = match payload {
                SamplePayload::Video {
                    codec,
                    raw,
                    random_access_indicator,
                    av1_carriage,
                    ..
                } => {
                    // Raw-first: the demuxer emits the encoded access unit; split
                    // it into NAL/OBU units here via the opt-in `split_video` so
                    // the Java VideoUnit list surface is unchanged. ES-conformance
                    // issues are not surfaced over this binding.
                    let (video_payload, _issues) =
                        split_video(raw, *codec, av1_carriage.unwrap_or_default());
                    let units = build_video_units(env, &video_payload)?;
                    let codec_obj = codec_enum(env, "VideoCodec", video_codec_name(*codec))?;
                    // `raw` parity with tst-py: the exact encoded AU as a heap
                    // (JVM-owned) copy — JDK < 22 forbids direct buffers over
                    // Rust memory.
                    let raw_buf = wrap_heap_byte_buffer(env, raw.as_slice())?;
                    // av1Carriage: Some(mode) → enum constant; None → null (non-AV1).
                    let av1_carriage_obj = match av1_carriage {
                        Some(mode) => enum_const(env, "Av1CarriageMode", av1_carriage_name(*mode))?,
                        None => JObject::null(),
                    };
                    env.new_object(
                        "org/tstrans/mpegts/DemuxEvent$Video",
                        "(Lorg/tstrans/mpegts/StreamId;JLjava/lang/Long;Lorg/tstrans/mpegts/VideoCodec;Ljava/util/List;Ljava/nio/ByteBuffer;ZLorg/tstrans/CodecParseException;Lorg/tstrans/mpegts/Av1CarriageMode;)V",
                        &[
                            JValue::Object(&stream_obj),
                            JValue::Long(pts_ticks),
                            JValue::Object(&dts_obj),
                            JValue::Object(&codec_obj),
                            JValue::Object(&units),
                            JValue::Object(&raw_buf),
                            JValue::Bool(*random_access_indicator as u8),
                            // codec_parse_error: always null for video — the
                            // binding split the NALs/OBUs itself, so typed
                            // payload construction cannot fail at this layer.
                            JValue::Object(&JObject::null()),
                            JValue::Object(&av1_carriage_obj),
                        ],
                    )
                    .map_err(|_| ())?
                }
                SamplePayload::Audio { codec, frames } => {
                    let (typed_list, raw_buf, parse_err) =
                        build_audio_payload(env, *codec, frames.as_slice())?;
                    let codec_obj = codec_enum(env, "AudioCodec", audio_codec_name(*codec))?;
                    env.new_object(
                        "org/tstrans/mpegts/DemuxEvent$Audio",
                        "(Lorg/tstrans/mpegts/StreamId;JLjava/lang/Long;Lorg/tstrans/mpegts/AudioCodec;Ljava/util/List;Ljava/nio/ByteBuffer;Lorg/tstrans/CodecParseException;)V",
                        &[
                            JValue::Object(&stream_obj),
                            JValue::Long(pts_ticks),
                            JValue::Object(&dts_obj),
                            JValue::Object(&codec_obj),
                            JValue::Object(&typed_list),
                            JValue::Object(&raw_buf),
                            JValue::Object(&parse_err),
                        ],
                    )
                    .map_err(|_| ())?
                }
                SamplePayload::Subtitle { codec, payload } => {
                    let buf = wrap_heap_byte_buffer(env, payload.as_slice())?;
                    let codec_obj = codec_enum(env, "SubtitleCodec", subtitle_codec_name(*codec))?;
                    env.new_object(
                        "org/tstrans/mpegts/DemuxEvent$Subtitle",
                        "(Lorg/tstrans/mpegts/StreamId;JLjava/lang/Long;Lorg/tstrans/mpegts/SubtitleCodec;Ljava/nio/ByteBuffer;)V",
                        &[
                            JValue::Object(&stream_obj),
                            JValue::Long(pts_ticks),
                            JValue::Object(&dts_obj),
                            JValue::Object(&codec_obj),
                            JValue::Object(&buf),
                        ],
                    )
                    .map_err(|_| ())?
                }
                SamplePayload::Unknown { stream_type, raw } => {
                    let buf = wrap_heap_byte_buffer(env, raw.as_slice())?;
                    env.new_object(
                        "org/tstrans/mpegts/DemuxEvent$UnknownSample",
                        "(Lorg/tstrans/mpegts/StreamId;JLjava/lang/Long;ILjava/nio/ByteBuffer;)V",
                        &[
                            JValue::Object(&stream_obj),
                            JValue::Long(pts_ticks),
                            JValue::Object(&dts_obj),
                            JValue::Int(stream_type.as_byte() as i32),
                            JValue::Object(&buf),
                        ],
                    )
                    .map_err(|_| ())?
                }
            };
            Ok(Some(obj))
        }
        DemuxEvent::Metadata {
            stream,
            pts,
            kind,
            payload,
        } => {
            let stream_obj = build_stream_id(env, stream)?;
            let (kind_obj, was_reassembled, cell_count) = metadata_kind(env, kind)?;
            // Raw KLV LS bytes (AU-cell header already stripped). Heap-copied,
            // JVM-owned (same safety story as the sample records).
            let buf = wrap_heap_byte_buffer(env, payload)?;
            let obj = env
                .new_object(
                    "org/tstrans/mpegts/DemuxEvent$Metadata",
                    "(Lorg/tstrans/mpegts/StreamId;JLorg/tstrans/mpegts/MetadataKind;Ljava/nio/ByteBuffer;ZI)V",
                    &[
                        JValue::Object(&stream_obj),
                        JValue::Long(pts.as_ticks()),
                        JValue::Object(&kind_obj),
                        JValue::Object(&buf),
                        JValue::Bool(was_reassembled as u8),
                        JValue::Int(cell_count as i32),
                    ],
                )
                .map_err(|_| ())?;
            Ok(Some(obj))
        }
        DemuxEvent::Discontinuity { stream, kind } => {
            let stream_obj = build_stream_id(env, stream)?;
            let kind_obj = discontinuity_kind(env, kind)?;
            let obj = env
                .new_object(
                    "org/tstrans/mpegts/DemuxEvent$Discontinuity",
                    "(Lorg/tstrans/mpegts/StreamId;Lorg/tstrans/mpegts/DiscontinuityKind;)V",
                    &[JValue::Object(&stream_obj), JValue::Object(&kind_obj)],
                )
                .map_err(|_| ())?;
            Ok(Some(obj))
        }
        DemuxEvent::NonConformant { stream, issue } => {
            let stream_obj = build_stream_id(env, stream)?;
            // The human-readable detail (Rust `NonConformantIssue`'s `Display`).
            let issue_str = env.new_string(issue.to_string()).map_err(|_| ())?;
            let kind_obj = nonconformant_kind(env, issue)?;
            // MultiCellAuReason constant (MULTI_CELL_AU only) or Java null.
            let reason_obj = nonconformant_reason(env, issue)?;
            // (observedCfi, treatedAs) CFI constants (CFI_TOLERATED only) or (null, null).
            let (observed_obj, treated_obj) = nonconformant_cfi(env, issue)?;
            let obj = env
                .new_object(
                    "org/tstrans/mpegts/DemuxEvent$NonConformant",
                    "(Lorg/tstrans/mpegts/StreamId;Ljava/lang/String;Lorg/tstrans/mpegts/NonConformantKind;Lorg/tstrans/mpegts/MultiCellAuReason;Lorg/tstrans/mpegts/CellFragmentIndication;Lorg/tstrans/mpegts/CellFragmentIndication;)V",
                    &[
                        JValue::Object(&stream_obj),
                        JValue::Object(&issue_str),
                        JValue::Object(&kind_obj),
                        JValue::Object(&reason_obj),
                        JValue::Object(&observed_obj),
                        JValue::Object(&treated_obj),
                    ],
                )
                .map_err(|_| ())?;
            Ok(Some(obj))
        }
        DemuxEvent::ReconnectDiscontinuity => {
            let obj = env
                .new_object(
                    "org/tstrans/mpegts/DemuxEvent$ReconnectDiscontinuity",
                    "()V",
                    &[],
                )
                .map_err(|_| ())?;
            Ok(Some(obj))
        }
    }
}

/// Resolve a `tst_core` [`DiscontinuityKind`] to its Java
/// `org.tstrans.mpegts.DiscontinuityKind` enum constant. Mirrors tst-py's
/// `DiscontinuityKindTag` mapping. [`DiscontinuityKind`] is not marked
/// non-exhaustive, so this match is exhaustive (no catch-all).
fn discontinuity_kind<'local>(
    env: &mut JNIEnv<'local>,
    kind: &DiscontinuityKind,
) -> Result<JObject<'local>, ()> {
    let name = match kind {
        DiscontinuityKind::ContinuityJump { .. } => "CONTINUITY_JUMP",
        DiscontinuityKind::PesOversize { .. } => "PES_OVERSIZE",
        DiscontinuityKind::PesTotalOversize => "PES_TOTAL_OVERSIZE",
        DiscontinuityKind::AdaptationFieldFlag => "ADAPTATION_FIELD_FLAG",
    };
    enum_const(env, "DiscontinuityKind", name)
}

/// Resolve the `org.tstrans.mpegts.NonConformantKind` enum constant for a
/// `tst_core` [`NonConformantIssue`]. Mirrors tst-py's `non_conformant_kind_name`
/// byte-for-byte: Rust's 30+ issue variants collapse to one of the Java
/// constants; the per-event `issue` string carries the human-readable detail.
/// [`NonConformantIssue`] is not marked non-exhaustive, so this match is
/// exhaustive (no catch-all) — a new Rust variant breaks the build here.
fn nonconformant_kind<'local>(
    env: &mut JNIEnv<'local>,
    issue: &NonConformantIssue,
) -> Result<JObject<'local>, ()> {
    use NonConformantIssue::*;
    let name = match issue {
        StreamTypeMismatchSyncOnAsyncPid | StreamTypeMismatchAsyncOnSyncPid => {
            "STREAM_TYPE_MISMATCH"
        }
        MissingMetadataDescriptor => "MISSING_METADATA_DESCRIPTOR",
        PcrAnomaly { .. } => "PCR_ANOMALY",
        PsiChecksumMismatch { .. } => "PSI_CHECKSUM_MISMATCH",
        PusiMidPes => "PUSI_MID_PES",
        MalformedPes { .. } => "MALFORMED_PES",
        PidReusedAcrossPrograms { .. } => "PID_REUSED_ACROSS_PROGRAMS",
        SubtitleMissingDescriptor { .. } => "SUBTITLE_MISSING_DESCRIPTOR",
        SubtitleDescriptorAmbiguous { .. } => "SUBTITLE_DESCRIPTOR_AMBIGUOUS",
        SubtitleDescriptorMalformed { .. } => "SUBTITLE_DESCRIPTOR_MALFORMED",
        Av1RegistrationMalformed { .. } => "AV1_REGISTRATION_MALFORMED",
        Av1ObuMissingSizeField { .. } => "AV1_OBU_MISSING_SIZE_FIELD",
        Av1TileListNotAllowed { .. } => "AV1_TILE_LIST_NOT_ALLOWED",
        PsiOverlongSection { .. } => "PSI_OVERLONG_SECTION",
        TransportErrorPacket { .. } => "TRANSPORT_ERROR_PACKET",
        DvbSubDataIdentifier { .. } => "DVB_SUB_DATA_IDENTIFIER",
        PtsAnomaly { .. } => "PTS_ANOMALY",
        MissingRequiredPts { .. } => "MISSING_REQUIRED_PTS",
        PesHeaderMalformed { .. } => "PES_HEADER_MALFORMED",
        SubtitleAlignmentMissing { .. } => "SUBTITLE_ALIGNMENT_MISSING",
        PcrMalformed { .. } => "PCR_MALFORMED",
        NalHeader { .. } => "NAL_HEADER",
        Av1ObuHeader { .. } => "AV1_OBU_HEADER",
        LatmFraming { .. } => "LATM_FRAMING",
        PsiCcDiscontinuity { .. } => "PSI_CC_DISCONTINUITY",
        MultiCellAu { .. } => "MULTI_CELL_AU",
        CfiTolerated { .. } => "CFI_TOLERATED",
        PsiMultiSectionUnsupported { .. } => "PSI_MULTI_SECTION_UNSUPPORTED",
        Ac3SyncMissing { .. } => "AC3_SYNC_MISSING",
        Av1WrongStreamId { .. } => "AV1_WRONG_STREAM_ID",
        Av1MissingTsObuFraming { .. } => "AV1_MISSING_TS_OBU_FRAMING",
        Other(_) => "OTHER",
    };
    enum_const(env, "NonConformantKind", name)
}

/// The `org.tstrans.mpegts.MultiCellAuReason` constant for a `MultiCellAu` issue,
/// or Java `null` for every other issue kind. Mirrors tst-py: only `MultiCellAu`
/// surfaces a typed reason.
fn nonconformant_reason<'local>(
    env: &mut JNIEnv<'local>,
    issue: &NonConformantIssue,
) -> Result<JObject<'local>, ()> {
    match issue {
        NonConformantIssue::MultiCellAu { reason, .. } => {
            let name = match reason {
                MultiCellAuReason::Orphan => "ORPHAN",
                MultiCellAuReason::SequenceGap => "SEQUENCE_GAP",
                MultiCellAuReason::ConcurrentFirst => "CONCURRENT_FIRST",
                MultiCellAuReason::Overflow => "OVERFLOW",
                MultiCellAuReason::OverflowTotal => "OVERFLOW_TOTAL",
                MultiCellAuReason::TooManyPids => "TOO_MANY_PIDS",
                // MultiCellAuReason is marked non-exhaustive; default to ORPHAN
                // like tst-py for any future variant.
                _ => "ORPHAN",
            };
            enum_const(env, "MultiCellAuReason", name)
        }
        _ => Ok(JObject::null()),
    }
}

/// The `(observedCfi, treatedAs)` pair of `org.tstrans.mpegts.CellFragmentIndication`
/// constants for a `CfiTolerated` issue, or `(null, null)` for every other issue
/// kind. Mirrors tst-py: only `CfiTolerated` surfaces the typed CFI bits.
fn nonconformant_cfi<'local>(
    env: &mut JNIEnv<'local>,
    issue: &NonConformantIssue,
) -> Result<(JObject<'local>, JObject<'local>), ()> {
    match issue {
        NonConformantIssue::CfiTolerated {
            observed_cfi,
            treated_as,
            ..
        } => Ok((cfi_const(env, *observed_cfi)?, cfi_const(env, *treated_as)?)),
        _ => Ok((JObject::null(), JObject::null())),
    }
}

/// Resolve the `org.tstrans.mpegts.CellFragmentIndication` constant for a
/// `tst_core` [`CellFragmentIndication`]. The enum is not marked non-exhaustive,
/// so this match is exhaustive (no catch-all).
fn cfi_const<'local>(
    env: &mut JNIEnv<'local>,
    cfi: CellFragmentIndication,
) -> Result<JObject<'local>, ()> {
    let name = match cfi {
        CellFragmentIndication::Middle => "MIDDLE",
        CellFragmentIndication::Last => "LAST",
        CellFragmentIndication::First => "FIRST",
        CellFragmentIndication::Complete => "COMPLETE",
    };
    enum_const(env, "CellFragmentIndication", name)
}

/// Fetch an `org.tstrans.mpegts.{class}.{name}` enum constant via
/// `get_static_field`. The `mpegts`-package twin of [`codec_enum`].
fn enum_const<'local>(
    env: &mut JNIEnv<'local>,
    class: &str,
    name: &str,
) -> Result<JObject<'local>, ()> {
    let class_path = format!("org/tstrans/mpegts/{class}");
    let descriptor = format!("Lorg/tstrans/mpegts/{class};");
    env.get_static_field(&class_path, name, &descriptor)
        .map_err(|_| ())?
        .l()
        .map_err(|_| ())
}

/// Resolve a `tst_core` [`MetadataKind`] to its Java `org.tstrans.mpegts.MetadataKind`
/// enum constant, along with the `(was_reassembled, cell_count)` pair carried on the
/// `DemuxEvent.Metadata` record. Mirrors tst-py's `convert` for the metadata event:
/// async / unknown collapse to `(false, 1)`.
pub(crate) fn metadata_kind<'local>(
    env: &mut JNIEnv<'local>,
    kind: &MetadataKind,
) -> Result<(JObject<'local>, bool, u32), ()> {
    let (name, wr, cc) = match kind {
        MetadataKind::KlvSyncAuCell {
            was_reassembled,
            cell_count,
            ..
        } => ("KLV_SYNC_AU_CELL", *was_reassembled, *cell_count),
        MetadataKind::KlvAsync => ("KLV_ASYNC", false, 1),
        MetadataKind::Unknown(_) => ("UNKNOWN", false, 1),
    };
    let obj = env
        .get_static_field(
            "org/tstrans/mpegts/MetadataKind",
            name,
            "Lorg/tstrans/mpegts/MetadataKind;",
        )
        .map_err(|_| ())?
        .l()
        .map_err(|_| ())?;
    Ok((obj, wr, cc))
}

/// Box an `Option<Pts90khz>` as a `java.lang.Long` (`Long.valueOf`) or Java
/// `null`. Used for the nullable `dts` field of every sample record.
pub(crate) fn opt_long<'local>(
    env: &mut JNIEnv<'local>,
    v: Option<Pts90khz>,
) -> Result<JObject<'local>, ()> {
    match v {
        Some(p) => env
            .call_static_method(
                "java/lang/Long",
                "valueOf",
                "(J)Ljava/lang/Long;",
                &[JValue::Long(p.as_ticks())],
            )
            .map_err(|_| ())?
            .l()
            .map_err(|_| ()),
        None => Ok(JObject::null()),
    }
}

/// Copy `bytes` into a fresh Java `byte[]` and wrap it as a heap `ByteBuffer`
/// (`java.nio.ByteBuffer.wrap`). The returned buffer is backed by JVM-owned
/// memory, so it is safe to retain past the next pull / after `close()`.
pub(crate) fn wrap_heap_byte_buffer<'local>(
    env: &mut JNIEnv<'local>,
    bytes: &[u8],
) -> Result<JObject<'local>, ()> {
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

/// Build a `java.util.List<VideoUnit>` from a `VideoPayload`: `List<NalUnit>`
/// for NAL-shaped codecs (H.264/H.265/H.266), `List<Obu>` for AV1. Each unit is
/// constructed inside a per-element local frame so its refs are reclaimed —
/// AU unit counts are unbounded, so a flat loop would risk local-ref-table
/// overflow. Mirrors tst-py's `convert_sample_event` video arm.
pub(crate) fn build_video_units<'local>(
    env: &mut JNIEnv<'local>,
    payload: &VideoPayload,
) -> Result<JObject<'local>, ()> {
    let list = env
        .new_object("java/util/ArrayList", "()V", &[])
        .map_err(|_| ())?;
    match payload {
        VideoPayload::Nals(nals) => {
            for nal in nals {
                env.with_local_frame(16, |inner| {
                    let val = build_nal_unit(inner, nal)
                        .map_err(|()| jni::errors::Error::JavaException)?;
                    inner.call_method(
                        &list,
                        "add",
                        "(Ljava/lang/Object;)Z",
                        &[JValue::Object(&val)],
                    )?;
                    Ok::<(), jni::errors::Error>(())
                })
                .map_err(|_| ())?;
            }
        }
        VideoPayload::Obus(obus) => {
            for obu in obus {
                env.with_local_frame(16, |inner| {
                    let val =
                        build_obu(inner, obu).map_err(|()| jni::errors::Error::JavaException)?;
                    inner.call_method(
                        &list,
                        "add",
                        "(Ljava/lang/Object;)Z",
                        &[JValue::Object(&val)],
                    )?;
                    Ok::<(), jni::errors::Error>(())
                })
                .map_err(|_| ())?;
            }
        }
    }
    Ok(list)
}

/// Build the audio payload triple for a `DemuxEvent.Audio` record:
/// `(typedList, rawPayload, codecParseError)`. Mirrors tst-py's
/// `convert_sample_event` audio arm EXACTLY:
///
/// * AAC / MP2 clean parse (every `frames_with_resync` item `Ok`) →
///   `(List<AudioFrame>, null, null)`.
/// * AAC / MP2 mid-stream parse failure (first `Err`) →
///   `(empty List, ByteBuffer raw, CodecParseException)` with codec label
///   `"aac"` / `"mp2"`.
/// * AAC-LATM / AC-3 / other (typed parse deferred) →
///   `(empty List, ByteBuffer raw, null)` — silent bytes fallback.
///
/// Each typed frame is built inside a per-element local frame (unbounded frame
/// counts per AU). Returns an empty list (never null) on the bytes-fallback
/// paths so the Java `payload` field is always a `List`.
fn build_audio_payload<'local>(
    env: &mut JNIEnv<'local>,
    codec: AudioCodec,
    frames: &[u8],
) -> Result<(JObject<'local>, JObject<'local>, JObject<'local>), ()> {
    use tst_core::codec::aac::frames_with_resync as aac_frames;
    use tst_core::codec::mpegaudio::frames_with_resync as mpegaudio_frames;

    match codec {
        AudioCodec::Aac => {
            // Collect owned frames, early-returning on the first Err (strict —
            // matches tst-py's `for res in aac_frames(..) { Ok => push, Err => return }`).
            let mut owned = Vec::new();
            let mut parse_err = None;
            for res in aac_frames(frames) {
                match res {
                    Ok(f) => owned.push(f.to_owned()),
                    Err(e) => {
                        parse_err = Some(e);
                        break;
                    }
                }
            }
            match parse_err {
                None => {
                    let list = env
                        .new_object("java/util/ArrayList", "()V", &[])
                        .map_err(|_| ())?;
                    for f in &owned {
                        env.with_local_frame(24, |inner| {
                            let val = build_adts_frame(inner, f)
                                .map_err(|()| jni::errors::Error::JavaException)?;
                            inner.call_method(
                                &list,
                                "add",
                                "(Ljava/lang/Object;)Z",
                                &[JValue::Object(&val)],
                            )?;
                            Ok::<(), jni::errors::Error>(())
                        })
                        .map_err(|_| ())?;
                    }
                    Ok((list, JObject::null(), JObject::null()))
                }
                Some(e) => {
                    let list = env
                        .new_object("java/util/ArrayList", "()V", &[])
                        .map_err(|_| ())?;
                    let raw = wrap_heap_byte_buffer(env, frames)?;
                    let exc = build_codec_exception(env, &e, "aac")?;
                    Ok((list, raw, exc))
                }
            }
        }
        AudioCodec::Mp2 => {
            let mut owned = Vec::new();
            let mut parse_err = None;
            for res in mpegaudio_frames(frames) {
                match res {
                    Ok(f) => owned.push(f.to_owned()),
                    Err(e) => {
                        parse_err = Some(e);
                        break;
                    }
                }
            }
            match parse_err {
                None => {
                    let list = env
                        .new_object("java/util/ArrayList", "()V", &[])
                        .map_err(|_| ())?;
                    for f in &owned {
                        env.with_local_frame(24, |inner| {
                            let val = build_mpeg2_audio_frame(inner, f)
                                .map_err(|()| jni::errors::Error::JavaException)?;
                            inner.call_method(
                                &list,
                                "add",
                                "(Ljava/lang/Object;)Z",
                                &[JValue::Object(&val)],
                            )?;
                            Ok::<(), jni::errors::Error>(())
                        })
                        .map_err(|_| ())?;
                    }
                    Ok((list, JObject::null(), JObject::null()))
                }
                Some(e) => {
                    let list = env
                        .new_object("java/util/ArrayList", "()V", &[])
                        .map_err(|_| ())?;
                    let raw = wrap_heap_byte_buffer(env, frames)?;
                    let exc = build_codec_exception(env, &e, "mp2")?;
                    Ok((list, raw, exc))
                }
            }
        }
        // AAC-LATM + AC-3 typed parsing is deferred — silent bytes fallback
        // (empty list, raw bytes, no parse error). Matches tst-py's `_ =>` arm.
        _ => {
            let list = env
                .new_object("java/util/ArrayList", "()V", &[])
                .map_err(|_| ())?;
            let raw = wrap_heap_byte_buffer(env, frames)?;
            Ok((list, raw, JObject::null()))
        }
    }
}

/// Build the Java `org.tstrans.mpegts.StreamId` record from a `tst_core`
/// [`StreamId`], constructing its nested [`StreamKind`] via [`build_stream_kind`].
pub(crate) fn build_stream_id<'local>(
    env: &mut JNIEnv<'local>,
    s: &StreamId,
) -> Result<JObject<'local>, ()> {
    let kind = build_stream_kind(env, &s.kind)?;
    env.new_object(
        "org/tstrans/mpegts/StreamId",
        "(ILorg/tstrans/mpegts/StreamKind;I)V",
        &[
            JValue::Int(s.pid as i32),
            JValue::Object(&kind),
            JValue::Int(s.program_number as i32),
        ],
    )
    .map_err(|_| ())
}

/// Build the sealed `org.tstrans.mpegts.StreamKind` variant matching a `tst_core`
/// [`StreamKind`]. Each arm news up the corresponding nested record (JNI class
/// names use `$`). `KlvSync`'s `declared_link: Option<u16>` boxes to a
/// `java.lang.Integer` or Java `null`.
fn build_stream_kind<'local>(
    env: &mut JNIEnv<'local>,
    kind: &StreamKind,
) -> Result<JObject<'local>, ()> {
    match kind {
        StreamKind::Video(c) => {
            let codec = codec_enum(env, "VideoCodec", video_codec_name(*c))?;
            env.new_object(
                "org/tstrans/mpegts/StreamKind$Video",
                "(Lorg/tstrans/mpegts/VideoCodec;)V",
                &[JValue::Object(&codec)],
            )
            .map_err(|_| ())
        }
        StreamKind::Audio(c) => {
            let codec = codec_enum(env, "AudioCodec", audio_codec_name(*c))?;
            env.new_object(
                "org/tstrans/mpegts/StreamKind$Audio",
                "(Lorg/tstrans/mpegts/AudioCodec;)V",
                &[JValue::Object(&codec)],
            )
            .map_err(|_| ())
        }
        StreamKind::Subtitle(c) => {
            let codec = codec_enum(env, "SubtitleCodec", subtitle_codec_name(*c))?;
            env.new_object(
                "org/tstrans/mpegts/StreamKind$Subtitle",
                "(Lorg/tstrans/mpegts/SubtitleCodec;)V",
                &[JValue::Object(&codec)],
            )
            .map_err(|_| ())
        }
        StreamKind::KlvSync { declared_link } => {
            let boxed = opt_boxed_int(env, *declared_link)?;
            env.new_object(
                "org/tstrans/mpegts/StreamKind$KlvSync",
                "(Ljava/lang/Integer;)V",
                &[JValue::Object(&boxed)],
            )
            .map_err(|_| ())
        }
        StreamKind::KlvAsync => env
            .new_object("org/tstrans/mpegts/StreamKind$KlvAsync", "()V", &[])
            .map_err(|_| ()),
        StreamKind::Unknown(b) => env
            .new_object(
                "org/tstrans/mpegts/StreamKind$Unknown",
                "(I)V",
                &[JValue::Int(*b as i32)],
            )
            .map_err(|_| ()),
    }
}

/// Fetch a codec enum constant: `org.tstrans.mpegts.{class}.{name}` via
/// `get_static_field`.
pub(crate) fn codec_enum<'local>(
    env: &mut JNIEnv<'local>,
    class: &str,
    name: &str,
) -> Result<JObject<'local>, ()> {
    enum_const(env, class, name)
}

/// Box an `Option<u16>` as a `java.lang.Integer` (`Integer.valueOf`) or Java
/// `null`. Used for `StreamKind$KlvSync`'s nullable `declaredLink`.
fn opt_boxed_int<'local>(
    env: &mut JNIEnv<'local>,
    value: Option<u16>,
) -> Result<JObject<'local>, ()> {
    match value {
        Some(v) => env
            .call_static_method(
                "java/lang/Integer",
                "valueOf",
                "(I)Ljava/lang/Integer;",
                &[JValue::Int(v as i32)],
            )
            .map_err(|_| ())?
            .l()
            .map_err(|_| ()),
        None => Ok(JObject::null()),
    }
}

/// `VideoCodec` enum-constant name in `org.tstrans.mpegts.VideoCodec`.
pub(crate) fn video_codec_name(c: VideoCodec) -> &'static str {
    match c {
        VideoCodec::H264 => "H264",
        VideoCodec::H265 => "H265",
        VideoCodec::H266 => "H266",
        VideoCodec::Av1 => "AV1",
    }
}

/// `AudioCodec` enum-constant name in `org.tstrans.mpegts.AudioCodec`.
fn audio_codec_name(c: AudioCodec) -> &'static str {
    match c {
        AudioCodec::Mp2 => "MP2",
        AudioCodec::Aac => "AAC",
        AudioCodec::AacLatm => "AAC_LATM",
        AudioCodec::Ac3 => "AC3",
    }
}

/// `SubtitleCodec` enum-constant name in `org.tstrans.mpegts.SubtitleCodec`.
fn subtitle_codec_name(c: SubtitleCodec) -> &'static str {
    match c {
        SubtitleCodec::DvbSubtitling => "DVB_SUBTITLING",
        SubtitleCodec::DvbTeletext => "DVB_TELETEXT",
        SubtitleCodec::Cea708Standalone => "CEA708_STANDALONE",
        SubtitleCodec::WebVttInTs => "WEBVTT_IN_TS",
    }
}

/// `Av1CarriageMode` enum-constant name in `org.tstrans.mpegts.Av1CarriageMode`.
/// `Av1CarriageMode` is non-exhaustive; unknown future variants fall back to
/// `MPEG2_TS_BINDING` (the default binding mode) since the demuxer only ever
/// sets the two real variants today.
fn av1_carriage_name(mode: Av1CarriageMode) -> &'static str {
    match mode {
        Av1CarriageMode::Mpeg2TsBinding => "MPEG2_TS_BINDING",
        Av1CarriageMode::InteropRawObu => "INTEROP_RAW_OBU",
        _ => "MPEG2_TS_BINDING",
    }
}
