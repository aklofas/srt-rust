// crates/srt-core/src/mpegts/demux/demuxer.rs
//! Top-level `Demuxer` state machine.

use crate::error::DemuxError;
use crate::mpegts::common::{pcr_diff_27mhz, pts_diff_33bit};
use crate::mpegts::demux::event::{
    DemuxEvent, DiscontinuityKind, KlvLink, LinkSource, MetadataKind, NalUnit, NonConformantIssue,
    ProgramMap, SamplePayload, StreamId, StreamInfo, StreamKind, VideoCodec,
};
use crate::mpegts::demux::payload::{KlvShape, classify_klv, split_nals};
use crate::mpegts::demux::pes::{Reassembler, ReassemblyOutcome};
use crate::mpegts::demux::psi::{
    Pmt, PsiParseError, extract_metadata_link, extract_user_label, has_klva_registration,
    parse_pat, parse_pmt,
};
use crate::mpegts::demux::strict::StrictMode;
use crate::mpegts::demux::ts::{TsParseError, parse_ts_packet};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

/// Stats snapshot for [`Demuxer`]. Used by
/// [`crate::pipeline::receiver::Receiver`] to compose its own `ReceiverStats`;
/// also exposed publicly for callers using `Demuxer` directly.
///
/// Per-stream entries are created lazily as events are emitted — the
/// receiver discovers topology rather than configuring it up front. PSI
/// PIDs (PAT 0x0000, active PMT PID) get hardcoded labels "PAT" / "PMT".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DemuxerStats {
    /// Number of `ProgramMap` events emitted (one per PMT version seen).
    pub program_maps_seen: u64,
    /// Number of distinct PMT version_number values seen, including the
    /// initial sighting. Resets to zero on `reset_stats`, so the next PMT
    /// always increments this counter.
    pub pmt_versions_seen: u64,
    /// Total discontinuity events emitted across all PIDs.
    pub discontinuities: u64,
    /// Total non-conformant events emitted across all PIDs.
    pub nonconformant: u64,
    /// Per-PID counters. Keys are PIDs. Entries are created on first event
    /// for a given PID; PSI PIDs (0x0000 for PAT, the PMT PID) are added
    /// with fixed "PAT"/"PMT" labels when a `ProgramMap` event fires.
    pub per_stream: BTreeMap<u16, crate::mpegts::stats::StreamStats>,
}

const DEFAULT_PES_CAP_PER_PID: usize = 4 * 1024 * 1024;
const DEFAULT_PES_CAP_TOTAL: usize = 64 * 1024 * 1024;

/// Maximum bytes the demuxer scans during sync recovery before declaring
/// the stream unrecoverable.
const SYNC_SEARCH_WINDOW: usize = 188 * 32;

/// PCR jump threshold beyond which we emit `PcrAnomaly`. 1 second @ 27 MHz.
const PCR_ANOMALY_THRESHOLD: i64 = 27_000_000;

/// Caller-supplied overrides for the demuxer.
#[derive(Debug, Clone, Default)]
pub struct DemuxerOptions {
    pub strict: StrictMode,
    pub pes_cap_per_pid: Option<usize>,
    pub pes_cap_total: Option<usize>,
    pub klv_link_overrides: Vec<(u16, u16)>,
    pub stream_kind_overrides: HashMap<u16, StreamKind>,
}

#[derive(Debug)]
pub struct Demuxer {
    options: DemuxerOptions,
    /// Bytes that haven't yet been sync-aligned into 188-byte packets.
    /// `sync_consumed` is the cursor into this buffer; the live region is
    /// `sync_buf[sync_consumed..]`. Avoiding `drain(..n)` per packet is
    /// what keeps `feed` amortized-linear on whole-file inputs (a naive
    /// drain is O(remaining) per call → O(N²) total).
    sync_buf: Vec<u8>,
    /// Cursor into `sync_buf`; bytes before this index are consumed and
    /// will be reclaimed on the next compaction.
    sync_consumed: usize,
    /// Per-PID PSI assembly buffer (PAT/PMT). Drained when `section_length`
    /// + 3 bytes have been seen.
    psi_buf: HashMap<u16, Vec<u8>>,
    pat_pmt_pid: Option<u16>,
    pmt: Option<Pmt>,
    pmt_version: Option<u8>,
    pat_version: Option<u8>,
    stream_kind_by_pid: HashMap<u16, StreamKind>,
    cc_by_pid: HashMap<u16, u8>,
    last_pcr_27mhz: Option<u64>,
    last_pts_by_pid: HashMap<u16, i64>,
    /// PIDs that have already surfaced a `StreamTypeMismatch{Sync,Async}On*Pid`
    /// nonconformance for the current PMT version. Real captures emit the
    /// same mismatch on every record (often thousands per stream), so we
    /// coalesce to one event per (PID, PMT version). Cleared on PMT bump.
    klv_mismatch_emitted_pids: HashSet<u16>,
    pes: Reassembler,
    queue: VecDeque<DemuxEvent>,
    bytes_since_sync: usize,
    /// First strict-mode-rejected issue captured this `feed` call. Drained
    /// at the end of each packet's processing and converted into a
    /// `DemuxError::StrictRejection` return. The `NonConformant` event
    /// itself is still pushed onto `queue` so a caller that already
    /// drained events sees the rejection narrative if they wish.
    fatal: Option<NonConformantIssue>,
    // ── stats counters ──────────────────────────────────────────────────
    program_maps_seen: u64,
    pmt_versions_seen: u64,
    discontinuities_count: u64,
    nonconformant_count: u64,
    /// Per-PID counters; entries created lazily on first event per PID.
    stats_per_stream: BTreeMap<u16, crate::mpegts::stats::StreamStats>,
    /// PMT PID last seen — needed to attach the "PMT" label on stats
    /// refresh without re-deriving it from scratch.
    stats_pmt_pid: Option<u16>,
}

impl Demuxer {
    pub fn new() -> Self {
        Self::with_options(DemuxerOptions::default())
    }

    pub fn with_options(options: DemuxerOptions) -> Self {
        let cap_per_pid = options.pes_cap_per_pid.unwrap_or(DEFAULT_PES_CAP_PER_PID);
        let cap_total = options.pes_cap_total.unwrap_or(DEFAULT_PES_CAP_TOTAL);
        Self {
            options,
            sync_buf: Vec::new(),
            sync_consumed: 0,
            psi_buf: HashMap::new(),
            pat_pmt_pid: None,
            pmt: None,
            pmt_version: None,
            pat_version: None,
            stream_kind_by_pid: HashMap::new(),
            cc_by_pid: HashMap::new(),
            last_pcr_27mhz: None,
            last_pts_by_pid: HashMap::new(),
            klv_mismatch_emitted_pids: HashSet::new(),
            pes: Reassembler::new(cap_per_pid, cap_total),
            queue: VecDeque::new(),
            bytes_since_sync: 0,
            fatal: None,
            program_maps_seen: 0,
            pmt_versions_seen: 0,
            discontinuities_count: 0,
            nonconformant_count: 0,
            stats_per_stream: BTreeMap::new(),
            stats_pmt_pid: None,
        }
    }

    /// Feed bytes into the demuxer. Bytes need not be 188-aligned; the
    /// demuxer handles TS sync recovery internally.
    ///
    /// When `feed` returns `Err(DemuxError::StrictRejection(_))`, the
    /// corresponding `NonConformant` event has already been pushed onto the
    /// internal queue. Drain `next_event()` after the error to retrieve the
    /// structured issue alongside the human-readable error string.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), DemuxError> {
        self.sync_buf.extend_from_slice(bytes);
        loop {
            let live = &self.sync_buf[self.sync_consumed..];
            if live.len() < 188 {
                self.compact_sync_buf();
                return Ok(());
            }
            // Sync to next 0x47.
            if live[0] != 0x47 {
                let mut i = 1;
                while i < live.len() && live[i] != 0x47 {
                    i += 1;
                }
                self.bytes_since_sync += i;
                if self.bytes_since_sync > SYNC_SEARCH_WINDOW {
                    return Err(DemuxError::Unrecoverable {
                        after_bytes: self.bytes_since_sync,
                    });
                }
                self.sync_consumed += i;
                self.compact_sync_buf();
                continue;
            }
            // Have sync; try to parse one packet.
            self.bytes_since_sync = 0;
            // Need to read 188 bytes; if the next byte after isn't 0x47 (or
            // we don't have enough buffer to check), we'll re-sync next loop.
            let pkt_buf: [u8; 188] = live[..188].try_into().unwrap();
            self.sync_consumed += 188;
            self.compact_sync_buf();
            // TODO: consider catching MalformedPes here per Task 4 review —
            // the plan currently propagates this fatally out of `feed`, which
            // ends the receive loop. A future task may convert it to a
            // NonConformant event so the loop survives a single corrupt PES.
            self.process_packet(&pkt_buf)?;
            // Strict-mode hatch: if the packet just processed produced a
            // `NonConformant` event whose issue category is rejected by the
            // configured `StrictMode`, surface it as a fatal error here. The
            // event itself is still in the queue; the caller can drain it
            // alongside the error if they want the narrative.
            if let Some(fatal) = self.fatal.take() {
                return Err(DemuxError::StrictRejection(format!("{fatal:?}")));
            }
        }
    }

    /// Pull the next available event. Returns `None` if no event is
    /// currently queued — feed more bytes and try again.
    pub fn next_event(&mut self) -> Option<DemuxEvent> {
        self.queue.pop_front()
    }

    /// Reclaim the consumed prefix of `sync_buf` once it grows past half
    /// the live size (or 1 MiB, whichever is larger). The half-and-compact
    /// rule keeps total memmove work amortized-linear in bytes fed; the
    /// 1 MiB floor avoids churn on tiny live regions.
    fn compact_sync_buf(&mut self) {
        let consumed = self.sync_consumed;
        let live = self.sync_buf.len() - consumed;
        if consumed >= live.max(1 << 20) {
            self.sync_buf.drain(..consumed);
            self.sync_consumed = 0;
        }
    }

    /// Drain any partial PES still buffered in the reassembler — emit any
    /// complete events from them. Use on stream end (e.g. SRT receive loop
    /// reaching `TransportError::Closed`) to flush the last in-flight video AU
    /// or any other unbounded-PES payload that hadn't yet been finalized
    /// by a subsequent PUSI.
    ///
    /// Idempotent: calling twice with no further `feed` between them is safe
    /// and a no-op the second time.
    pub fn flush(&mut self) {
        let partials = self.pes.drain_partial();
        for pes in partials {
            self.handle_complete_pes(pes);
        }
    }

    fn process_packet(&mut self, buf: &[u8; 188]) -> Result<(), DemuxError> {
        let pkt = match parse_ts_packet(buf) {
            Ok(p) => p,
            Err(TsParseError::NoSyncByte)
            | Err(TsParseError::Truncated)
            | Err(TsParseError::BadAdaptationLength) => return Ok(()),
        };
        self.check_pcr(&pkt);
        self.check_continuity(&pkt);
        if pkt.pid == 0x0000 {
            self.handle_psi(pkt.pid, pkt.payload, pkt.payload_unit_start, true)?;
        } else if Some(pkt.pid) == self.pat_pmt_pid {
            self.handle_psi(pkt.pid, pkt.payload, pkt.payload_unit_start, false)?;
        } else if pkt.has_payload && self.stream_kind_by_pid.contains_key(&pkt.pid) {
            self.handle_pes_packet(&pkt)?;
        }
        Ok(())
    }

    fn check_pcr(&mut self, pkt: &crate::mpegts::demux::ts::TsPacket<'_>) {
        // Rewritten from a let-chain (`if let A && let B`) to nested if-let
        // for MSRV 1.85 compatibility — let-chains require Rust 1.88.
        if let Some(now) = pkt.pcr_27mhz {
            if let Some(last) = self.last_pcr_27mhz {
                let diff = pcr_diff_27mhz(now, last);
                if diff.abs() > PCR_ANOMALY_THRESHOLD {
                    let issue = NonConformantIssue::PcrAnomaly { delta: diff };
                    if let Some(stream) = self.lookup_stream(pkt.pid) {
                        self.queue_nonconformant(stream, issue);
                    }
                }
            }
        }
        if let Some(p) = pkt.pcr_27mhz {
            self.last_pcr_27mhz = Some(p);
        }
    }

    fn check_continuity(&mut self, pkt: &crate::mpegts::demux::ts::TsPacket<'_>) {
        if !pkt.has_payload {
            return;
        }
        if let Some(prev_cc) = self.cc_by_pid.get(&pkt.pid).copied() {
            let expected = (prev_cc + 1) & 0x0F;
            if expected != pkt.continuity_counter {
                if let Some(stream) = self.lookup_stream(pkt.pid) {
                    self.discontinuities_count += 1;
                    self.stats_per_stream
                        .entry(stream.pid)
                        .or_insert_with(|| crate::mpegts::stats::StreamStats {
                            pid: stream.pid,
                            stream_type: stream_type_from_kind(&stream.kind),
                            ..Default::default()
                        })
                        .discontinuities += 1;
                    self.queue.push_back(DemuxEvent::Discontinuity {
                        stream,
                        kind: DiscontinuityKind::ContinuityJump {
                            expected,
                            observed: pkt.continuity_counter,
                        },
                    });
                }
            }
        }
        if pkt.discontinuity_indicator {
            if let Some(stream) = self.lookup_stream(pkt.pid) {
                self.discontinuities_count += 1;
                self.stats_per_stream
                    .entry(stream.pid)
                    .or_insert_with(|| crate::mpegts::stats::StreamStats {
                        pid: stream.pid,
                        stream_type: stream_type_from_kind(&stream.kind),
                        ..Default::default()
                    })
                    .discontinuities += 1;
                self.queue.push_back(DemuxEvent::Discontinuity {
                    stream,
                    kind: DiscontinuityKind::AdaptationFieldFlag,
                });
            }
        }
        self.cc_by_pid.insert(pkt.pid, pkt.continuity_counter);
    }

    fn handle_psi(
        &mut self,
        pid: u16,
        payload: &[u8],
        pusi: bool,
        is_pat: bool,
    ) -> Result<(), DemuxError> {
        if pusi {
            // First byte after pointer_field marks where the section starts.
            if payload.is_empty() {
                return Ok(());
            }
            let pointer_field = payload[0] as usize;
            if 1 + pointer_field > payload.len() {
                return Ok(());
            }
            self.psi_buf
                .insert(pid, payload[1 + pointer_field..].to_vec());
        } else {
            // Continuation: append.
            self.psi_buf
                .entry(pid)
                .or_default()
                .extend_from_slice(payload);
        }
        // Try to drain a complete section if section_length is satisfied.
        // Rewritten from a let-chain (`if let A && cond`) to nested
        // if-let / if for MSRV 1.85 compatibility.
        let drained: Option<Vec<u8>> = if let Some(buf) = self.psi_buf.get(&pid) {
            if buf.len() >= 3 {
                let section_length = (((buf[1] & 0x0F) as u16) << 8) | buf[2] as u16;
                let total_len = 3 + section_length as usize;
                if buf.len() >= total_len {
                    Some(buf[..total_len].to_vec())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        if let Some(section) = drained {
            self.psi_buf.remove(&pid);
            if is_pat {
                self.handle_pat_section(&section);
            } else {
                self.handle_pmt_section(pid, &section);
            }
        }
        Ok(())
    }

    fn handle_pat_section(&mut self, section: &[u8]) {
        let pat = match parse_pat(section) {
            Ok(p) => p,
            Err(PsiParseError::CrcMismatch { .. }) => {
                let stream = StreamId {
                    pid: 0x0000,
                    kind: StreamKind::Unknown(0),
                };
                self.queue_nonconformant(
                    stream,
                    NonConformantIssue::PsiChecksumMismatch { pid: 0x0000 },
                );
                return;
            }
            Err(_) => return,
        };
        // Bump-version-only: re-emit only if version changed or first ever.
        if Some(pat.version) == self.pat_version {
            return;
        }
        self.pat_version = Some(pat.version);
        // First non-network program is our PMT PID.
        for entry in &pat.programs {
            if entry.program_number != 0 {
                self.pat_pmt_pid = Some(entry.pid);
                break;
            }
        }
    }

    fn handle_pmt_section(&mut self, pid: u16, section: &[u8]) {
        let pmt = match parse_pmt(section) {
            Ok(p) => p,
            Err(PsiParseError::CrcMismatch { .. }) => {
                let stream = self.lookup_stream(pid).unwrap_or(StreamId {
                    pid,
                    kind: StreamKind::Unknown(0),
                });
                self.queue_nonconformant(stream, NonConformantIssue::PsiChecksumMismatch { pid });
                return;
            }
            Err(_) => return,
        };
        if Some(pmt.version) == self.pmt_version {
            return;
        }
        self.pmt_version = Some(pmt.version);
        let map = self.build_program_map(&pmt);
        // Update stats: ProgramMap event about to be emitted.
        self.program_maps_seen += 1;
        self.pmt_versions_seen += 1;
        // Upsert PSI PIDs with hardcoded labels.
        self.stats_per_stream
            .entry(0x0000)
            .or_insert_with(|| crate::mpegts::stats::StreamStats {
                pid: 0x0000,
                stream_type: 0x00,
                label: Some("PAT".into()),
                ..Default::default()
            });
        // Record the active PMT PID with a "PMT" label.
        self.stats_pmt_pid = Some(pid);
        self.stats_per_stream
            .entry(pid)
            .or_insert_with(|| crate::mpegts::stats::StreamStats {
                pid,
                stream_type: 0x00,
                label: Some("PMT".into()),
                ..Default::default()
            })
            .label = Some("PMT".into());
        // Upsert per-stream entries from the PMT, populating user labels
        // via psi::extract_user_label on the per-stream descriptor list.
        for (pmt_stream, map_info) in pmt.streams.iter().zip(map.streams.iter()) {
            let label = extract_user_label(&pmt_stream.descriptors);
            let entry = self
                .stats_per_stream
                .entry(map_info.pid)
                .or_insert_with(|| crate::mpegts::stats::StreamStats {
                    pid: map_info.pid,
                    stream_type: map_info.stream_type,
                    label: label.clone(),
                    ..Default::default()
                });
            // Refresh stream_type and label on PMT version bump.
            entry.stream_type = map_info.stream_type;
            if label.is_some() {
                entry.label = label;
            }
        }
        self.pmt = Some(pmt);
        // PMT changed — re-arm KLV mismatch coalescing so the new layout
        // gets one fresh nonconformance event per affected PID.
        self.klv_mismatch_emitted_pids.clear();
        // Fill stream_kind_by_pid so PES dispatch knows codec.
        self.stream_kind_by_pid.clear();
        for s in &map.streams {
            self.stream_kind_by_pid.insert(s.pid, s.kind);
        }
        // Apply caller overrides.
        for (override_pid, kind) in &self.options.stream_kind_overrides {
            self.stream_kind_by_pid.insert(*override_pid, *kind);
        }
        self.queue.push_back(DemuxEvent::ProgramMap(map));
    }

    fn build_program_map(&mut self, pmt: &Pmt) -> ProgramMap {
        let mut streams = Vec::new();
        let mut klv_pids: Vec<(u16, Option<u16>)> = Vec::new();
        let mut video_pids: Vec<u16> = Vec::new();
        for s in &pmt.streams {
            let (kind, declared_link) = self.derive_stream_kind(s);
            if let StreamKind::Video(_) = kind {
                video_pids.push(s.elementary_pid);
            }
            if matches!(kind, StreamKind::KlvSync { .. } | StreamKind::KlvAsync) {
                klv_pids.push((s.elementary_pid, declared_link));
            }
            streams.push(StreamInfo {
                pid: s.elementary_pid,
                stream_type: s.stream_type,
                kind,
                raw_descriptors: s.descriptors.clone(),
            });
        }
        // Build klv_links table.
        let mut klv_links = Vec::new();
        for (klv_pid, declared) in klv_pids {
            // 1. Caller override wins.
            if let Some(&(_, video_pid)) = self
                .options
                .klv_link_overrides
                .iter()
                .find(|&&(k, _)| k == klv_pid)
            {
                klv_links.push(KlvLink {
                    klv_pid,
                    video_pid,
                    source: LinkSource::Override,
                });
                continue;
            }
            // 2. Declared via metadata_descriptor.
            if let Some(video_pid) = declared {
                klv_links.push(KlvLink {
                    klv_pid,
                    video_pid,
                    source: LinkSource::Declared,
                });
                continue;
            }
            // 3. Inferred — exactly one video PID in this PMT.
            if video_pids.len() == 1 {
                klv_links.push(KlvLink {
                    klv_pid,
                    video_pid: video_pids[0],
                    source: LinkSource::Inferred,
                });
            }
            // 4. Otherwise: no entry. Surface MissingMetadataDescriptor as
            // non-conformant.
            else {
                let stream = StreamId {
                    pid: klv_pid,
                    kind: StreamKind::KlvSync {
                        declared_link: None,
                    },
                };
                self.queue_nonconformant(stream, NonConformantIssue::MissingMetadataDescriptor);
            }
        }
        ProgramMap {
            program_number: pmt.program_number,
            pcr_pid: pmt.pcr_pid,
            streams,
            klv_links,
        }
    }

    fn derive_stream_kind(
        &self,
        s: &crate::mpegts::demux::psi::PmtStream,
    ) -> (StreamKind, Option<u16>) {
        let declared_link = extract_metadata_link(&s.descriptors);
        let kind = match s.stream_type {
            0x1B => StreamKind::Video(VideoCodec::H264),
            0x24 => StreamKind::Video(VideoCodec::H265),
            0x06 => {
                if has_klva_registration(&s.descriptors) {
                    StreamKind::KlvAsync
                } else {
                    StreamKind::Unknown(0x06)
                }
            }
            0x15 => StreamKind::KlvSync { declared_link },
            other => StreamKind::Unknown(other),
        };
        (kind, declared_link)
    }

    fn handle_pes_packet(
        &mut self,
        pkt: &crate::mpegts::demux::ts::TsPacket<'_>,
    ) -> Result<(), DemuxError> {
        let outcomes = self
            .pes
            .push(pkt.pid, pkt.payload, pkt.payload_unit_start)?;
        for outcome in outcomes {
            match outcome {
                ReassemblyOutcome::Complete(pes) => {
                    self.handle_complete_pes(pes);
                }
                ReassemblyOutcome::Overflow { pid } => {
                    if let Some(stream) = self.lookup_stream(pid) {
                        self.discontinuities_count += 1;
                        self.stats_per_stream
                            .entry(stream.pid)
                            .or_insert_with(|| crate::mpegts::stats::StreamStats {
                                pid: stream.pid,
                                stream_type: stream_type_from_kind(&stream.kind),
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
                        self.stats_per_stream
                            .entry(stream.pid)
                            .or_insert_with(|| crate::mpegts::stats::StreamStats {
                                pid: stream.pid,
                                stream_type: stream_type_from_kind(&stream.kind),
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

    fn handle_complete_pes(&mut self, pes: crate::mpegts::demux::pes::PesPayload) {
        let kind = match self.stream_kind_by_pid.get(&pes.pid).copied() {
            Some(k) => k,
            None => return,
        };
        let stream = StreamId { pid: pes.pid, kind };
        let pts = pes.pts.unwrap_or(0);
        // Backward-PTS check.
        if let Some(last) = self.last_pts_by_pid.get(&pes.pid).copied() {
            let delta = pts_diff_33bit(pts as u64, last as u64);
            if delta < -90_000 {
                self.queue_nonconformant(stream, NonConformantIssue::PcrAnomaly { delta });
            }
        }
        self.last_pts_by_pid.insert(pes.pid, pts);
        match kind {
            StreamKind::Video(codec) => {
                let nals = split_nals(&pes.payload, codec);
                let payload_bytes = nal_payload_bytes(&nals);
                let sample = SamplePayload::Video { codec, nals };
                self.stats_per_stream
                    .entry(stream.pid)
                    .or_insert_with(|| crate::mpegts::stats::StreamStats {
                        pid: stream.pid,
                        stream_type: stream_type_from_kind(&stream.kind),
                        ..Default::default()
                    })
                    .items += 1;
                self.stats_per_stream.get_mut(&stream.pid).unwrap().bytes += payload_bytes as u64;
                self.queue.push_back(DemuxEvent::Sample {
                    stream,
                    pts,
                    dts: pes.dts,
                    payload: sample,
                });
            }
            StreamKind::KlvSync { .. } | StreamKind::KlvAsync => {
                let shape = classify_klv(&pes.payload);
                let (kind_meta, payload, used_pts) = match (shape, kind) {
                    (KlvShape::SyncAuCell { klv, au_cell_pts }, _) => {
                        // If declared async but payload is sync, surface mismatch
                        // — but only once per PID per PMT version. Coalesces
                        // what would otherwise be thousands of identical events.
                        if matches!(kind, StreamKind::KlvAsync)
                            && self.klv_mismatch_emitted_pids.insert(pes.pid)
                        {
                            self.queue_nonconformant(
                                stream,
                                NonConformantIssue::StreamTypeMismatchSyncOnAsyncPid,
                            );
                        }
                        (MetadataKind::KlvSyncAuCell, klv, au_cell_pts)
                    }
                    (KlvShape::Async { klv }, StreamKind::KlvSync { .. }) => {
                        if self.klv_mismatch_emitted_pids.insert(pes.pid) {
                            self.queue_nonconformant(
                                stream,
                                NonConformantIssue::StreamTypeMismatchAsyncOnSyncPid,
                            );
                        }
                        (MetadataKind::KlvAsync, klv, pts)
                    }
                    (KlvShape::Async { klv }, _) => (MetadataKind::KlvAsync, klv, pts),
                    (KlvShape::Other, _) => {
                        let payload_len = pes.payload.len();
                        let raw = pes.payload;
                        let entry = self.stats_per_stream.entry(stream.pid).or_insert_with(|| {
                            crate::mpegts::stats::StreamStats {
                                pid: stream.pid,
                                stream_type: stream_type_from_kind(&stream.kind),
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
                                stream_type: 0x15,
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
                        stream_type: stream_type_from_kind(&stream.kind),
                        ..Default::default()
                    }
                });
                entry.items += 1;
                entry.bytes += meta_len as u64;
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
                        stream_type,
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
                        stream_type,
                        raw: pes.payload,
                    },
                });
            }
            StreamKind::Audio(_) | StreamKind::Subtitle(_) => {
                // Reserved variants; not yet emitted by the demuxer
                // until typed audio/subtitle codecs land.
            }
        }
    }

    fn lookup_stream(&self, pid: u16) -> Option<StreamId> {
        self.stream_kind_by_pid
            .get(&pid)
            .copied()
            .map(|kind| StreamId { pid, kind })
    }

    fn queue_nonconformant(&mut self, stream: StreamId, issue: NonConformantIssue) {
        // Capture the first strict-rejected issue per `feed` call. The
        // event itself is still queued so a caller draining events
        // before/after the `feed` error sees the narrative.
        if self.options.strict.rejects(&issue) && self.fatal.is_none() {
            self.fatal = Some(issue.clone());
        }
        self.nonconformant_count += 1;
        self.queue
            .push_back(DemuxEvent::NonConformant { stream, issue });
    }

    /// Return a snapshot of current demuxer statistics.
    pub fn stats(&self) -> DemuxerStats {
        DemuxerStats {
            program_maps_seen: self.program_maps_seen,
            pmt_versions_seen: self.pmt_versions_seen,
            discontinuities: self.discontinuities_count,
            nonconformant: self.nonconformant_count,
            per_stream: self.stats_per_stream.clone(),
        }
    }

    /// Reset all stats counters to zero and clear per-stream entries.
    ///
    /// Also drops the cached PMT version so the next incoming PMT will
    /// increment `pmt_versions_seen` even if the version_number hasn't
    /// changed.
    pub fn reset_stats(&mut self) {
        self.program_maps_seen = 0;
        self.pmt_versions_seen = 0;
        self.discontinuities_count = 0;
        self.nonconformant_count = 0;
        self.stats_per_stream.clear();
        // Drop cached PMT version so the next PMT triggers pmt_versions_seen += 1.
        self.pmt_version = None;
    }
}

/// Compute the total payload byte count for a slice of NAL units.
fn nal_payload_bytes(nals: &[NalUnit]) -> usize {
    nals.iter()
        .map(|n| match n {
            NalUnit::H264 { payload, .. } | NalUnit::H265 { payload, .. } => payload.len(),
        })
        .sum()
}

/// Map a `StreamKind` to its MPEG-TS `stream_type` byte (PMT value).
fn stream_type_from_kind(k: &StreamKind) -> u8 {
    match k {
        StreamKind::Video(VideoCodec::H264) => 0x1B,
        StreamKind::Video(VideoCodec::H265) => 0x24,
        StreamKind::Audio(_) => 0x0F,
        StreamKind::Subtitle(_) => 0x06,
        StreamKind::KlvSync { .. } => 0x15,
        StreamKind::KlvAsync => 0x06,
        StreamKind::Unknown(t) => *t,
    }
}

impl Default for Demuxer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Default)]
pub struct DemuxerBuilder {
    options: DemuxerOptions,
}

impl DemuxerBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn strict(mut self, mode: StrictMode) -> Self {
        self.options.strict = mode;
        self
    }

    pub fn pes_cap_per_pid(mut self, bytes: usize) -> Self {
        self.options.pes_cap_per_pid = Some(bytes);
        self
    }

    pub fn pes_cap_total(mut self, bytes: usize) -> Self {
        self.options.pes_cap_total = Some(bytes);
        self
    }

    pub fn link_klv(mut self, klv_pid: u16, video_pid: u16) -> Self {
        self.options.klv_link_overrides.push((klv_pid, video_pid));
        self
    }

    pub fn treat_as(mut self, pid: u16, kind: StreamKind) -> Self {
        self.options.stream_kind_overrides.insert(pid, kind);
        self
    }

    pub fn build(self) -> Demuxer {
        Demuxer::with_options(self.options)
    }
}

#[allow(dead_code)] // exposed for tests + future plan tasks.
pub(crate) const fn default_pes_cap_per_pid() -> usize {
    DEFAULT_PES_CAP_PER_PID
}

#[allow(dead_code)] // exposed for tests + future plan tasks.
pub(crate) const fn default_pes_cap_total() -> usize {
    DEFAULT_PES_CAP_TOTAL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_carries_defaults() {
        let d = DemuxerBuilder::new().build();
        assert_eq!(d.options.strict, StrictMode::Off);
        assert_eq!(d.options.pes_cap_per_pid, None);
    }

    #[test]
    fn builder_overrides_apply() {
        let d = DemuxerBuilder::new()
            .strict(StrictMode::TimingOnly)
            .pes_cap_per_pid(1 << 20)
            .pes_cap_total(8 << 20)
            .link_klv(0x100, 0x101)
            .build();
        assert_eq!(d.options.strict, StrictMode::TimingOnly);
        assert_eq!(d.options.pes_cap_per_pid, Some(1 << 20));
        assert_eq!(d.options.pes_cap_total, Some(8 << 20));
        assert_eq!(d.options.klv_link_overrides, vec![(0x100, 0x101)]);
    }

    #[test]
    fn builder_treat_as_override_applies() {
        let d = DemuxerBuilder::new()
            .treat_as(0x100, StreamKind::Video(VideoCodec::H265))
            .build();
        assert_eq!(
            d.options.stream_kind_overrides.get(&0x100),
            Some(&StreamKind::Video(VideoCodec::H265))
        );
    }

    #[test]
    fn default_caps_match_plan_decision() {
        // Spec §11.2 closure: 4 MiB / 64 MiB.
        assert_eq!(default_pes_cap_per_pid(), 4 * 1024 * 1024);
        assert_eq!(default_pes_cap_total(), 64 * 1024 * 1024);
    }

    #[test]
    fn empty_input_produces_no_events() {
        let mut d = Demuxer::new();
        d.feed(&[]).unwrap();
        assert!(d.next_event().is_none());
    }

    #[test]
    fn unrecoverable_after_bytes() {
        let mut d = Demuxer::new();
        let big = vec![0xAA; SYNC_SEARCH_WINDOW * 2];
        let err = d.feed(&big).unwrap_err();
        assert!(matches!(err, DemuxError::Unrecoverable { .. }));
    }

    #[test]
    fn flush_is_idempotent_and_safe_with_no_state() {
        let mut d = Demuxer::new();
        // Empty — no events queued by flush.
        d.flush();
        assert!(d.next_event().is_none());
        // Second call also a no-op.
        d.flush();
        assert!(d.next_event().is_none());
    }

    #[test]
    fn stats_lazy_creates_per_stream_on_first_event() {
        let d = Demuxer::new();
        let st = d.stats();
        assert_eq!(st.program_maps_seen, 0);
        assert_eq!(st.pmt_versions_seen, 0);
        assert_eq!(st.per_stream.len(), 0);
    }

    #[test]
    fn stats_reset_clears_per_stream_and_zeroes_counters() {
        let mut d = Demuxer::new();
        d.reset_stats();
        let st = d.stats();
        assert_eq!(st.program_maps_seen, 0);
        assert_eq!(st.pmt_versions_seen, 0);
        assert_eq!(st.discontinuities, 0);
        assert_eq!(st.nonconformant, 0);
        assert!(st.per_stream.is_empty());
    }

    #[test]
    fn pmt_program_map_event_carries_raw_descriptors() {
        use crate::mpegts::demux::event::DemuxEvent;
        use crate::mpegts::descriptors;
        use crate::mpegts::mux::{Config, VideoCodec as MuxVideoCodec};

        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, MuxVideoCodec::H264)
            .stream_descriptors_for_video(0, vec![descriptors::user_private(b"EO 1080p")])
            .end_program()
            .build()
            .unwrap();
        let mut mux = crate::mpegts::mux::Muxer::new(cfg).unwrap();
        // Push a minimal H.264 AU to trigger PSI + PES emission.
        mux.push_video(&[0, 0, 0, 1, 0x09, 0x10], 9000, true)
            .unwrap();
        let mut buf = vec![0u8; 188 * 32];
        let n = mux.pull(&mut buf);

        let mut demuxer = Demuxer::new();
        demuxer.feed(&buf[..n]).unwrap();

        let mut events = Vec::new();
        while let Some(e) = demuxer.next_event() {
            events.push(e);
        }

        let pm = events
            .iter()
            .find_map(|e| match e {
                DemuxEvent::ProgramMap(pm) => Some(pm),
                _ => None,
            })
            .expect("ProgramMap event emitted");
        let stream = pm
            .streams
            .iter()
            .find(|s| s.pid == 0x100)
            .expect("video PID 0x100 in ProgramMap");
        assert_eq!(stream.raw_descriptors.len(), 1);
        assert_eq!(stream.raw_descriptors[0].tag, 0xFF);
        assert_eq!(stream.raw_descriptors[0].data, b"EO 1080p".to_vec());
    }
}
