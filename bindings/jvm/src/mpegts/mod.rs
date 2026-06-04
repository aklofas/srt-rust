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
//! Handle convention: `nOpen` boxes a [`Demuxer`] and returns the raw pointer as
//! a `jlong`; `nClose` reconstitutes + drops the box (guarded by `handle != 0`);
//! the per-call fns validate the handle ([`checked_handle`] throws
//! `IllegalStateException` on a zero handle) before dereferencing.
//!
//! The sample-record `payload` is a COPIED, Java-owned heap `ByteBuffer` (`ByteBuffer.wrap`
//! over a fresh `byte[]`). The earlier zero-copy direct-buffer over Rust-owned
//! memory was a use-after-free hazard once a consumer retained the buffer past
//! the next pull, and a JDK-17-stable primitive for *defined*-on-stale-read
//! zero-copy does not exist (the real one is FFM `Arena`/`MemorySegment`, stable
//! only in JDK 22+). Real zero-copy is therefore deferred to a JDK-22+ FFM path;
//! the keystone copies, which is unconditionally safe. See the design spec §5.4.

use jni::JNIEnv;
use jni::objects::{JByteArray, JClass, JObject, JValue};
use jni::sys::{jlong, jobject};

use tst_core::error::DemuxError;
use tst_core::mpegts::au_cell::CellFragmentIndication;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::demux::event::MultiCellAuReason;
use tst_core::mpegts::demux::{
    AudioCodec, DemuxEvent, Demuxer, DiscontinuityKind, MetadataKind, NalUnit, NonConformantIssue,
    SamplePayload, StreamId, StreamKind, SubtitleCodec, VideoCodec, VideoPayload,
};

use crate::error::throw_demux;

/// `org.tstrans.mpegts.Demuxer.nOpen()` — allocate a [`Demuxer`] and hand the JVM
/// its raw pointer as a `jlong` handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Demuxer_nOpen<'local>(
    _env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jlong {
    Box::into_raw(Box::new(Demuxer::new())) as jlong
}

/// `nClose(handle)` — drop the boxed [`Demuxer`]. No-op on a zero
/// (already-closed) handle so a double `close()` is safe.
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
    let Some(ptr) = checked_handle(&mut env, handle) else {
        return;
    };
    // SAFETY: `checked_handle` rejected 0; the pointer is a live `Box<Demuxer>`
    // from `nOpen` (single-threaded use per spec §5.5).
    let dx = unsafe { &mut *ptr };

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
            // DemuxError is marked non-exhaustive; forward-compat catch-all.
            _ => throw_demux(&mut env, "INTERNAL", &e.to_string()),
        }
    }
}

/// `nFlush(handle)` — flush in-flight PES reassembly (call once at EOF).
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_mpegts_Demuxer_nFlush<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
) {
    let Some(ptr) = checked_handle(&mut env, handle) else {
        return;
    };
    // SAFETY: validated non-zero live pointer from `nOpen`.
    let dx = unsafe { &mut *ptr };
    dx.flush();
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
    let Some(ptr) = checked_handle(&mut env, handle) else {
        return JObject::null().into_raw();
    };
    // SAFETY: validated non-zero live pointer from `nOpen`.
    let dx = unsafe { &mut *ptr };

    loop {
        let Some(ev) = dx.next_event() else {
            return JObject::null().into_raw();
        };
        match convert_event(&mut env, &ev) {
            Ok(Some(obj)) => return obj.into_raw(),
            // All current `DemuxEvent` variants map to `Ok(Some(..))`, so this
            // branch is currently unreachable; retained as a forward-compat
            // guard should a future skip-worthy variant appear.
            Ok(None) => continue,
            Err(()) => {
                throw_demux(&mut env, "INTERNAL", "event conversion failed");
                return JObject::null().into_raw();
            }
        }
    }
}

/// Validate a native handle. Returns the live `*mut Demuxer`, or throws
/// `IllegalStateException` and returns `None` for a zero (closed) handle. This is
/// the native-side enforcement of the same closed-handle contract the Java
/// `ensureOpen()` already checks — the JNI boundary fails closed even if a
/// private native method is reached by reflection.
fn checked_handle(env: &mut JNIEnv, handle: jlong) -> Option<*mut Demuxer> {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "Demuxer is closed");
        return None;
    }
    Some(handle as *mut Demuxer)
}

/// Convert one `DemuxEvent` to a Java `DemuxEvent` record.
///
/// `DemuxEvent` is not marked non-exhaustive, so this match is exhaustive and
/// every variant currently builds a record: `Ok(Some(obj))`. The `Ok(None)`
/// "skip this event" channel is retained in the return type as a forward-compat
/// guard (see `nNextEvent`) but is not produced today. `Err(())` — a JNI call
/// failed.
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
            dts,
            payload,
        } => {
            // Common pieces shared by every sample record. The payload is a
            // COPIED, Java-owned heap `ByteBuffer` (see the module doc + spec
            // §5.4 for why zero-copy is deferred to a JDK-22+ FFM path).
            let stream_obj = build_stream_id(env, stream)?;
            let pts_ticks = pts.as_ticks();
            let dts_obj = opt_long(env, *dts)?;
            let buf = wrap_heap_byte_buffer(env, &sample_bytes(payload))?;
            let obj = match payload {
                SamplePayload::Video {
                    random_access_indicator,
                    ..
                } => env
                    .new_object(
                        "org/tstrans/mpegts/DemuxEvent$Video",
                        "(Lorg/tstrans/mpegts/StreamId;JLjava/lang/Long;Ljava/nio/ByteBuffer;Z)V",
                        &[
                            JValue::Object(&stream_obj),
                            JValue::Long(pts_ticks),
                            JValue::Object(&dts_obj),
                            JValue::Object(&buf),
                            JValue::Bool(*random_access_indicator as u8),
                        ],
                    )
                    .map_err(|_| ())?,
                SamplePayload::Audio { .. } => env
                    .new_object(
                        "org/tstrans/mpegts/DemuxEvent$Audio",
                        "(Lorg/tstrans/mpegts/StreamId;JLjava/lang/Long;Ljava/nio/ByteBuffer;)V",
                        &[
                            JValue::Object(&stream_obj),
                            JValue::Long(pts_ticks),
                            JValue::Object(&dts_obj),
                            JValue::Object(&buf),
                        ],
                    )
                    .map_err(|_| ())?,
                SamplePayload::Subtitle { .. } => env
                    .new_object(
                        "org/tstrans/mpegts/DemuxEvent$Subtitle",
                        "(Lorg/tstrans/mpegts/StreamId;JLjava/lang/Long;Ljava/nio/ByteBuffer;)V",
                        &[
                            JValue::Object(&stream_obj),
                            JValue::Long(pts_ticks),
                            JValue::Object(&dts_obj),
                            JValue::Object(&buf),
                        ],
                    )
                    .map_err(|_| ())?,
                SamplePayload::Unknown { stream_type, .. } => env
                    .new_object(
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
                    .map_err(|_| ())?,
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
fn metadata_kind<'local>(
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
fn opt_long<'local>(env: &mut JNIEnv<'local>, v: Option<Pts90khz>) -> Result<JObject<'local>, ()> {
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
fn wrap_heap_byte_buffer<'local>(
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

/// Derive a contiguous payload byte vector from a `SamplePayload`.
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

/// Build the Java `org.tstrans.mpegts.StreamId` record from a `tst_core`
/// [`StreamId`], constructing its nested [`StreamKind`] via [`build_stream_kind`].
fn build_stream_id<'local>(env: &mut JNIEnv<'local>, s: &StreamId) -> Result<JObject<'local>, ()> {
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
fn codec_enum<'local>(
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
fn video_codec_name(c: VideoCodec) -> &'static str {
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
