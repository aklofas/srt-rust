//! PES reassembly dispatch + complete-PES-to-DemuxEvent conversion.
//!
//! Hosts 2 helper methods on `Demuxer`:
//!
//! - `handle_pes_packet` — feeds a TS packet's PES bytes into the
//!   reassembler, surfaces oversize / total-cap discontinuities,
//!   converts complete PES results into events.
//! - `handle_complete_pes` — the central event-construction site.
//!   Dispatches by `StreamKind`; constructs `DemuxEvent::Sample` /
//!   `DemuxEvent::Metadata` with codec-specific payload shapes
//!   (NAL split for H.264/265/266; OBU split for AV1; envelope strip
//!   for DVB-sub; AU cell peel for sync KLV; raw pass-through for
//!   audio / async KLV / unknown).
//!
//! All items are `pub(super)`.

use super::pmt_classify::{nal_payload_bytes, stream_type_from_kind};
use crate::mpegts::common::{Pts90khz, StreamTypeCode, pts_diff_33bit};
use crate::mpegts::demux::event::{
    AudioCodec, DemuxEvent, DiscontinuityKind, MetadataKind, NonConformantIssue, SamplePayload,
    StreamId, StreamKind, SubtitleCodec, VideoCodec, VideoPayload,
};
use crate::mpegts::demux::payload::{
    Av1BindingUnwrap, DvbSubStripResult, KlvShape, classify_klv, split_nals, split_obus,
    strip_dvb_sub_envelope, unwrap_av1_binding,
};
use crate::mpegts::demux::pes::{PesPayload, ReassemblyOutcome};

/// True when H.222.0 §2.7.4 requires the PES to carry a PTS for this
/// stream kind. Audio and video have a mandatory PTS contract; subtitle,
/// KLV (sync via PTS-bearing AU cells, async without PTS), and unknown
/// stream types are optional/codec-defined.
fn stream_type_requires_pts(kind: &StreamKind) -> bool {
    matches!(kind, StreamKind::Video(_) | StreamKind::Audio(_))
}

impl super::demuxer::Demuxer {
    pub(super) fn handle_pes_packet(
        &mut self,
        pkt: &crate::mpegts::demux::ts::TsPacket<'_>,
    ) -> Result<(), crate::error::DemuxError> {
        let outcomes = self.pes.push(
            pkt.pid,
            pkt.payload,
            pkt.payload_unit_start,
            pkt.random_access_indicator,
        )?;
        for outcome in outcomes {
            match outcome {
                ReassemblyOutcome::Complete(pes) => {
                    self.handle_complete_pes(pes);
                }
                ReassemblyOutcome::Overflow { pid } => {
                    if let Some(stream) = self.lookup_stream(pid) {
                        self.discontinuities_count += 1;
                        let program_number = self.program_number_for_pid(stream.pid);
                        self.stats_per_stream
                            .entry(stream.pid)
                            .or_insert_with(|| crate::mpegts::stats::StreamStats {
                                pid: stream.pid,
                                stream_type: StreamTypeCode::from_byte(stream_type_from_kind(
                                    &stream.kind,
                                )),
                                program_number,
                                ..Default::default()
                            })
                            .discontinuities += 1;
                        self.queue.push_back(DemuxEvent::Discontinuity {
                            stream,
                            kind: DiscontinuityKind::PesOversize { pid },
                        });
                    }
                }
                ReassemblyOutcome::OverflowTotal => {
                    if let Some(stream) = self.lookup_stream(pkt.pid) {
                        self.discontinuities_count += 1;
                        let program_number = self.program_number_for_pid(stream.pid);
                        self.stats_per_stream
                            .entry(stream.pid)
                            .or_insert_with(|| crate::mpegts::stats::StreamStats {
                                pid: stream.pid,
                                stream_type: StreamTypeCode::from_byte(stream_type_from_kind(
                                    &stream.kind,
                                )),
                                program_number,
                                ..Default::default()
                            })
                            .discontinuities += 1;
                        self.queue.push_back(DemuxEvent::Discontinuity {
                            stream,
                            kind: DiscontinuityKind::PesTotalOversize,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn handle_complete_pes(&mut self, pes: PesPayload) {
        let kind = match self.stream_kind_by_pid.get(&pes.pid).copied() {
            Some(k) => k,
            None => return,
        };
        let program_number = self.program_number_for_pid(pes.pid);
        let stream = StreamId {
            pid: pes.pid,
            kind,
            program_number,
        };
        // B5 — surface PES header structural issues collected during
        // parse_complete. These travel through the strict-mode cascade
        // like any other NonConformantIssue. We process them BEFORE the
        // PTS / DTS dispatch so consumers see the issue alongside the
        // (possibly-truncated) sample event.
        for kind_violation in &pes.header_issues {
            self.queue_nonconformant(
                stream,
                NonConformantIssue::PesHeaderMalformed {
                    pid: pes.pid,
                    kind: *kind_violation,
                },
            );
        }
        // B4 — PTS distinct from PCR. Only update `last_pts_by_pid`
        // when an actual PTS arrived; never write 0 as a fallback (the
        // prior code corrupted the monotonicity check for streams that
        // omit PTS sporadically). For stream types where H.222.0 §2.7.4
        // makes PTS mandatory (audio + video), emit
        // `MissingRequiredPts` when absent.
        let pts = pes.pts.unwrap_or(Pts90khz::new(0));
        if pes.pts.is_none() && stream_type_requires_pts(&kind) {
            self.queue_nonconformant(
                stream,
                NonConformantIssue::MissingRequiredPts { pid: pes.pid },
            );
        }
        if let Some(observed_pts) = pes.pts {
            if let Some(last) = self.last_pts_by_pid.get(&pes.pid).copied() {
                let delta = pts_diff_33bit(observed_pts.as_ticks() as u64, last as u64);
                if delta < -90_000 {
                    // PTS anomaly is its own variant (90 kHz / per-PID
                    // elementary stream), distinct from PcrAnomaly
                    // (27 MHz / per-program PCR PID).
                    self.queue_nonconformant(stream, NonConformantIssue::PtsAnomaly { delta });
                }
            }
            self.last_pts_by_pid
                .insert(pes.pid, observed_pts.as_ticks());
        }
        match kind {
            StreamKind::Video(codec) => {
                // Codec dispatches the payload-shape: H.26x splits Annex-B NAL
                // units (split_nals); AV1 splits OBUs (split_obus). The two
                // share the same Sample event surface but emit different
                // VideoPayload variants — the invariant is documented on
                // VideoPayload.
                let rai = pes.random_access_indicator;
                let (sample, payload_bytes, suppress_sample) = match codec {
                    VideoCodec::H264 | VideoCodec::H265 | VideoCodec::H266 => {
                        let (nals, issues) = split_nals(&pes.payload, codec);
                        // NAL-header issues from B9 — forward to the
                        // non-conformance pipeline. If strict mode rejects
                        // any of them, suppress the Sample event so strict
                        // consumers see only the StrictRejection (mirrors
                        // C10's DvbSubDataIdentifier handling).
                        let mut reject_sample = false;
                        for issue in issues {
                            if self.options.strict.rejects(&issue) {
                                reject_sample = true;
                            }
                            self.queue_nonconformant(stream, issue);
                        }
                        let bytes = nal_payload_bytes(&nals);
                        (
                            SamplePayload::Video {
                                codec,
                                payload: VideoPayload::Nals(nals),
                                random_access_indicator: rai,
                            },
                            bytes,
                            reject_sample,
                        )
                    }
                    VideoCodec::Av1 => {
                        // AV1-in-MPEG-2-TS binding §3.2 + §3.4 conformance —
                        // surface binding-spec violations BEFORE running the
                        // OBU splitter. In `Mpeg2TsBinding` mode the demuxer
                        // expects PES `stream_id=0xBD` (§3.4) and a
                        // `ts_open_bitstream_unit()` start-code prefix on
                        // each OBU (§3.2). The matching mux-side carriage
                        // is `MuxerConfig::av1_carriage = Mpeg2TsBinding`.
                        //
                        // In `InteropRawObu` mode (matches ffmpeg / libaom
                        // / hls.js / mediamtx today) the demuxer accepts
                        // `stream_id=0xE0` and raw OBUs without raising
                        // any binding issues. Senders that emit interop
                        // carriage but consumers running binding-strict
                        // demuxers will see one each of `Av1WrongStreamId`
                        // + `Av1MissingTsObuFraming` per PES — both are
                        // best-effort detectors; lenient mode still
                        // surfaces the OBUs via the raw-OBU fallback.
                        use crate::mpegts::mux::Av1CarriageMode;
                        let binding_mode =
                            matches!(self.options.av1_carriage, Av1CarriageMode::Mpeg2TsBinding);
                        let mut reject_sample = false;

                        // Per AV1-in-MPEG-2-TS binding §3.4 PES stream_id
                        // MUST be 0xBD. Surface the observed byte when it
                        // disagrees, but only in binding mode — interop
                        // mode tolerates 0xE0 silently.
                        if binding_mode && pes.stream_id != 0xBD {
                            let issue = NonConformantIssue::Av1WrongStreamId {
                                pid: stream.pid,
                                observed: pes.stream_id,
                            };
                            if self.options.strict.rejects(&issue) {
                                reject_sample = true;
                            }
                            self.queue_nonconformant(stream, issue);
                        }

                        // Try the `ts_open_bitstream_unit()` unwrap in
                        // binding mode. If it fails, surface
                        // `Av1MissingTsObuFraming` and fall back to raw-OBU
                        // parsing on the original payload — strict mode
                        // will still reject via `reject_sample`.
                        let owned_payload: Vec<u8>;
                        let obu_input: &[u8] = if binding_mode {
                            match unwrap_av1_binding(&pes.payload) {
                                Av1BindingUnwrap::Conformant(v) => {
                                    owned_payload = v;
                                    &owned_payload
                                }
                                Av1BindingUnwrap::MissingFraming => {
                                    let issue = NonConformantIssue::Av1MissingTsObuFraming {
                                        pid: stream.pid,
                                    };
                                    if self.options.strict.rejects(&issue) {
                                        reject_sample = true;
                                    }
                                    self.queue_nonconformant(stream, issue);
                                    &pes.payload
                                }
                            }
                        } else {
                            &pes.payload
                        };

                        let (obus, mut issues) = split_obus(obu_input);
                        // split_obus uses pid=0 as a sentinel on the issues it
                        // raises (it doesn't know its own PID context). Patch
                        // each issue with the real stream pid before forwarding
                        // to the non-conformance pipeline.
                        for issue in &mut issues {
                            match issue {
                                NonConformantIssue::Av1ObuMissingSizeField { pid, .. } => {
                                    *pid = stream.pid
                                }
                                NonConformantIssue::Av1TileListNotAllowed { pid } => {
                                    *pid = stream.pid
                                }
                                NonConformantIssue::Av1ObuHeader { pid, .. } => *pid = stream.pid,
                                _ => {}
                            }
                        }
                        for issue in issues {
                            if self.options.strict.rejects(&issue) {
                                reject_sample = true;
                            }
                            self.queue_nonconformant(stream, issue);
                        }
                        let bytes: usize = obus.iter().map(|o| o.payload.len()).sum();
                        (
                            SamplePayload::Video {
                                codec,
                                payload: VideoPayload::Obus(obus),
                                random_access_indicator: rai,
                            },
                            bytes,
                            reject_sample,
                        )
                    }
                };
                self.stats_per_stream
                    .entry(stream.pid)
                    .or_insert_with(|| crate::mpegts::stats::StreamStats {
                        pid: stream.pid,
                        stream_type: StreamTypeCode::from_byte(stream_type_from_kind(&stream.kind)),
                        program_number,
                        ..Default::default()
                    })
                    .items += 1;
                self.stats_per_stream.get_mut(&stream.pid).unwrap().bytes += payload_bytes as u64;
                // Codec-specific counter bump. `nals_or_obus` counts the units
                // split off this AU; `random_access_aus` increments by 1 when
                // the TS adaptation-field RAI bit was set on the PES_start
                // packet (latched into `random_access_indicator` on the Video
                // variant).
                let (nals_or_obus_count, ra_count) = match &sample {
                    SamplePayload::Video {
                        payload: VideoPayload::Nals(nals),
                        random_access_indicator,
                        ..
                    } => (
                        nals.len() as u64,
                        if *random_access_indicator { 1 } else { 0 },
                    ),
                    SamplePayload::Video {
                        payload: VideoPayload::Obus(obus),
                        random_access_indicator,
                        ..
                    } => (
                        obus.len() as u64,
                        if *random_access_indicator { 1 } else { 0 },
                    ),
                    _ => (0, 0),
                };
                if nals_or_obus_count > 0 || ra_count > 0 {
                    self.bump_video_counters(stream.pid, nals_or_obus_count, ra_count);
                }
                // In strict mode, NAL/OBU header violations suppress the
                // Sample event so consumers see only the StrictRejection
                // (same shape as C10's DvbSubDataIdentifier handling).
                if !suppress_sample {
                    self.queue.push_back(DemuxEvent::Sample {
                        stream,
                        pts,
                        dts: pes.dts,
                        payload: sample,
                    });
                }
            }
            StreamKind::KlvSync { .. } | StreamKind::KlvAsync => {
                let shape = classify_klv(&pes.payload);
                let (kind_meta, payload, used_pts) = match (shape, kind) {
                    (KlvShape::SyncAuCell { klv, header }, _) => {
                        // If declared async but payload is sync, surface mismatch
                        // — but only once per PID per PMT version. Coalesces
                        // what would otherwise be thousands of identical events.
                        // Coalesce set now lives on ProgramTracker; look up by PID.
                        if matches!(kind, StreamKind::KlvAsync) && self.klv_mismatch_insert(pes.pid)
                        {
                            self.queue_nonconformant(
                                stream,
                                NonConformantIssue::StreamTypeMismatchSyncOnAsyncPid,
                            );
                        }
                        let kind_meta = MetadataKind::KlvSyncAuCell {
                            metadata_service_id: header.metadata_service_id,
                            sequence_number: header.sequence_number,
                            cell_fragment_indication: header.cell_fragment_indication,
                            decoder_config_flag: header.decoder_config_flag,
                            random_access_indicator: header.random_access_indicator,
                        };
                        // PES PTS surfaces unchanged; per H.222.0 §2.12.4.1 the
                        // AU cell itself carries no embedded timestamp.
                        (kind_meta, klv, pts)
                    }
                    (KlvShape::Async { klv }, StreamKind::KlvSync { .. }) => {
                        if self.klv_mismatch_insert(pes.pid) {
                            self.queue_nonconformant(
                                stream,
                                NonConformantIssue::StreamTypeMismatchAsyncOnSyncPid,
                            );
                        }
                        (MetadataKind::KlvAsync, klv, pts)
                    }
                    (KlvShape::Async { klv }, _) => (MetadataKind::KlvAsync, klv, pts),
                    (KlvShape::PartialAuCell { dropped_bytes }, _) => {
                        // AU cell header parsed but CFI != Complete (First /
                        // Middle / Last). Reassembly is not implemented; drop
                        // the payload and emit a detect-only NonConformant event
                        // so consumers can observe the loss in telemetry.
                        self.queue_nonconformant(
                            stream,
                            NonConformantIssue::MultiCellAu {
                                pid: pes.pid,
                                dropped_bytes,
                            },
                        );
                        return;
                    }
                    (KlvShape::Other, _) => {
                        let payload_len = pes.payload.len();
                        let raw = pes.payload;
                        let entry = self.stats_per_stream.entry(stream.pid).or_insert_with(|| {
                            crate::mpegts::stats::StreamStats {
                                pid: stream.pid,
                                stream_type: StreamTypeCode::from_byte(stream_type_from_kind(
                                    &stream.kind,
                                )),
                                program_number,
                                ..Default::default()
                            }
                        });
                        entry.items += 1;
                        entry.bytes += payload_len as u64;
                        self.queue.push_back(DemuxEvent::Sample {
                            stream,
                            pts,
                            dts: pes.dts,
                            payload: SamplePayload::Unknown {
                                stream_type: StreamTypeCode::from_byte(0x15),
                                raw,
                            },
                        });
                        return;
                    }
                };
                let meta_len = payload.len();
                let entry = self.stats_per_stream.entry(stream.pid).or_insert_with(|| {
                    crate::mpegts::stats::StreamStats {
                        pid: stream.pid,
                        stream_type: StreamTypeCode::from_byte(stream_type_from_kind(&stream.kind)),
                        program_number,
                        ..Default::default()
                    }
                });
                entry.items += 1;
                entry.bytes += meta_len as u64;
                // Codec-specific counter bump. Today every KLV PES carries
                // exactly one record (sender-side `push_klv` is one-record-
                // per-call, and the demuxer emits one event per PES). If a
                // future sender or external tool ships multi-record PESes,
                // replace `1` with an LS-substrate iterator count on
                // `payload`.
                self.bump_klv_counters(stream.pid, 1);
                self.queue.push_back(DemuxEvent::Metadata {
                    stream,
                    pts: used_pts,
                    kind: kind_meta,
                    payload,
                });
            }
            StreamKind::Unknown(stream_type) => {
                let payload_len = pes.payload.len();
                let entry = self.stats_per_stream.entry(stream.pid).or_insert_with(|| {
                    crate::mpegts::stats::StreamStats {
                        pid: stream.pid,
                        stream_type: StreamTypeCode::from_byte(stream_type),
                        program_number,
                        ..Default::default()
                    }
                });
                entry.items += 1;
                entry.bytes += payload_len as u64;
                self.queue.push_back(DemuxEvent::Sample {
                    stream,
                    pts,
                    dts: pes.dts,
                    payload: SamplePayload::Unknown {
                        stream_type: StreamTypeCode::from_byte(stream_type),
                        raw: pes.payload,
                    },
                });
            }
            StreamKind::Audio(codec) => {
                let payload_len = pes.payload.len();
                let entry = self.stats_per_stream.entry(stream.pid).or_insert_with(|| {
                    crate::mpegts::stats::StreamStats {
                        pid: stream.pid,
                        stream_type: StreamTypeCode::from_byte(stream_type_from_kind(&stream.kind)),
                        program_number,
                        ..Default::default()
                    }
                });
                entry.items += 1;
                entry.bytes += payload_len as u64;
                // C11 — for AAC-LATM (stream_type 0x11) validate the LOAS
                // syncword at the start of the PES payload. Pre-C11 we
                // advertised LATM without any framing check, so malformed
                // streams produced opaque Sample events that downstream
                // decoders couldn't parse. Lenient mode surfaces the
                // NonConformantIssue alongside the Sample (callers may
                // want the raw bytes for forensic analysis); strict mode
                // (Full) suppresses the sample.
                let latm_rejected = if codec == AudioCodec::AacLatm {
                    match crate::codec::aac::latm::validate_latm_sync(&pes.payload) {
                        Ok(_) => false,
                        Err(kind) => {
                            let issue = NonConformantIssue::LatmFraming { pid: pes.pid, kind };
                            let reject = self.options.strict.rejects(&issue);
                            self.queue_nonconformant(stream, issue);
                            reject
                        }
                    }
                } else {
                    false
                };
                if latm_rejected {
                    return;
                }
                // Codec-specific counter bump. AAC-ADTS + MP2 have frame
                // iterators in `codec::*`; LATM + AC-3 don't (their
                // `stream_codec_stats` accessor falls back to
                // `StreamCodecStats::Unknown` via the stats_per_stream-only
                // path).
                //
                // validate-1 followup-2: use the resync variants
                // (`frames_with_resync`) so a single malformed syncframe in
                // the middle of a PES payload doesn't drop the rest of the
                // frame count — the strict `frames()` iterators terminate
                // on first parse error and undercount stats. Strict
                // `frames()` remains available for fail-fast conformance
                // callers (fuzzers, spec tests).
                let frames_delta: u64 = match codec {
                    AudioCodec::Aac => crate::codec::aac::frames_with_resync(&pes.payload)
                        .filter_map(Result::ok)
                        .count() as u64,
                    AudioCodec::Mp2 => crate::codec::mpegaudio::frames_with_resync(&pes.payload)
                        .filter_map(Result::ok)
                        .count() as u64,
                    _ => 0, // AacLatm / Ac3 — no iterator yet
                };
                if frames_delta > 0 {
                    self.bump_audio_counters(stream.pid, frames_delta);
                }
                // C12 — AC-3 syncframe alignment enforcement.
                //
                // ATSC A/52:2018 §A.6.3 mandates `data_alignment_indicator=1`
                // for every AC-3 PES, with the implication that the PES
                // payload starts at an AC-3 syncframe (sync word 0x0B77).
                // Surface a NonConformantIssue when the alignment flag is
                // set but the payload doesn't begin with the syncword;
                // strict mode (Full) suppresses the sample so consumers
                // can fail closed.
                let ac3_sync_rejected = if matches!(codec, AudioCodec::Ac3)
                    && pes.data_alignment_indicator
                    && !(pes.payload.len() >= 2 && pes.payload[0] == 0x0B && pes.payload[1] == 0x77)
                {
                    let issue = NonConformantIssue::Ac3SyncMissing { pid: pes.pid };
                    let reject = self.options.strict.rejects(&issue);
                    self.queue_nonconformant(stream, issue);
                    reject
                } else {
                    false
                };
                if !ac3_sync_rejected {
                    self.queue.push_back(DemuxEvent::Sample {
                        stream,
                        pts,
                        dts: None,
                        payload: SamplePayload::Audio {
                            codec,
                            frames: pes.payload.to_vec(),
                        },
                    });
                }
            }
            StreamKind::Subtitle(codec) => {
                let payload_len = pes.payload.len();
                if self.subtitle_pids_seen.insert(stream.pid) {
                    self.subtitle_streams_seen_count += 1;
                }
                let entry = self.stats_per_stream.entry(stream.pid).or_insert_with(|| {
                    crate::mpegts::stats::StreamStats {
                        pid: stream.pid,
                        stream_type: StreamTypeCode::from_byte(stream_type_from_kind(&stream.kind)),
                        program_number,
                        label: Some(
                            crate::mpegts::stats::demux_subtitle_codec_label(codec).to_string(),
                        ),
                        ..Default::default()
                    }
                });
                entry.items += 1;
                entry.bytes += payload_len as u64;
                // B6 — EN 300 743 §6.2 (DVB-sub) + EN 300 472 §4.2
                // (teletext) mandate `data_alignment_indicator = 1`.
                // CEA-708 standalone and WebVTT-in-TS don't formally
                // require it but conventionally set it. Surface a
                // NonConformant issue when absent on the DVB pair;
                // strict mode (Full) suppresses the sample.
                let needs_alignment = matches!(
                    codec,
                    SubtitleCodec::DvbSubtitling | SubtitleCodec::DvbTeletext
                );
                let alignment_rejected = if needs_alignment && !pes.data_alignment_indicator {
                    let issue = NonConformantIssue::SubtitleAlignmentMissing { pid: pes.pid };
                    let reject = self.options.strict.rejects(&issue);
                    self.queue_nonconformant(stream, issue);
                    reject
                } else {
                    false
                };
                // For DVB subtitling, strip the EN 300 743 §6.2 PES_data_field
                // envelope (data_identifier + subtitle_stream_id + segments +
                // 0xFF end_marker) so callers see just the segment bytes —
                // matching what libavcodec's dvbsubdec expects (it rejects
                // anything that doesn't begin with a segment sync_byte 0x0F).
                // Other subtitle codecs (teletext, CEA-708 standalone, WebVTT)
                // do not have this wrapper; pass through verbatim.
                //
                // §6.2 Table 3 binds DVB-subtitle data_identifier to exactly
                // 0x20. The strip helper distinguishes Conformant (== 0x20),
                // NonConformantDataId (in the legacy permissive range
                // 0x20..=0x3F | 0x70..=0x7F but != 0x20), and Malformed
                // (anything else). For NonConformantDataId, lenient mode
                // strips + emits the sample alongside the
                // DvbSubDataIdentifier issue; strict mode suppresses the
                // sample so consumers can fail closed.
                let raw = &pes.payload;
                let surfaced_payload = if alignment_rejected {
                    // Strict-mode B6 rejection suppresses the sample so
                    // the receive loop can fail closed (parallel to the
                    // DvbSubDataIdentifier strict-mode path).
                    None
                } else {
                    match codec {
                        SubtitleCodec::DvbSubtitling => match strip_dvb_sub_envelope(raw) {
                            DvbSubStripResult::Conformant(s) => Some(s.to_vec()),
                            DvbSubStripResult::NonConformantDataId { observed, stripped } => {
                                let stripped = stripped.to_vec();
                                let issue = NonConformantIssue::DvbSubDataIdentifier { observed };
                                let reject = self.options.strict.rejects(&issue);
                                self.queue_nonconformant(stream, issue);
                                if reject { None } else { Some(stripped) }
                            }
                            DvbSubStripResult::Malformed => Some(raw.to_vec()),
                        },
                        _ => Some(raw.to_vec()),
                    }
                };
                if let Some(surfaced_payload) = surfaced_payload {
                    self.queue.push_back(DemuxEvent::Sample {
                        stream,
                        pts,
                        dts: None,
                        payload: SamplePayload::Subtitle {
                            codec,
                            payload: surfaced_payload,
                        },
                    });
                }
            }
        }
    }
}
