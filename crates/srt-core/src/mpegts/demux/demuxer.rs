// crates/srt-core/src/mpegts/demux/demuxer.rs
//! Top-level `Demuxer` state machine.

use crate::error::DemuxError;
use crate::mpegts::common::{pcr_diff_27mhz, pts_diff_33bit};
use crate::mpegts::demux::event::{
    DemuxEvent, DiscontinuityKind, KlvLink, LinkSource, MetadataKind, NalUnit, NonConformantIssue,
    ProgramMap, SamplePayload, StreamId, StreamInfo, StreamKind, SubtitleCodec, VideoCodec,
};
use crate::mpegts::demux::payload::{KlvShape, classify_klv, split_nals};
use crate::mpegts::demux::pes::{Reassembler, ReassemblyOutcome};
use crate::mpegts::demux::psi::{
    Pmt, PsiParseError, classify_audio_stream_type, extract_metadata_link, has_klva_registration,
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
    /// Number of programs currently tracked (entries in the PAT that have
    /// been received). Reflects the live PAT — increases when a PAT version
    /// bump adds a program, decreases when one is removed.
    pub programs_seen: u32,
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

/// Per-program state tracked after a PAT entry is discovered and a PMT
/// arrives. One entry per `pmt_pid` in `Demuxer::programs`.
///
/// Exposed `pub` so that `programs_for_test` can name it in its return type.
/// Not part of the stable API — treat as opaque outside this crate.
#[derive(Debug)]
pub struct ProgramTracker {
    pub program_number: u16,
    pub pmt_pid: u16,
    pub pmt_version: Option<u8>,
    pub pcr_pid: Option<u16>,
    pub streams: Vec<StreamInfo>,
    /// PIDs that have already had a KLV stream-type-mismatch nonconformant
    /// emitted for the current PMT version. Cleared on PMT version bump.
    pub(crate) klv_mismatch_coalesce: HashSet<u16>,
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
    /// Per-PID PSI assembly buffers (PAT + any active PMT PIDs). Drained
    /// when `section_length + 3` bytes have been accumulated for that PID.
    psi_buffers: HashMap<u16, Vec<u8>>,
    /// Programs found in the current PAT, keyed by `pmt_pid`.
    /// O(1) lookup when routing PMT-bound packets.
    programs: HashMap<u16, ProgramTracker>,
    /// Latest PAT version. Bump triggers PAT diff (programs added/removed).
    pat_version: Option<u8>,
    /// Per-PID stream kind cache for PES dispatch. Flat across all programs
    /// (PIDs must be unique cross-program per ISO 13818-1).
    stream_kind_by_pid: HashMap<u16, StreamKind>,
    cc_by_pid: HashMap<u16, u8>,
    last_pcr_27mhz: Option<u64>,
    last_pts_by_pid: HashMap<u16, i64>,
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
    /// PIDs that have already emitted `SubtitleMissingDescriptor` for the
    /// current PMT version. Cleared at the top of each PMT-version bump so
    /// a fresh PMT re-fires if the descriptor is still missing.
    subtitle_missing_descriptor_emitted: HashSet<u16>,
}

impl Demuxer {
    pub fn new() -> Self {
        Self::with_options(DemuxerOptions::default())
    }

    pub fn with_options(options: DemuxerOptions) -> Self {
        let cap_per_pid = options.pes_cap_per_pid.unwrap_or(DEFAULT_PES_CAP_PER_PID);
        let cap_total = options.pes_cap_total.unwrap_or(DEFAULT_PES_CAP_TOTAL);
        // Seed the PAT PID (0x0000) in psi_buffers so the PSI assembler
        // is ready without a separate "first packet" initialisation step.
        let mut psi_buffers: HashMap<u16, Vec<u8>> = HashMap::new();
        psi_buffers.insert(0x0000, Vec::new());
        Self {
            options,
            sync_buf: Vec::new(),
            sync_consumed: 0,
            psi_buffers,
            programs: HashMap::new(),
            pat_version: None,
            stream_kind_by_pid: HashMap::new(),
            cc_by_pid: HashMap::new(),
            last_pcr_27mhz: None,
            last_pts_by_pid: HashMap::new(),
            pes: Reassembler::new(cap_per_pid, cap_total),
            queue: VecDeque::new(),
            bytes_since_sync: 0,
            fatal: None,
            program_maps_seen: 0,
            pmt_versions_seen: 0,
            discontinuities_count: 0,
            nonconformant_count: 0,
            stats_per_stream: BTreeMap::new(),
            subtitle_missing_descriptor_emitted: HashSet::new(),
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
        } else if self.programs.contains_key(&pkt.pid) {
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
                    let program_number = self.program_number_for_pid(stream.pid);
                    self.stats_per_stream
                        .entry(stream.pid)
                        .or_insert_with(|| crate::mpegts::stats::StreamStats {
                            pid: stream.pid,
                            stream_type: stream_type_from_kind(&stream.kind),
                            program_number,
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
                let program_number = self.program_number_for_pid(stream.pid);
                self.stats_per_stream
                    .entry(stream.pid)
                    .or_insert_with(|| crate::mpegts::stats::StreamStats {
                        pid: stream.pid,
                        stream_type: stream_type_from_kind(&stream.kind),
                        program_number,
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
            self.psi_buffers
                .insert(pid, payload[1 + pointer_field..].to_vec());
        } else {
            // Continuation: append.
            self.psi_buffers
                .entry(pid)
                .or_default()
                .extend_from_slice(payload);
        }
        // Try to drain a complete section if section_length is satisfied.
        // Rewritten from a let-chain (`if let A && cond`) to nested
        // if-let / if for MSRV 1.85 compatibility.
        let drained: Option<Vec<u8>> = if let Some(buf) = self.psi_buffers.get(&pid) {
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
            self.psi_buffers.remove(&pid);
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
                self.queue_nonconformant(
                    StreamId {
                        pid: 0x0000,
                        kind: StreamKind::Unknown(0),
                    },
                    NonConformantIssue::PsiChecksumMismatch { pid: 0x0000 },
                );
                return;
            }
            Err(_) => return,
        };
        // Same version — nothing changed, skip the diff.
        if Some(pat.version) == self.pat_version {
            return;
        }
        self.pat_version = Some(pat.version);

        // Build the set of PMT PIDs in the new PAT, skipping program 0 (NIT).
        let new_pmt_pids: HashSet<u16> = pat
            .programs
            .iter()
            .filter(|e| e.program_number != 0)
            .map(|e| e.pid)
            .collect();

        // Drop trackers for programs that disappeared from this PAT version.
        let removed: Vec<u16> = self
            .programs
            .keys()
            .copied()
            .filter(|pid| !new_pmt_pids.contains(pid))
            .collect();
        for pmt_pid in removed {
            if let Some(tracker) = self.programs.remove(&pmt_pid) {
                // Remove the PES stream-kind entries owned by this program.
                for stream in &tracker.streams {
                    self.stream_kind_by_pid.remove(&stream.pid);
                }
                // Free the PSI assembly buffer for this PMT PID.
                self.psi_buffers.remove(&pmt_pid);
            }
        }

        // Add empty trackers for programs that are new in this PAT version.
        // PMT contents will populate them when handle_pmt_section fires.
        for entry in &pat.programs {
            if entry.program_number == 0 {
                continue; // program 0 = Network PID, not a real program
            }
            self.programs
                .entry(entry.pid)
                .or_insert_with(|| ProgramTracker {
                    program_number: entry.program_number,
                    pmt_pid: entry.pid,
                    pmt_version: None,
                    pcr_pid: None,
                    streams: Vec::new(),
                    klv_mismatch_coalesce: HashSet::new(),
                });
            // Seed the PSI buffer for this PMT PID so handle_psi can accumulate
            // bytes without a separate "first packet" init step.
            self.psi_buffers.entry(entry.pid).or_default();
        }
    }

    fn handle_pmt_section(&mut self, pmt_pid: u16, section: &[u8]) {
        let pmt = match parse_pmt(section) {
            Ok(p) => p,
            Err(PsiParseError::CrcMismatch { .. }) => {
                self.queue_nonconformant(
                    StreamId {
                        pid: pmt_pid,
                        kind: StreamKind::Unknown(0),
                    },
                    NonConformantIssue::PsiChecksumMismatch { pid: pmt_pid },
                );
                return;
            }
            Err(_) => return,
        };

        // Look up the tracker — must exist if PAT pre-populated us.
        let program_number = match self.programs.get(&pmt_pid) {
            Some(t) => t.program_number,
            None => return, // PMT arriving on PID not in PAT — drop.
        };

        // Dedup: re-emit only if version changed or first ever.
        if let Some(tracker) = self.programs.get(&pmt_pid) {
            if Some(pmt.version) == tracker.pmt_version {
                return;
            }
        }

        // Fresh PMT version — clear the per-PID guard for SubtitleMissingDescriptor
        // emission so that a new PMT version re-fires the warning if the
        // descriptor is still absent. Only PIDs owned by *this* PMT are
        // dropped to leave any other program's state intact.
        for s in &pmt.streams {
            self.subtitle_missing_descriptor_emitted
                .remove(&s.elementary_pid);
        }

        // Build StreamInfo list + check cross-program PID collisions.
        // Collect work to do before mutating self — satisfies borrow checker.
        let mut stream_infos: Vec<StreamInfo> = Vec::with_capacity(pmt.streams.len());
        let mut kind_inserts: Vec<(u16, StreamKind)> = Vec::new();
        let mut collision_issues: Vec<(StreamId, NonConformantIssue)> = Vec::new();
        let mut subtitle_missing: Vec<(u16, StreamKind)> = Vec::new();

        for s in &pmt.streams {
            let (kind, _declared_link) = self.get_stream_kind(s.elementary_pid, s);

            // Cross-program PID collision check: scan all other programs'
            // stream lists. First-program-wins — skip this PID if already owned.
            let other_prog = self
                .programs
                .iter()
                .find(|(other_pmt_pid, t)| {
                    **other_pmt_pid != pmt_pid
                        && t.streams.iter().any(|st| st.pid == s.elementary_pid)
                })
                .map(|(_, t)| t.program_number);

            if let Some(other_program_number) = other_prog {
                collision_issues.push((
                    StreamId {
                        pid: s.elementary_pid,
                        kind: StreamKind::Unknown(0),
                    },
                    NonConformantIssue::PidReusedAcrossPrograms {
                        pid: s.elementary_pid,
                        programs: [other_program_number, program_number],
                    },
                ));
                continue; // Skip this stream — first-program-wins.
            }

            // Subtitle-resolved PIDs without a recognized subtitle descriptor
            // (subtitling/teletext/VTTC/GA94) are non-conformant — most often
            // because a `treat_as` override forced StreamKind::Subtitle on a
            // PID whose PMT entry doesn't carry the corresponding tag.
            if matches!(kind, StreamKind::Subtitle(_))
                && !has_recognized_subtitle_descriptor(&s.descriptors)
            {
                subtitle_missing.push((s.elementary_pid, kind));
            }

            stream_infos.push(StreamInfo {
                pid: s.elementary_pid,
                stream_type: s.stream_type,
                kind,
                program_number,
                raw_descriptors: s.descriptors.clone(),
            });
            kind_inserts.push((s.elementary_pid, kind));
        }

        // Emit collision NonConformant events.
        for (stream_id, issue) in collision_issues {
            self.queue_nonconformant(stream_id, issue);
        }

        // Emit SubtitleMissingDescriptor once per PID per fresh PMT version.
        for (pid, kind) in subtitle_missing {
            if self.subtitle_missing_descriptor_emitted.insert(pid) {
                self.queue_nonconformant(
                    StreamId { pid, kind },
                    NonConformantIssue::SubtitleMissingDescriptor { pid },
                );
            }
        }

        // Update stream_kind_by_pid for all non-colliding streams.
        for (pid, kind) in kind_inserts {
            self.stream_kind_by_pid.insert(pid, kind);
        }

        // Build klv_links from the accepted streams.
        let prog_map = self.build_program_map(&pmt, program_number, &stream_infos);

        // Update tracker.
        let tracker = self.programs.get_mut(&pmt_pid).expect("checked above");
        tracker.pmt_version = Some(pmt.version);
        tracker.pcr_pid = Some(pmt.pcr_pid);
        tracker.streams = stream_infos;
        tracker.klv_mismatch_coalesce.clear();

        // Emit ProgramMap event.
        self.queue.push_back(DemuxEvent::ProgramMap(prog_map));
        self.program_maps_seen += 1;
        self.pmt_versions_seen += 1;
    }

    /// Build a `ProgramMap` event payload from a parsed PMT and the accepted
    /// stream list (after cross-program collision filtering).
    fn build_program_map(
        &mut self,
        pmt: &Pmt,
        program_number: u16,
        streams: &[StreamInfo],
    ) -> ProgramMap {
        let mut klv_pids: Vec<(u16, Option<u16>)> = Vec::new();
        let mut video_pids: Vec<u16> = Vec::new();
        for info in streams {
            if let StreamKind::Video(_) = info.kind {
                video_pids.push(info.pid);
            }
            if matches!(info.kind, StreamKind::KlvSync { .. } | StreamKind::KlvAsync) {
                let declared_link = extract_metadata_link_for_pid(pmt, info.pid);
                klv_pids.push((info.pid, declared_link));
            }
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
            program_number,
            pcr_pid: pmt.pcr_pid,
            streams: streams.to_vec(),
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
            0x06 => classify_0x06(&s.descriptors),
            0x15 => StreamKind::KlvSync { declared_link },
            other => {
                if let Some(codec) = classify_audio_stream_type(other) {
                    StreamKind::Audio(codec)
                } else {
                    StreamKind::Unknown(other)
                }
            }
        };
        (kind, declared_link)
    }

    fn get_stream_kind(
        &self,
        pid: u16,
        s: &crate::mpegts::demux::psi::PmtStream,
    ) -> (StreamKind, Option<u16>) {
        // Caller override wins over PMT classification.
        if let Some(&kind) = self.options.stream_kind_overrides.get(&pid) {
            let declared_link = extract_metadata_link(&s.descriptors);
            (kind, declared_link)
        } else {
            self.derive_stream_kind(s)
        }
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
                        let program_number = self.program_number_for_pid(stream.pid);
                        self.stats_per_stream
                            .entry(stream.pid)
                            .or_insert_with(|| crate::mpegts::stats::StreamStats {
                                pid: stream.pid,
                                stream_type: stream_type_from_kind(&stream.kind),
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
                                stream_type: stream_type_from_kind(&stream.kind),
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
        let program_number = self.program_number_for_pid(stream.pid);
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
                        program_number,
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
                        // Coalesce set now lives on ProgramTracker; look up by PID.
                        if matches!(kind, StreamKind::KlvAsync) && self.klv_mismatch_insert(pes.pid)
                        {
                            self.queue_nonconformant(
                                stream,
                                NonConformantIssue::StreamTypeMismatchSyncOnAsyncPid,
                            );
                        }
                        (MetadataKind::KlvSyncAuCell, klv, au_cell_pts)
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
                    (KlvShape::Other, _) => {
                        let payload_len = pes.payload.len();
                        let raw = pes.payload;
                        let entry = self.stats_per_stream.entry(stream.pid).or_insert_with(|| {
                            crate::mpegts::stats::StreamStats {
                                pid: stream.pid,
                                stream_type: stream_type_from_kind(&stream.kind),
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
                        program_number,
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
                        stream_type,
                        raw: pes.payload,
                    },
                });
            }
            StreamKind::Audio(codec) => {
                let payload_len = pes.payload.len();
                let entry = self.stats_per_stream.entry(stream.pid).or_insert_with(|| {
                    crate::mpegts::stats::StreamStats {
                        pid: stream.pid,
                        stream_type: stream_type_from_kind(&stream.kind),
                        program_number,
                        ..Default::default()
                    }
                });
                entry.items += 1;
                entry.bytes += payload_len as u64;
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
            StreamKind::Subtitle(codec) => {
                let payload_len = pes.payload.len();
                let entry = self.stats_per_stream.entry(stream.pid).or_insert_with(|| {
                    crate::mpegts::stats::StreamStats {
                        pid: stream.pid,
                        stream_type: stream_type_from_kind(&stream.kind),
                        program_number,
                        ..Default::default()
                    }
                });
                entry.items += 1;
                entry.bytes += payload_len as u64;
                self.queue.push_back(DemuxEvent::Sample {
                    stream,
                    pts,
                    dts: None,
                    payload: SamplePayload::Subtitle {
                        codec,
                        payload: pes.payload.to_vec(),
                    },
                });
            }
        }
    }

    fn lookup_stream(&self, pid: u16) -> Option<StreamId> {
        self.stream_kind_by_pid
            .get(&pid)
            .copied()
            .map(|kind| StreamId { pid, kind })
    }

    /// Look up the program_number for a PID by searching active ProgramTrackers.
    /// Returns 0 if the PID is not owned by any known program (e.g. PSI PIDs).
    fn program_number_for_pid(&self, pid: u16) -> u16 {
        for tracker in self.programs.values() {
            if tracker.streams.iter().any(|s| s.pid == pid) {
                return tracker.program_number;
            }
        }
        0
    }

    /// Insert `pid` into the KLV mismatch coalesce set for whichever program
    /// owns it. Returns `true` (first-time mismatch for this PID) if the PID
    /// is new to the set, `false` if it was already recorded.
    ///
    /// When `programs` is empty (PSI handlers stubbed, Tasks 10–11), there is
    /// no tracker to update, so this conservatively returns `true` — i.e. does
    /// not suppress any nonconformant emission, which is safe.
    fn klv_mismatch_insert(&mut self, pid: u16) -> bool {
        // Find the tracker that owns this PID via its streams list.
        for tracker in self.programs.values_mut() {
            if tracker.streams.iter().any(|s| s.pid == pid) {
                return tracker.klv_mismatch_coalesce.insert(pid);
            }
        }
        // No tracker found — no suppression.
        true
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

    /// Return a reference to the programs map for integration tests.
    ///
    /// Keyed by `pmt_pid`. Exposed for white-box testing of PAT/PMT diffing
    /// logic; not part of the stable API.
    #[doc(hidden)]
    pub fn programs_for_test(&self) -> &HashMap<u16, ProgramTracker> {
        &self.programs
    }

    /// Return a snapshot of current demuxer statistics.
    pub fn stats(&self) -> DemuxerStats {
        DemuxerStats {
            program_maps_seen: self.program_maps_seen,
            pmt_versions_seen: self.pmt_versions_seen,
            discontinuities: self.discontinuities_count,
            nonconformant: self.nonconformant_count,
            programs_seen: self.programs.len() as u32,
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
        // Drop cached PMT versions on each ProgramTracker so the next PMT
        // triggers pmt_versions_seen += 1 even if the version_number hasn't changed.
        for tracker in self.programs.values_mut() {
            tracker.pmt_version = None;
        }
    }
}

/// Extract the `metadata_descriptor` declared link for a specific PID from
/// a parsed PMT. Used by `build_program_map` to rebuild klv_links after
/// collision filtering has already reduced the stream list.
fn extract_metadata_link_for_pid(pmt: &Pmt, pid: u16) -> Option<u16> {
    pmt.streams
        .iter()
        .find(|s| s.elementary_pid == pid)
        .and_then(|s| extract_metadata_link(&s.descriptors))
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

/// Classify a stream_type 0x06 ("PES private data") by inspecting its
/// PMT-stream descriptors. Subtitle-disambiguating tags take priority
/// over the existing KLV registration check; if no subtitle descriptor
/// is present the result is identical to the prior behavior.
///
/// Priority (most-specific first):
///   1. `subtitling_descriptor` (tag 0x59, ETSI EN 300 468) → DVB subtitling.
///   2. `teletext_descriptor` (tag 0x56) or `VBI_teletext_descriptor`
///      (tag 0x46) → DVB teletext.
///   3. `registration_descriptor` (tag 0x05) format_identifier `"VTTC"` →
///      WebVTT-in-MPEG-TS.
///   4. `registration_descriptor` format_identifier `"GA94"` → CEA-708
///      standalone.
///   5. `registration_descriptor` format_identifier `"KLVA"` → asynchronous
///      MISB KLV (existing behavior).
///   6. Otherwise → `StreamKind::Unknown(0x06)`.
fn classify_0x06(descriptors: &[crate::mpegts::demux::psi::RawDescriptor]) -> StreamKind {
    use crate::mpegts::descriptors::{find_descriptor_tag, find_format_identifier};
    if find_descriptor_tag(descriptors, 0x59) {
        StreamKind::Subtitle(SubtitleCodec::DvbSubtitling)
    } else if find_descriptor_tag(descriptors, 0x56) || find_descriptor_tag(descriptors, 0x46) {
        StreamKind::Subtitle(SubtitleCodec::DvbTeletext)
    } else if find_format_identifier(descriptors, b"VTTC") {
        StreamKind::Subtitle(SubtitleCodec::WebVttInTs)
    } else if find_format_identifier(descriptors, b"GA94") {
        StreamKind::Subtitle(SubtitleCodec::Cea708Standalone)
    } else if has_klva_registration(descriptors) {
        StreamKind::KlvAsync
    } else {
        StreamKind::Unknown(0x06)
    }
}

/// True iff `descriptors` contains any descriptor that lets the demuxer
/// recognize this stream as a subtitle/caption track:
///   * `subtitling_descriptor`  (tag 0x59)
///   * `teletext_descriptor`    (tag 0x56)
///   * `VBI_teletext_descriptor`(tag 0x46)
///   * `registration_descriptor` with format_identifier `"VTTC"` or `"GA94"`.
///
/// Used by the PMT classifier to surface `SubtitleMissingDescriptor`
/// when a `treat_as` override (or any other path) routes a PID to
/// `StreamKind::Subtitle(_)` but the PMT entry has none of the above.
fn has_recognized_subtitle_descriptor(
    descriptors: &[crate::mpegts::demux::psi::RawDescriptor],
) -> bool {
    use crate::mpegts::descriptors::{find_descriptor_tag, find_format_identifier};
    find_descriptor_tag(descriptors, 0x59)
        || find_descriptor_tag(descriptors, 0x56)
        || find_descriptor_tag(descriptors, 0x46)
        || find_format_identifier(descriptors, b"VTTC")
        || find_format_identifier(descriptors, b"GA94")
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
        use crate::mpegts::mux::{ConfigBuilder, VideoCodec as MuxVideoCodec};

        let cfg = ConfigBuilder::default()
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

    #[test]
    fn demuxer_emits_audio_sample_for_aac_pes() {
        use crate::mpegts::demux::event::AudioCodec;
        use crate::mpegts::mux::{AudioCodec as MuxAudioCodec, ConfigBuilder};

        // Mux: single-program with one AAC audio stream.
        let cfg = ConfigBuilder::default()
            .add_program(1, 0x1000)
            .add_audio(0x300, MuxAudioCodec::Aac)
            .end_program()
            .build()
            .unwrap();
        let mut muxer = crate::mpegts::mux::Muxer::new(cfg).unwrap();
        let audio_payload: Vec<u8> = vec![
            0xFF, 0xF1, 0x4C, 0x80, 0x00, 0x1F, 0xFC, 0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01, 0x02,
            0x03, 0x04,
        ];
        muxer.push_audio(&audio_payload, 90_000).unwrap();
        let mut buf = vec![0u8; 188 * 64];
        let n = muxer.pull(&mut buf);
        buf.truncate(n);

        // Demux: feed the muxed bytes, capture events.
        let mut demuxer = Demuxer::new();
        demuxer.feed(&buf).unwrap();
        demuxer.flush();
        let mut events = Vec::new();
        while let Some(e) = demuxer.next_event() {
            events.push(e);
        }

        let sample = events
            .iter()
            .find(|e| {
                matches!(
                    e,
                    DemuxEvent::Sample {
                        payload: SamplePayload::Audio { .. },
                        ..
                    }
                )
            })
            .expect("audio Sample event present");

        if let DemuxEvent::Sample {
            stream,
            pts,
            dts,
            payload: event_payload,
        } = sample
        {
            assert_eq!(stream.pid, 0x300);
            assert!(matches!(stream.kind, StreamKind::Audio(AudioCodec::Aac)));
            assert_eq!(*pts, 90_000);
            assert_eq!(*dts, None, "audio has no DTS");
            if let SamplePayload::Audio { codec, frames } = event_payload {
                assert_eq!(*codec, AudioCodec::Aac);
                assert_eq!(
                    frames.as_slice(),
                    audio_payload.as_slice(),
                    "all payload bytes recovered"
                );
            }
        }
    }

    #[test]
    fn treat_as_overrides_pmt_classification_to_typed_audio() {
        use crate::mpegts::demux::event::AudioCodec;
        use crate::mpegts::mux::{AudioCodec as MuxAudioCodec, ConfigBuilder};

        // Mux an AAC audio stream (PMT stream_type = 0x0F, default classifies
        // as Aac).
        let cfg = ConfigBuilder::default()
            .add_program(1, 0x1000)
            .add_audio(0x300, MuxAudioCodec::Aac)
            .end_program()
            .build()
            .unwrap();
        let mut muxer = crate::mpegts::mux::Muxer::new(cfg).unwrap();
        let audio_payload: Vec<u8> = vec![
            0xFF, 0xF1, 0x4C, 0x80, 0x00, 0x1F, 0xFC, 0xDE, 0xAD, 0xBE, 0xEF,
        ];
        muxer.push_audio(&audio_payload, 90_000).unwrap();
        let mut buf = vec![0u8; 188 * 64];
        let n = muxer.pull(&mut buf);
        buf.truncate(n);

        // Demux WITHOUT treat_as: classification follows PMT → Audio(Aac).
        let mut demuxer = Demuxer::new();
        demuxer.feed(&buf).unwrap();
        demuxer.flush();
        let mut events = Vec::new();
        while let Some(e) = demuxer.next_event() {
            events.push(e);
        }
        let aac_classified = events.iter().any(|e| {
            matches!(
                e,
                DemuxEvent::Sample {
                    payload: SamplePayload::Audio {
                        codec: AudioCodec::Aac,
                        ..
                    },
                    ..
                }
            )
        });
        assert!(aac_classified, "default: PMT classifies as Aac");

        // Demux WITH treat_as override: classifies as Mp2 (override wins).
        let mut options = DemuxerOptions::default();
        options
            .stream_kind_overrides
            .insert(0x300, StreamKind::Audio(AudioCodec::Mp2));
        let mut demuxer = Demuxer::with_options(options);
        demuxer.feed(&buf).unwrap();
        demuxer.flush();
        let mut events = Vec::new();
        while let Some(e) = demuxer.next_event() {
            events.push(e);
        }
        let mp2_overridden = events.iter().any(|e| {
            matches!(
                e,
                DemuxEvent::Sample {
                    payload: SamplePayload::Audio {
                        codec: AudioCodec::Mp2,
                        ..
                    },
                    ..
                }
            )
        });
        assert!(mp2_overridden, "treat_as override: classifies as Mp2");
    }

    #[test]
    fn treat_as_routes_arbitrary_pid_to_subtitle_codec() {
        use crate::mpegts::demux::event::SubtitleCodec as DemuxSubtitleCodec;
        use crate::mpegts::mux::{AudioCodec as MuxAudioCodec, ConfigBuilder};

        // Mux an audio stream on PID 0x200 (PMT stream_type = 0x04 for MP2).
        // The PMT entry will have no subtitle descriptor — but the
        // `stream_kind_overrides` map will remap the PID to
        // `StreamKind::Subtitle(WebVttInTs)`. The demuxer should dispatch
        // through the subtitle arm of `handle_complete_pes` and produce a
        // `SamplePayload::Subtitle` event.
        let cfg = ConfigBuilder::default()
            .add_program(1, 0x1000)
            .add_audio(0x200, MuxAudioCodec::Mp2)
            .end_program()
            .build()
            .unwrap();
        let mut muxer = crate::mpegts::mux::Muxer::new(cfg).unwrap();
        // Body content irrelevant to dispatch — just needs to traverse
        // PES reassembly cleanly. Use a WEBVTT-like header for clarity.
        let payload = b"WEBVTT\n\n00:00.000 --> 00:01.000\nhi\n".to_vec();
        muxer.push_audio(&payload, 90_000).unwrap();
        let mut buf = vec![0u8; 188 * 64];
        let n = muxer.pull(&mut buf);
        buf.truncate(n);

        let mut options = DemuxerOptions::default();
        options
            .stream_kind_overrides
            .insert(0x200, StreamKind::Subtitle(DemuxSubtitleCodec::WebVttInTs));
        let mut demuxer = Demuxer::with_options(options);
        demuxer.feed(&buf).unwrap();
        demuxer.flush();

        let mut got_subtitle = false;
        while let Some(ev) = demuxer.next_event() {
            if let DemuxEvent::Sample {
                stream,
                payload: SamplePayload::Subtitle { codec, .. },
                ..
            } = ev
            {
                if stream.pid == 0x200 && codec == DemuxSubtitleCodec::WebVttInTs {
                    got_subtitle = true;
                }
            }
        }
        assert!(
            got_subtitle,
            "treat_as remap produced subtitle Sample event"
        );
    }

    #[test]
    fn treat_as_routes_subtitle_without_descriptor_emits_non_conformant() {
        use crate::mpegts::demux::event::SubtitleCodec as DemuxSubtitleCodec;
        use crate::mpegts::mux::{AudioCodec as MuxAudioCodec, ConfigBuilder};

        // PMT entry for 0x200 carries no subtitle descriptor. `treat_as`
        // remaps it to a subtitle codec — classifier should surface
        // `NonConformantIssue::SubtitleMissingDescriptor` once for that PID.
        let cfg = ConfigBuilder::default()
            .add_program(1, 0x1000)
            .add_audio(0x200, MuxAudioCodec::Mp2)
            .end_program()
            .build()
            .unwrap();
        let mut muxer = crate::mpegts::mux::Muxer::new(cfg).unwrap();
        muxer.push_audio(b"WEBVTT\n", 90_000).unwrap();
        let mut buf = vec![0u8; 188 * 64];
        let n = muxer.pull(&mut buf);
        buf.truncate(n);

        let mut options = DemuxerOptions::default();
        options
            .stream_kind_overrides
            .insert(0x200, StreamKind::Subtitle(DemuxSubtitleCodec::WebVttInTs));
        let mut demuxer = Demuxer::with_options(options);
        demuxer.feed(&buf).unwrap();
        demuxer.flush();

        let mut got_missing_descriptor = false;
        let mut count = 0;
        while let Some(ev) = demuxer.next_event() {
            if let DemuxEvent::NonConformant {
                issue: NonConformantIssue::SubtitleMissingDescriptor { pid },
                ..
            } = ev
            {
                if pid == 0x200 {
                    got_missing_descriptor = true;
                    count += 1;
                }
            }
        }
        assert!(
            got_missing_descriptor,
            "expected SubtitleMissingDescriptor for PID 0x200"
        );
        assert_eq!(count, 1, "deduped: one event per PID per PMT version");
    }

    // -- classify_0x06: PSI cascade for stream_type 0x06 ----------------------

    fn raw_desc(tag: u8, data: Vec<u8>) -> crate::mpegts::demux::psi::RawDescriptor {
        crate::mpegts::demux::psi::RawDescriptor { tag, data }
    }

    #[test]
    fn classify_0x06_subtitling_descriptor_wins() {
        // subtitling_descriptor (tag 0x59) — body shape per ETSI EN 300 468:
        // ISO 639 lang (3) + subtitling_type (1) + composition_page_id (2) +
        // ancillary_page_id (2). Content irrelevant to classification.
        let descs = vec![raw_desc(
            0x59,
            vec![b'e', b'n', b'g', 0x10, 0x00, 0x01, 0x00, 0x01],
        )];
        assert_eq!(
            classify_0x06(&descs),
            StreamKind::Subtitle(SubtitleCodec::DvbSubtitling)
        );
    }

    #[test]
    fn classify_0x06_teletext_descriptor_wins() {
        // teletext_descriptor (tag 0x56) — body: ISO 639 lang (3) +
        // (teletext_type<<3 | teletext_magazine_number) + teletext_page_number.
        let descs = vec![raw_desc(
            0x56,
            vec![b'e', b'n', b'g', (0x02 << 3) | 1, 0x88],
        )];
        assert_eq!(
            classify_0x06(&descs),
            StreamKind::Subtitle(SubtitleCodec::DvbTeletext)
        );
    }

    #[test]
    fn classify_0x06_vbi_teletext_descriptor_also_classifies_teletext() {
        // VBI_teletext_descriptor (tag 0x46) — same outcome as 0x56.
        let descs = vec![raw_desc(0x46, vec![])];
        assert_eq!(
            classify_0x06(&descs),
            StreamKind::Subtitle(SubtitleCodec::DvbTeletext)
        );
    }

    #[test]
    fn classify_0x06_vttc_format_identifier_classifies_webvtt() {
        let descs = vec![raw_desc(0x05, b"VTTC".to_vec())];
        assert_eq!(
            classify_0x06(&descs),
            StreamKind::Subtitle(SubtitleCodec::WebVttInTs)
        );
    }

    #[test]
    fn classify_0x06_ga94_format_identifier_classifies_cea708_standalone() {
        let descs = vec![raw_desc(0x05, b"GA94".to_vec())];
        assert_eq!(
            classify_0x06(&descs),
            StreamKind::Subtitle(SubtitleCodec::Cea708Standalone)
        );
    }

    #[test]
    fn classify_0x06_klva_still_klv_async_regression_guard() {
        let descs = vec![raw_desc(0x05, b"KLVA".to_vec())];
        assert_eq!(classify_0x06(&descs), StreamKind::KlvAsync);
    }

    #[test]
    fn demuxer_emits_subtitle_sample_for_webvtt_pes() {
        use crate::mpegts::demux::event::SubtitleCodec as DemuxSubtitleCodec;
        use crate::mpegts::mux::{ConfigBuilder, SubtitleCodec as MuxSubtitleCodec, VideoCodec};

        // Mux: single-program with one video stream and one WebVTT subtitle
        // stream. Video is required because Config::validate enforces at least
        // one video or KLV per program; subtitle alone wouldn't be a valid
        // program shape.
        let cfg = ConfigBuilder::default()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_subtitle(0x200, MuxSubtitleCodec::WebVttInTs)
            .end_program()
            .build()
            .unwrap();
        let mut muxer = crate::mpegts::mux::Muxer::new(cfg).unwrap();
        let h = muxer.subtitle_handles()[0];
        let cue = b"WEBVTT\n\nx-cue\n";
        muxer.push_subtitle_to(h, 90_000, cue).unwrap();
        let mut buf = vec![0u8; 188 * 64];
        let n = muxer.pull(&mut buf);
        buf.truncate(n);

        // Demux: feed the muxed bytes, capture events.
        let mut demuxer = Demuxer::new();
        demuxer.feed(&buf).unwrap();
        demuxer.flush();
        let mut events = Vec::new();
        while let Some(e) = demuxer.next_event() {
            events.push(e);
        }

        let sample = events
            .iter()
            .find(|e| {
                matches!(
                    e,
                    DemuxEvent::Sample {
                        payload: SamplePayload::Subtitle { .. },
                        ..
                    }
                )
            })
            .expect("subtitle Sample event present");

        if let DemuxEvent::Sample {
            stream,
            pts,
            dts,
            payload: event_payload,
        } = sample
        {
            assert_eq!(stream.pid, 0x200);
            assert!(matches!(
                stream.kind,
                StreamKind::Subtitle(DemuxSubtitleCodec::WebVttInTs)
            ));
            assert_eq!(*pts, 90_000);
            assert_eq!(*dts, None, "subtitles have no DTS (no B-frame reorder)");
            if let SamplePayload::Subtitle { codec, payload } = event_payload {
                assert_eq!(*codec, DemuxSubtitleCodec::WebVttInTs);
                assert!(
                    payload.windows(6).any(|w| w == b"WEBVTT"),
                    "WEBVTT signature recovered from payload"
                );
            }
        }
    }
}
