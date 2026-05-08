// crates/srt-core/src/mpegts/demux/demuxer.rs
//! Top-level `Demuxer` state machine.

use crate::error::DemuxError;
use crate::mpegts::common::{pcr_diff_27mhz, pts_diff_33bit};
use crate::mpegts::demux::event::{
    AudioCodec, DemuxEvent, DiscontinuityKind, KlvLink, LinkSource, MetadataKind, NalUnit,
    NonConformantIssue, ProgramMap, SamplePayload, StreamId, StreamInfo, StreamKind, SubtitleCodec,
    VideoCodec, VideoPayload,
};
use crate::mpegts::demux::payload::{
    KlvShape, classify_klv, split_nals, split_obus, strip_dvb_sub_envelope,
};
use crate::mpegts::demux::pes::{Reassembler, ReassemblyOutcome};
use crate::mpegts::demux::psi::{
    Pmt, PsiParseError, classify_audio_stream_type, extract_metadata_link, has_klva_registration,
    parse_pat, parse_pmt,
};
use crate::mpegts::demux::psi_assembler::{AssemblerError, PsiSectionAssembler};
use crate::mpegts::demux::strict::StrictMode;
use crate::mpegts::demux::ts::{TsParseError, parse_ts_packet};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

/// Stats snapshot for [`Demuxer`]. Used by
/// [`crate::pipeline::DemuxReceiver`] to compose its own `DemuxReceiverStats`;
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
    /// Number of distinct subtitle PIDs the demuxer has seen at least one
    /// PES sample for. Increments on the first `SamplePayload::Subtitle`
    /// event per PID; resets to zero on `reset_stats`.
    pub subtitle_streams_seen: u32,
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

/// Hard ceiling on `Demuxer::sync_buf`. `feed` always runs
/// `extend_from_slice` before the inner sync-search-window check fires,
/// so an oversized single-call feed (multi-GB of garbage) would otherwise
/// allocate the whole input before the loop got to bail. The 4 MiB cap
/// matches ffmpeg's `MpegTSSectionFilter` ceiling and is comfortably
/// larger than `SYNC_SEARCH_WINDOW` (~6 KiB), so well-formed streams are
/// unaffected.
const MAX_SYNC_BUF_BYTES: usize = 4 << 20;

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
    /// When `true`, PSI section reassembly accepts continuation packets
    /// across continuity-counter jumps (today's permissive behavior —
    /// section either passes by luck or fails CRC). Default `false` is
    /// strict-correctness: drop the partial section on jump and emit
    /// `NonConformantIssue::PsiCcDiscontinuity`. Matches ffmpeg
    /// `mpegts.c:3118-3142`.
    pub lenient_psi_reassembly: bool,
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
    /// Per-PID PSI assembly state (PAT + any active PMT PIDs). Each
    /// assembler enforces the 4 KiB `MAX_SECTION_SIZE` cap and yields a
    /// complete section once `section_length + 3` bytes have been
    /// accumulated for that PID. See `psi_assembler.rs`.
    psi_assemblers: HashMap<u16, PsiSectionAssembler>,
    /// Programs found in the current PAT, keyed by `pmt_pid`.
    /// O(1) lookup when routing PMT-bound packets.
    programs: HashMap<u16, ProgramTracker>,
    /// Latest PAT version. Bump triggers PAT diff (programs added/removed).
    pat_version: Option<u8>,
    /// Per-PID stream kind cache for PES dispatch. Flat across all programs
    /// (PIDs must be unique cross-program per ISO 13818-1).
    stream_kind_by_pid: HashMap<u16, StreamKind>,
    cc_by_pid: HashMap<u16, u8>,
    /// Captured (expected, observed) CC pair when `check_continuity`
    /// flagged a real jump on the packet currently being routed. Drained
    /// by `handle_psi` when it consumes the strict-mode drop arm; cleared
    /// at the top of every `check_continuity` call so PSI packets without
    /// a jump don't carry stale state.
    last_psi_cc_jump: Option<(u8, u8)>,
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
    subtitle_streams_seen_count: u32,
    /// Per-PID counters; entries created lazily on first event per PID.
    stats_per_stream: BTreeMap<u16, crate::mpegts::stats::StreamStats>,
    /// PIDs that have already emitted `SubtitleMissingDescriptor` for the
    /// current PMT version. Cleared at the top of each PMT-version bump so
    /// a fresh PMT re-fires if the descriptor is still missing.
    subtitle_missing_descriptor_emitted: HashSet<u16>,
    /// PIDs the demuxer has emitted at least one `SamplePayload::Subtitle`
    /// event for. Used to dedupe `subtitle_streams_seen` increments so
    /// repeat samples on the same PID don't double-count. Cleared on
    /// `reset_stats`.
    subtitle_pids_seen: HashSet<u16>,
    /// PIDs that have already emitted `Av1RegistrationMalformed` for the
    /// current PMT version. Cleared at the top of each PMT-version bump so
    /// a fresh PMT re-fires if the malformed registration is still present.
    av1_registration_malformed_emitted: HashSet<u16>,
    /// PIDs that have already emitted `SubtitleDescriptorAmbiguous` for the
    /// current PMT version. Cleared at the top of each PMT-version bump so
    /// a fresh PMT re-fires if the ambiguity is still present.
    subtitle_descriptor_ambiguous_emitted: HashSet<u16>,
}

impl Demuxer {
    pub fn new() -> Self {
        Self::with_options(DemuxerOptions::default())
    }

    pub fn with_options(options: DemuxerOptions) -> Self {
        let cap_per_pid = options.pes_cap_per_pid.unwrap_or(DEFAULT_PES_CAP_PER_PID);
        let cap_total = options.pes_cap_total.unwrap_or(DEFAULT_PES_CAP_TOTAL);
        // Seed the PAT PID (0x0000) so the PSI assembler is ready without a
        // separate "first packet" initialisation step.
        let mut psi_assemblers: HashMap<u16, PsiSectionAssembler> = HashMap::new();
        psi_assemblers.insert(0x0000, PsiSectionAssembler::new());
        Self {
            options,
            sync_buf: Vec::new(),
            sync_consumed: 0,
            psi_assemblers,
            programs: HashMap::new(),
            pat_version: None,
            stream_kind_by_pid: HashMap::new(),
            cc_by_pid: HashMap::new(),
            last_psi_cc_jump: None,
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
            subtitle_streams_seen_count: 0,
            stats_per_stream: BTreeMap::new(),
            subtitle_missing_descriptor_emitted: HashSet::new(),
            subtitle_pids_seen: HashSet::new(),
            av1_registration_malformed_emitted: HashSet::new(),
            subtitle_descriptor_ambiguous_emitted: HashSet::new(),
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
        // Enforce the hard ceiling immediately — the inner sync-search-window
        // check below only fires per loop iteration, but the `extend_from_slice`
        // above has already allocated the entire input. A single oversized
        // adversarial feed must be rejected before we walk the buffer.
        if self.sync_buf.len() > MAX_SYNC_BUF_BYTES {
            let observed = self.sync_buf.len();
            // Defensive: once the cap is exceeded, the parser is in a known-bad
            // state and we should release the adversarial bytes. Subsequent
            // feed calls will start from an empty buffer; if the peer is still
            // hostile, they'll trip the cap again. The caller's only sane
            // response is to teardown the demuxer.
            self.sync_buf.clear();
            self.sync_consumed = 0;
            return Err(DemuxError::SyncBufExhausted {
                observed,
                max: MAX_SYNC_BUF_BYTES,
            });
        }
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
        // ISO/IEC 13818-1 §2.4.3.2: `transport_error_indicator=1` means an
        // upstream link-layer flagged the packet as known-corrupt. ffmpeg
        // drops these and flags AV_PKT_FLAG_CORRUPT (mpegts.c:3091-3097);
        // feeding the payload to PES/PSI reassembly would corrupt downstream
        // parse state. Drop entirely and surface the drop as non-conformant
        // so consumers can correlate with downstream parse failures.
        if pkt.transport_error_indicator {
            let stream = self.lookup_stream(pkt.pid).unwrap_or(StreamId {
                pid: pkt.pid,
                kind: StreamKind::Unknown(0),
            });
            self.queue_nonconformant(
                stream,
                NonConformantIssue::TransportErrorPacket { pid: pkt.pid },
            );
            return Ok(());
        }
        self.check_pcr(&pkt);
        let cc_jumped = self.check_continuity(&pkt);
        if pkt.pid == 0x0000 {
            self.handle_psi(
                pkt.pid,
                pkt.payload,
                pkt.payload_unit_start,
                true,
                cc_jumped,
            )?;
        } else if self.programs.contains_key(&pkt.pid) {
            self.handle_psi(
                pkt.pid,
                pkt.payload,
                pkt.payload_unit_start,
                false,
                cc_jumped,
            )?;
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

    /// Returns `true` if a CC jump was observed AND not suppressed by
    /// `discontinuity_indicator`. The caller (`process_packet`) uses this
    /// signal to gate strict-mode PSI reassembly drops in `handle_psi`.
    ///
    /// Side effect: clears `self.last_psi_cc_jump` at entry, sets it to
    /// `Some((expected, observed))` when a real jump fires. `handle_psi`
    /// drains it via `.take()` when emitting `PsiCcDiscontinuity`.
    fn check_continuity(&mut self, pkt: &crate::mpegts::demux::ts::TsPacket<'_>) -> bool {
        self.last_psi_cc_jump = None;
        if !pkt.has_payload {
            return false;
        }
        let mut real_jump = false;
        if let Some(prev_cc) = self.cc_by_pid.get(&pkt.pid).copied() {
            let expected = (prev_cc + 1) & 0x0F;
            // Per ISO/IEC 13818-1 §2.4.3.5, when discontinuity_indicator=1
            // the CC is explicitly permitted to be discontinuous on this
            // packet. Suppress the ContinuityJump (matches ffmpeg
            // mpegts.c:3075-3078); the separate `AdaptationFieldFlag`
            // event below already surfaces the discontinuity hint to
            // consumers, so emitting both would double-count.
            if expected != pkt.continuity_counter && !pkt.discontinuity_indicator {
                real_jump = true;
                self.last_psi_cc_jump = Some((expected, pkt.continuity_counter));
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
        real_jump
    }

    fn handle_psi(
        &mut self,
        pid: u16,
        payload: &[u8],
        pusi: bool,
        is_pat: bool,
        cc_jumped: bool,
    ) -> Result<(), DemuxError> {
        // Strict mode: drop the partial section if a continuation packet
        // arrives with a CC jump (matches ffmpeg `mpegts.c:3118-3142`).
        // Lenient mode (opt-in via DemuxerOptions::lenient_psi_reassembly)
        // preserves today's permissive behavior — bytes are accumulated
        // regardless, either passing CRC by luck or surfacing as
        // PsiChecksumMismatch.
        if !pusi && cc_jumped && !self.options.lenient_psi_reassembly {
            if let Some(assembler) = self.psi_assemblers.get_mut(&pid) {
                assembler.reset();
            }
            let (expected, observed) = self
                .last_psi_cc_jump
                .take()
                .expect("check_continuity populated last_psi_cc_jump on real jump");
            let stream = self.lookup_stream(pid).unwrap_or(StreamId {
                pid,
                kind: StreamKind::Unknown(0),
            });
            self.queue_nonconformant(
                stream,
                NonConformantIssue::PsiCcDiscontinuity {
                    pid,
                    expected,
                    observed,
                },
            );
            return Ok(());
        }

        let assembler = self.psi_assemblers.entry(pid).or_default();

        let result = if pusi {
            // First byte after pointer_field marks where the section starts.
            if payload.is_empty() {
                return Ok(());
            }
            let pointer_field = payload[0] as usize;
            if 1 + pointer_field > payload.len() {
                return Ok(());
            }
            assembler.start_new_section(&payload[1 + pointer_field..])
        } else {
            assembler.append_continuation(payload)
        };

        let section = match result {
            Ok(Some(section)) => section,
            Ok(None) => return Ok(()),
            Err(AssemblerError::Overflow { observed_len })
            | Err(AssemblerError::DeclaredTooLong {
                declared_len: observed_len,
            }) => {
                // Cap fired — partial section discarded by the assembler. Surface
                // the overflow as a NonConformant event keyed to this PSI PID so
                // the caller can detect the DoS attempt without losing the
                // receive loop.
                let stream = self.lookup_stream(pid).unwrap_or(StreamId {
                    pid,
                    kind: StreamKind::Unknown(0),
                });
                self.queue_nonconformant(
                    stream,
                    NonConformantIssue::PsiOverlongSection { pid, observed_len },
                );
                return Ok(());
            }
        };

        if is_pat {
            self.handle_pat_section(&section);
        } else {
            self.handle_pmt_section(pid, &section);
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
                self.psi_assemblers.remove(&pmt_pid);
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
            // Seed the PSI assembler for this PMT PID so handle_psi can
            // accumulate bytes without a separate "first packet" init step.
            self.psi_assemblers.entry(entry.pid).or_default();
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
        // and Av1RegistrationMalformed emission so that a new PMT version
        // re-fires the warning if the descriptor is still absent / still
        // malformed. Only PIDs owned by *this* PMT are dropped to leave any
        // other program's state intact.
        for s in &pmt.streams {
            self.subtitle_missing_descriptor_emitted
                .remove(&s.elementary_pid);
            self.av1_registration_malformed_emitted
                .remove(&s.elementary_pid);
            self.subtitle_descriptor_ambiguous_emitted
                .remove(&s.elementary_pid);
        }

        // Build StreamInfo list + check cross-program PID collisions.
        // Collect work to do before mutating self — satisfies borrow checker.
        let mut stream_infos: Vec<StreamInfo> = Vec::with_capacity(pmt.streams.len());
        let mut kind_inserts: Vec<(u16, StreamKind)> = Vec::new();
        let mut collision_issues: Vec<(StreamId, NonConformantIssue)> = Vec::new();
        let mut subtitle_missing: Vec<(u16, StreamKind)> = Vec::new();
        let mut av1_malformed: Vec<(u16, StreamKind)> = Vec::new();
        let mut subtitle_ambiguous: Vec<(u16, StreamKind, Vec<u8>)> = Vec::new();

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

            // stream_type 0x06 entries that contain a Registration descriptor
            // whose body looks like a truncated AV01 attempt (starts with
            // "AV", < 4 bytes total) — fires only when classify_0x06 didn't
            // already return Video(Av1), i.e. the malformed registration
            // didn't match `b"AV01"` exactly. Outer length-vs-buffer overflow
            // is already caught by walk_descriptors as DescriptorLoopOverflow;
            // this is the in-bounds variant.
            if s.stream_type == 0x06
                && !matches!(kind, StreamKind::Video(VideoCodec::Av1))
                && is_malformed_av1_registration(&s.descriptors)
            {
                av1_malformed.push((s.elementary_pid, kind));
            }

            // stream_type 0x06 entries with more than one recognized
            // subtitle/KLV codec marker — classification cascade still
            // picks the highest-priority match (subtitling > teletext >
            // VTTC > GA94 > KLVA), but the ambiguity is surfaced for
            // diagnostics. Only checked on 0x06 since the other stream
            // types disambiguate by stream_type alone. AV1 wins
            // exclusively (binding §2.1) so AV01 alongside other markers
            // is not flagged here — classify_0x06 returned Video(Av1).
            if s.stream_type == 0x06 && !matches!(kind, StreamKind::Video(VideoCodec::Av1)) {
                let (_, ambiguous_tags) = classify_0x06_with_ambiguity(&s.descriptors);
                if !ambiguous_tags.is_empty() {
                    subtitle_ambiguous.push((s.elementary_pid, kind, ambiguous_tags));
                }
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

        // Emit Av1RegistrationMalformed once per PID per fresh PMT version.
        for (pid, kind) in av1_malformed {
            if self.av1_registration_malformed_emitted.insert(pid) {
                self.queue_nonconformant(
                    StreamId { pid, kind },
                    NonConformantIssue::Av1RegistrationMalformed { pid },
                );
            }
        }

        // Emit SubtitleDescriptorAmbiguous once per PID per fresh PMT version.
        for (pid, kind, tags) in subtitle_ambiguous {
            if self.subtitle_descriptor_ambiguous_emitted.insert(pid) {
                self.queue_nonconformant(
                    StreamId { pid, kind },
                    NonConformantIssue::SubtitleDescriptorAmbiguous { pid, tags },
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
            0x33 => StreamKind::Video(VideoCodec::H266),
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
                // Codec dispatches the payload-shape: H.26x splits Annex-B NAL
                // units (split_nals); AV1 splits OBUs (split_obus). The two
                // share the same Sample event surface but emit different
                // VideoPayload variants — the invariant is documented on
                // VideoPayload.
                let (sample, payload_bytes) = match codec {
                    VideoCodec::H264 | VideoCodec::H265 | VideoCodec::H266 => {
                        let nals = split_nals(&pes.payload, codec);
                        let bytes = nal_payload_bytes(&nals);
                        (
                            SamplePayload::Video {
                                codec,
                                payload: VideoPayload::Nals(nals),
                            },
                            bytes,
                        )
                    }
                    VideoCodec::Av1 => {
                        let (obus, mut issues) = split_obus(&pes.payload);
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
                                _ => {}
                            }
                        }
                        for issue in issues {
                            self.queue_nonconformant(stream, issue);
                        }
                        let bytes: usize = obus.iter().map(|o| o.payload.len()).sum();
                        (
                            SamplePayload::Video {
                                codec,
                                payload: VideoPayload::Obus(obus),
                            },
                            bytes,
                        )
                    }
                };
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
                if self.subtitle_pids_seen.insert(stream.pid) {
                    self.subtitle_streams_seen_count += 1;
                }
                let entry = self.stats_per_stream.entry(stream.pid).or_insert_with(|| {
                    crate::mpegts::stats::StreamStats {
                        pid: stream.pid,
                        stream_type: stream_type_from_kind(&stream.kind),
                        program_number,
                        label: Some(
                            crate::mpegts::stats::demux_subtitle_codec_label(codec).to_string(),
                        ),
                        ..Default::default()
                    }
                });
                entry.items += 1;
                entry.bytes += payload_len as u64;
                // For DVB subtitling, strip the EN 300 743 §6.2 PES_data_field
                // envelope (data_identifier + subtitle_stream_id + segments +
                // 0xFF end_marker) so callers see just the segment bytes —
                // matching what libavcodec's dvbsubdec expects (it rejects
                // anything that doesn't begin with a segment sync_byte 0x0F).
                // Other subtitle codecs (teletext, CEA-708 standalone, WebVTT)
                // do not have this wrapper; pass through verbatim.
                let raw = &pes.payload;
                let surfaced_payload = match codec {
                    SubtitleCodec::DvbSubtitling => strip_dvb_sub_envelope(raw)
                        .map(|s| s.to_vec())
                        .unwrap_or_else(|| raw.to_vec()),
                    _ => raw.to_vec(),
                };
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
            subtitle_streams_seen: self.subtitle_streams_seen_count,
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
        self.subtitle_streams_seen_count = 0;
        self.subtitle_pids_seen.clear();
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
            NalUnit::H264 { payload, .. }
            | NalUnit::H265 { payload, .. }
            | NalUnit::H266 { payload, .. } => payload.len(),
        })
        .sum()
}

/// Map a `StreamKind` to its MPEG-TS `stream_type` byte (PMT value).
///
/// Used for `StreamStats.stream_type` labelling on the receiver side; not
/// emitted on the wire (the demuxer reads stream_type from the PMT). See
/// `mpegts::common::StreamType` for the canonical mux-side encoding.
fn stream_type_from_kind(k: &StreamKind) -> u8 {
    match k {
        StreamKind::Video(VideoCodec::H264) => 0x1B,
        StreamKind::Video(VideoCodec::H265) => 0x24,
        StreamKind::Video(VideoCodec::H266) => 0x33,
        // AV1 rides stream_type 0x06 (PES private data) plus an AV01
        // registration_descriptor in the PMT.
        StreamKind::Video(VideoCodec::Av1) => 0x06,
        StreamKind::Audio(AudioCodec::Mp2) => 0x03,
        StreamKind::Audio(AudioCodec::Aac) => 0x0F,
        StreamKind::Audio(AudioCodec::AacLatm) => 0x11,
        StreamKind::Audio(AudioCodec::Ac3) => 0x81,
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
    // AV1 in MPEG-2 TS binding §2.1: format_identifier = "AV01".
    // AV01 registration is exclusive — wins over any other descriptor.
    if find_format_identifier(descriptors, b"AV01") {
        return StreamKind::Video(VideoCodec::Av1);
    }
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

/// Same as [`classify_0x06`] but also returns the list of recognized
/// subtitle/KLV codec markers found on the PID — empty if there's no
/// ambiguity (zero or one marker), populated if more than one was found.
///
/// Tag list encoding mirrors [`NonConformantIssue::SubtitleDescriptorAmbiguous`]:
/// descriptor tag bytes for tag-presence matches (0x59 / 0x56 / 0x46),
/// synthetic codepoints for `format_identifier` matches (0xF0=VTTC,
/// 0xF1=GA94, 0xF2=KLVA). The classification result follows the existing
/// first-match priority — only the diagnostic tag list changes.
fn classify_0x06_with_ambiguity(
    descriptors: &[crate::mpegts::demux::psi::RawDescriptor],
) -> (StreamKind, Vec<u8>) {
    use crate::mpegts::descriptors::{find_descriptor_tag, find_format_identifier};
    let mut markers: Vec<u8> = Vec::new();
    if find_descriptor_tag(descriptors, 0x59) {
        markers.push(0x59);
    }
    // 0x56 and 0x46 are sibling teletext tags — count as one marker so
    // a stream carrying both doesn't trip ambiguity on the teletext side.
    if find_descriptor_tag(descriptors, 0x56) {
        markers.push(0x56);
    } else if find_descriptor_tag(descriptors, 0x46) {
        markers.push(0x46);
    }
    if find_format_identifier(descriptors, b"VTTC") {
        markers.push(0xF0);
    }
    if find_format_identifier(descriptors, b"GA94") {
        markers.push(0xF1);
    }
    if find_format_identifier(descriptors, b"KLVA") {
        markers.push(0xF2);
    }
    let kind = classify_0x06(descriptors);
    let ambiguous = if markers.len() <= 1 {
        Vec::new()
    } else {
        markers
    };
    (kind, ambiguous)
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

/// True iff `descriptors` contains a Registration descriptor that
/// LOOKS like an attempted AV1 (`AV01`) registration but is truncated.
/// Specifically: a descriptor with `tag == 0x05`, body length < 4 bytes,
/// and body starts with `b"AV"`. Outer length-vs-buffer overflow would
/// already error via `PsiParseError::DescriptorLoopOverflow` at walk
/// time; this catches the subtler case where the descriptor is
/// well-formed but its body can't be a valid 4-byte format_identifier.
///
/// Used by the demuxer to surface `NonConformantIssue::Av1RegistrationMalformed`
/// from the PMT processing path. Lenient mode silently still falls
/// through to `StreamKind::Unknown(0x06)` from the standard cascade;
/// strict mode (`StrictMode::Full`) converts the issue to a fatal
/// `DemuxError::StrictRejection`.
fn is_malformed_av1_registration(descriptors: &[crate::mpegts::demux::psi::RawDescriptor]) -> bool {
    descriptors
        .iter()
        .any(|d| d.tag == 0x05 && d.data.len() < 4 && d.data.starts_with(b"AV"))
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
    fn demuxer_options_default_strict_psi_reassembly() {
        let opts = DemuxerOptions::default();
        assert!(
            !opts.lenient_psi_reassembly,
            "default is strict (per ffmpeg parity); opt-in lenient via lenient_psi_reassembly=true"
        );
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

    /// Per-codec dispatch in stats `stream_type` byte. Prior code returned
    /// 0x0F (ADTS AAC) for every audio kind; MP2/LATM/AC-3 streams misreported
    /// in StreamStats.
    #[test]
    fn stream_type_from_kind_per_audio_codec() {
        assert_eq!(
            stream_type_from_kind(&StreamKind::Audio(AudioCodec::Mp2)),
            0x03
        );
        assert_eq!(
            stream_type_from_kind(&StreamKind::Audio(AudioCodec::Aac)),
            0x0F
        );
        assert_eq!(
            stream_type_from_kind(&StreamKind::Audio(AudioCodec::AacLatm)),
            0x11
        );
        assert_eq!(
            stream_type_from_kind(&StreamKind::Audio(AudioCodec::Ac3)),
            0x81
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

    #[test]
    fn multi_descriptor_0x06_emits_subtitle_ambiguous() {
        use crate::mpegts::mux::{
            ConfigBuilder, SubtitleCodec as MuxSubtitleCodec, VideoCodec as MuxVideoCodec,
        };

        // Caller supplies BOTH a subtitling_descriptor (0x59) and a
        // VTTC registration_descriptor on the same WebVTT subtitle PID.
        // The mux auto-emit suppresses on either marker, so caller has
        // to bypass that path by stuffing both into the descriptor list
        // directly. Two recognized subtitle codec markers on the same
        // 0x06 PID — the classifier still picks subtitling per first-match
        // priority, and the demuxer surfaces the ambiguity for diagnostics.
        // subtitling_descriptor body: ISO 639 lang (3) + subtitling_type
        // (1) + composition_page_id (2) + ancillary_page_id (2).
        let subtitling_tlv = vec![0x59u8, 0x08, b'e', b'n', b'g', 0x10, 0x00, 0x01, 0x00, 0x01];
        // registration_descriptor body: 4-byte format_identifier "VTTC".
        let vttc_tlv = vec![0x05u8, 0x04, b'V', b'T', b'T', b'C'];
        let cfg = ConfigBuilder::default()
            .add_program(1, 0x1000)
            .add_video(0x101, MuxVideoCodec::H264)
            .add_subtitle(0x200, MuxSubtitleCodec::WebVttInTs)
            .stream_descriptors_for_subtitle(0, vec![subtitling_tlv, vttc_tlv])
            .end_program()
            .build()
            .unwrap();
        let mut muxer = crate::mpegts::mux::Muxer::new(cfg).unwrap();
        // Push something to force PSI emission.
        let h = muxer.subtitle_handles()[0];
        muxer.push_subtitle_to(h, 90_000, b"WEBVTT\n\nx\n").unwrap();
        let mut buf = vec![0u8; 188 * 64];
        let n = muxer.pull(&mut buf);
        buf.truncate(n);

        let mut demuxer = Demuxer::new();
        demuxer.feed(&buf).unwrap();
        demuxer.flush();

        let mut got_ambiguous = false;
        let mut count = 0;
        while let Some(ev) = demuxer.next_event() {
            if let DemuxEvent::NonConformant {
                issue: NonConformantIssue::SubtitleDescriptorAmbiguous { pid, tags },
                ..
            } = ev
            {
                if pid == 0x200 {
                    got_ambiguous = true;
                    count += 1;
                    // 0x59 (subtitling) priority before 0xF0 (synthetic VTTC).
                    assert_eq!(tags, vec![0x59, 0xF0]);
                }
            }
        }
        assert!(
            got_ambiguous,
            "expected SubtitleDescriptorAmbiguous for PID 0x200"
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
    fn classify_0x06_av1_registration_takes_priority() {
        let descs = vec![raw_desc(0x05, b"AV01".to_vec())];
        assert_eq!(classify_0x06(&descs), StreamKind::Video(VideoCodec::Av1));
    }

    #[test]
    fn classify_0x06_av01_wins_over_klva_and_subtitle_arms() {
        // AV01 registration alongside (hypothetical, spec-violating) KLVA
        // registration — AV01 wins per descriptor exclusivity (binding §2.1).
        let descs = vec![
            raw_desc(0x05, b"AV01".to_vec()),
            raw_desc(0x05, b"KLVA".to_vec()),
        ];
        assert_eq!(classify_0x06(&descs), StreamKind::Video(VideoCodec::Av1));
    }

    #[test]
    fn classify_0x06_emits_ambiguous_on_subtitling_plus_vttc() {
        // PMT entry with both subtitling_descriptor (0x59) AND VTTC
        // registration — ambiguous which subtitle codec actually rides on
        // the PID. Classifier still picks subtitling per first-match
        // priority; ambiguity helper surfaces both markers.
        let descs = vec![
            raw_desc(0x59, vec![b'e', b'n', b'g', 0x10, 0x00, 0x01, 0x00, 0x01]),
            raw_desc(0x05, b"VTTC".to_vec()),
        ];
        let (kind, ambiguous_tags) = classify_0x06_with_ambiguity(&descs);
        assert_eq!(kind, StreamKind::Subtitle(SubtitleCodec::DvbSubtitling));
        assert_eq!(
            ambiguous_tags,
            vec![0x59, 0xF0],
            "ambiguity reports both 0x59 and synthetic 0xF0 (VTTC)"
        );
    }

    #[test]
    fn classify_0x06_no_ambiguity_when_single_marker() {
        let descs = vec![raw_desc(0x05, b"VTTC".to_vec())];
        let (kind, ambiguous_tags) = classify_0x06_with_ambiguity(&descs);
        assert_eq!(kind, StreamKind::Subtitle(SubtitleCodec::WebVttInTs));
        assert!(ambiguous_tags.is_empty());
    }

    #[test]
    fn classify_0x06_no_ambiguity_when_no_markers() {
        // No recognized markers — empty tag list, falls through to Unknown.
        let descs: Vec<crate::mpegts::demux::psi::RawDescriptor> = vec![];
        let (kind, ambiguous_tags) = classify_0x06_with_ambiguity(&descs);
        assert_eq!(kind, StreamKind::Unknown(0x06));
        assert!(ambiguous_tags.is_empty());
    }

    #[test]
    fn classify_0x06_ambiguous_teletext_synonyms_count_once() {
        // 0x56 and 0x46 are teletext synonyms — one marker total even when
        // both are present. Combined with VTTC, that's two markers.
        let descs = vec![
            raw_desc(0x56, vec![b'e', b'n', b'g', (0x02 << 3) | 1, 0x88]),
            raw_desc(0x46, vec![]),
            raw_desc(0x05, b"VTTC".to_vec()),
        ];
        let (kind, ambiguous_tags) = classify_0x06_with_ambiguity(&descs);
        assert_eq!(kind, StreamKind::Subtitle(SubtitleCodec::DvbTeletext));
        assert_eq!(ambiguous_tags, vec![0x56, 0xF0]);
    }

    // -- is_malformed_av1_registration -----------------------------------------

    #[test]
    fn malformed_av01_registration_truncated_to_three_bytes() {
        // Tag 0x05, length 3 (well-formed at walk time), contents "AV0".
        // This is the "tried to be AV01 but truncated" case.
        let descs = vec![raw_desc(0x05, b"AV0".to_vec())];
        assert!(is_malformed_av1_registration(&descs));
    }

    #[test]
    fn well_formed_av01_registration_not_flagged() {
        let descs = vec![raw_desc(0x05, b"AV01".to_vec())];
        assert!(!is_malformed_av1_registration(&descs));
    }

    #[test]
    fn other_short_registration_not_flagged() {
        // KLVA truncated — doesn't start with "AV" so not flagged.
        let descs = vec![raw_desc(0x05, b"KL".to_vec())];
        assert!(!is_malformed_av1_registration(&descs));
    }

    #[test]
    fn empty_descriptors_not_flagged() {
        assert!(!is_malformed_av1_registration(&[]));
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

    #[test]
    fn demuxer_stats_increments_subtitle_streams_seen() {
        use crate::mpegts::mux::{ConfigBuilder, SubtitleCodec, VideoCodec as MuxVideoCodec};

        let cfg = ConfigBuilder::default()
            .add_program(1, 0x100)
            .add_video(0x101, MuxVideoCodec::H264)
            .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
            .end_program()
            .build()
            .unwrap();
        let mut mux = crate::mpegts::mux::Muxer::new(cfg).unwrap();
        let h = mux.subtitle_handles()[0];
        // Push twice on the same PID — the dedupe HashSet should keep
        // subtitle_streams_seen at 1 (one distinct PID seen).
        mux.push_subtitle_to(h, 90_000, b"WEBVTT\n").unwrap();
        mux.push_subtitle_to(h, 180_000, b"WEBVTT\n\n").unwrap();
        let mut bytes = vec![0u8; 188 * 64];
        let n = mux.pull(&mut bytes);
        bytes.truncate(n);

        let mut demux = Demuxer::new();
        demux.feed(&bytes).unwrap();
        demux.flush();
        while demux.next_event().is_some() {}
        let s = demux.stats();
        assert_eq!(s.subtitle_streams_seen, 1);
        let stream_stat = s.per_stream.get(&0x200).unwrap();
        assert_eq!(stream_stat.label.as_deref(), Some("WebVTT-in-TS"));
        assert!(stream_stat.items >= 1);

        // reset_stats clears the dedup set + counter so a fresh sample
        // would re-bump.
        demux.reset_stats();
        let s = demux.stats();
        assert_eq!(s.subtitle_streams_seen, 0);
    }

    /// A malicious PMT PUSI claiming `section_length=0xFFF` (4095 bytes,
    /// total = 4098 > 4096 cap) and never closing must NOT cause unbounded
    /// buffer growth — ffmpeg caps at 4096 (`MAX_SECTION_SIZE` in
    /// `mpegts.h:34`). On overflow we discard the partial section and emit
    /// `NonConformantIssue::PsiOverlongSection`.
    ///
    /// We pre-seed the demuxer with a PAT that maps program 1 → PMT PID
    /// 0x100 so that `process_packet` routes the malicious packet to the
    /// PMT-PID branch of `handle_psi`. Without this, packets on PID 0x100
    /// would be silently ignored (PID not in PAT, not in stream_kind_by_pid).
    #[test]
    fn psi_buffer_caps_at_4kib_and_emits_psi_overlong_section() {
        let mut demux = Demuxer::new();

        // ── Step 1: feed a PAT that announces program 1 on PMT PID 0x100. ──
        // PAT section bytes (per ISO 13818-1 §2.4.4.3):
        //   table_id=0x00,
        //   syntax=1, '0', reserved=11, section_length (12 bits),
        //   transport_stream_id (16 bits),
        //   reserved=11, version=0, current_next=1,
        //   section_number=0, last_section_number=0,
        //   { program_number=1, reserved=111, pid=0x100 } * 1,
        //   CRC32 (4 bytes).
        // section_length spans from after section_length itself through CRC,
        // so = 5 (header tail) + 4 (program loop) + 4 (CRC) = 13 (0x0D).
        let mut pat = vec![
            0x00, // table_id
            0xB0, 0x0D, // syntax=1, section_length=0x00D
            0x00, 0x01, // transport_stream_id=1
            0xC1, // reserved=11, version=0, current_next=1
            0x00, 0x00, // section_number, last_section_number
            0x00, 0x01, // program_number=1
            0xE1, 0x00, // reserved=111, pid=0x100
        ];
        let crc = crate::mpegts::common::crc32::crc32_mpeg2(&pat);
        pat.extend_from_slice(&crc.to_be_bytes());

        // Wrap PAT into a single TS packet on PID 0x0000.
        let mut ts_pat = [0xFFu8; 188];
        ts_pat[0] = 0x47;
        ts_pat[1] = 0x40; // PUSI=1, pid_high=0
        ts_pat[2] = 0x00; // pid_low=0
        ts_pat[3] = 0x10; // adaptation=01, cc=0
        ts_pat[4] = 0x00; // pointer_field=0
        ts_pat[5..5 + pat.len()].copy_from_slice(&pat);
        demux.feed(&ts_pat).expect("PAT feed");

        // Drain any events emitted by the PAT (no PMT yet → no ProgramMap).
        while demux.next_event().is_some() {}

        // ── Step 2: feed N packets on PMT PID 0x100 — first PUSI claims a
        // section_length=0xFFF (4095 bytes), total declared length 4098 >
        // 4096 cap. The assembler should reject this on the FIRST packet
        // (declared_too_long path).
        let mut ts_pmt = [0xFFu8; 188];
        ts_pmt[0] = 0x47;
        ts_pmt[1] = 0x41; // PUSI=1, pid_high=1
        ts_pmt[2] = 0x00; // pid_low=0 → PID = 0x100
        ts_pmt[3] = 0x10; // adaptation=01, cc=0
        ts_pmt[4] = 0x00; // pointer_field=0
        // table_id=0x02 (PMT), syntax=1 + section_length=0xFFF (4095).
        ts_pmt[5] = 0x02;
        ts_pmt[6] = 0xBF; // 1011 1111 → syntax=1, '0'=0, reserved=11, sl_hi=0xF
        ts_pmt[7] = 0xFF; // sl_lo=0xFF → section_length=0xFFF=4095
        // Remaining 181 bytes are arbitrary attacker payload.

        demux.feed(&ts_pmt).expect("malicious PMT feed");

        // ── Step 3: assert PsiOverlongSection emitted. ──
        let mut saw_overlong = false;
        while let Some(ev) = demux.next_event() {
            if let DemuxEvent::NonConformant {
                issue: NonConformantIssue::PsiOverlongSection { pid, observed_len },
                ..
            } = ev
            {
                assert_eq!(pid, 0x100);
                assert_eq!(observed_len, 4098); // 3 + 0xFFF
                saw_overlong = true;
            }
        }
        assert!(
            saw_overlong,
            "expected PsiOverlongSection on PID 0x100 from malicious PMT"
        );
    }

    /// Per ISO/IEC 13818-1 §2.4.3.2, bit 0x80 of byte 1 (transport_error_indicator)
    /// marks a packet as link-layer-corrupt. ffmpeg drops these
    /// (mpegts.c:3091-3097); we must too — silently feeding the payload to
    /// PES/PSI reassembly produces garbage parse output downstream.
    ///
    /// We pre-seed the demuxer with a PAT that maps program 1 → PMT PID 0x100
    /// so that without the TEI drop, the packet would route to the PMT-PID
    /// branch of `handle_psi` and PSI parsing would be observable.
    #[test]
    fn tei_packets_are_dropped_with_non_conformant_event() {
        let mut demux = Demuxer::new();

        // Seed a PAT that announces program 1 on PMT PID 0x100. (Same
        // construction as in `psi_buffer_caps_at_4kib_and_emits_psi_overlong_section`.)
        let mut pat = vec![
            0x00, // table_id
            0xB0, 0x0D, // syntax=1, section_length=0x00D
            0x00, 0x01, // transport_stream_id=1
            0xC1, // reserved=11, version=0, current_next=1
            0x00, 0x00, // section_number, last_section_number
            0x00, 0x01, // program_number=1
            0xE1, 0x00, // reserved=111, pid=0x100
        ];
        let crc = crate::mpegts::common::crc32::crc32_mpeg2(&pat);
        pat.extend_from_slice(&crc.to_be_bytes());

        let mut ts_pat = [0xFFu8; 188];
        ts_pat[0] = 0x47;
        ts_pat[1] = 0x40; // PUSI=1, pid_high=0
        ts_pat[2] = 0x00; // pid_low=0
        ts_pat[3] = 0x10; // adaptation=01, cc=0
        ts_pat[4] = 0x00; // pointer_field=0
        ts_pat[5..5 + pat.len()].copy_from_slice(&pat);
        demux.feed(&ts_pat).expect("PAT feed");

        // Drain PAT-induced events.
        while demux.next_event().is_some() {}

        // Build a TS packet with PUSI=1, CC=0, PID=0x100, AND TEI=1.
        let mut pkt = [0xFFu8; 188];
        pkt[0] = 0x47;
        pkt[1] = 0xC1; // TEI=1, PUSI=1, transport_priority=0, pid_high=1
        pkt[2] = 0x00; // pid_low=0 → PID=0x100
        pkt[3] = 0x10; // adaptation=01, cc=0
        pkt[4] = 0x00; // pointer_field=0
        // Put a valid-looking PMT prefix in the payload so we'd ASSUME PSI
        // parsing would happen if the packet weren't dropped.
        pkt[5] = 0x02; // table_id (PMT)
        pkt[6] = 0xB0; // section_syntax_indicator=1, reserved=11, sl_hi=0
        pkt[7] = 0x05; // sl_lo=5

        demux.feed(&pkt).expect("TEI feed");

        // Expect TransportErrorPacket on PID 0x100, NO PSI parse.
        let mut saw_tei = false;
        let mut saw_psi = false;
        while let Some(ev) = demux.next_event() {
            match ev {
                DemuxEvent::NonConformant {
                    issue: NonConformantIssue::TransportErrorPacket { pid: 0x100 },
                    ..
                } => {
                    saw_tei = true;
                }
                DemuxEvent::NonConformant {
                    issue: NonConformantIssue::PsiOverlongSection { .. },
                    ..
                }
                | DemuxEvent::ProgramMap(_) => {
                    saw_psi = true;
                }
                _ => {}
            }
        }
        assert!(saw_tei, "expected TransportErrorPacket on PID 0x100");
        assert!(!saw_psi, "TEI packet should be dropped before PSI parsing");
    }

    /// Per ISO/IEC 13818-1 §2.4.3.5, when adaptation_field.discontinuity_indicator=1
    /// the CC is *allowed* to be discontinuous on that packet. ffmpeg
    /// suppresses the CC error in that case (mpegts.c:3075-3078). We must
    /// too — emitting both `DiscontinuityKind::AdaptationFieldFlag` AND
    /// `DiscontinuityKind::ContinuityJump` double-counts the same event in
    /// stats and confuses strict-mode consumers.
    #[test]
    fn discontinuity_indicator_suppresses_continuity_jump_event() {
        use crate::mpegts::mux::{ConfigBuilder, VideoCodec as MuxVideoCodec};
        // Build a real PAT+PMT+video PES through the muxer so the demuxer's
        // PSI tables get populated for PID 0x100 and `cc_by_pid` is primed.
        // This is the same pattern already used by other unit tests in this
        // module (e.g. `demuxer_emits_audio_sample_for_aac_pes`) — we need
        // PSI parsed for `lookup_stream(0x100)` to resolve and for the CC
        // tracker to have a baseline against which to detect a jump.
        let cfg = ConfigBuilder::default()
            .add_program(1, 0x1000)
            .add_video(0x100, MuxVideoCodec::H264)
            .end_program()
            .build()
            .unwrap();
        let mut mux = crate::mpegts::mux::Muxer::new(cfg).unwrap();
        let mut au = vec![0x00, 0x00, 0x00, 0x01, 0x09, 0x10];
        au.extend(std::iter::repeat(0xAB).take(64));
        mux.push_video(&au, 9_000, true).unwrap();
        let mut buf = vec![0u8; 188 * 64];
        let n = mux.pull(&mut buf);

        let mut demux = Demuxer::new();
        demux.feed(&buf[..n]).expect("seed feed");

        // Find the last CC value emitted by the muxer for PID 0x100 so we
        // can construct a guaranteed-jumping CC on our synthetic packet.
        // Walk the seeded bytes packet-by-packet; CC is the low nibble of
        // byte 3.
        let mut last_cc = 0u8;
        for i in (0..n).step_by(188) {
            if buf[i] != 0x47 {
                continue;
            }
            let pid = ((u16::from(buf[i + 1] & 0x1F)) << 8) | u16::from(buf[i + 2]);
            if pid == 0x100 {
                last_cc = buf[i + 3] & 0x0F;
            }
        }
        // Pick a CC that is NOT (last_cc + 1) & 0x0F — bump by 5.
        let bad_cc = (last_cc.wrapping_add(5)) & 0x0F;

        // Drain events queued from the seed feed so we only observe events
        // produced by our synthetic packet.
        while demux.next_event().is_some() {}

        // Build a TS packet on PID 0x100 with adaptation_field present,
        // discontinuity_indicator=1, and a CC that has jumped relative to
        // the last muxer-emitted CC for this PID.
        let mut pkt = [0xFFu8; 188];
        pkt[0] = 0x47;
        // PUSI=0, PID high bits = 1 (PID 0x100).
        pkt[1] = 0x01;
        pkt[2] = 0x00;
        // adaptation_field_control = '11' (both AF + payload).
        pkt[3] = 0x30 | (bad_cc & 0x0F);
        // adaptation_field_length = 1 (just the flags byte).
        pkt[4] = 1;
        // Flags byte: bit 7 = discontinuity_indicator.
        pkt[5] = 0x80;
        // Bytes 6..188 are payload — left as 0xFF (the buffer init).

        demux.feed(&pkt).expect("DI packet feed");

        let mut cc_jumps = 0usize;
        let mut di_events = 0usize;
        while let Some(ev) = demux.next_event() {
            if let DemuxEvent::Discontinuity {
                stream: StreamId { pid: 0x100, .. },
                kind,
            } = ev
            {
                match kind {
                    DiscontinuityKind::ContinuityJump { .. } => cc_jumps += 1,
                    DiscontinuityKind::AdaptationFieldFlag => di_events += 1,
                    _ => {}
                }
            }
        }
        assert_eq!(
            cc_jumps, 0,
            "ContinuityJump must be suppressed when discontinuity_indicator=1"
        );
        assert!(
            di_events >= 1,
            "AdaptationFieldFlag event should still fire (got {di_events})"
        );
    }

    /// Per ISO/IEC 13818-1 §2.4.4.4, PSI sections with `current_next_indicator=0`
    /// describe a future-staged table not yet in effect. ffmpeg drops these
    /// (mpegts.c:1759, 2610-2611, 2832, 2969-2970, 2974). If we instead process
    /// them and bump `pat_version`, the *real* current section that follows is
    /// then deduplicated as "same version, skip", silently dropping streams
    /// that are still active.
    ///
    /// We verify the staged PAT is not processed by feeding only the staged
    /// PAT (cn=0) followed by a PMT on the PID it announces. With the bug,
    /// the staged PAT installs PID 0x100 in `programs` and the PMT then fires
    /// a ProgramMap. With the fix, the PAT is dropped, PID 0x100 is unknown,
    /// and the PMT is silently ignored — no ProgramMap.
    #[test]
    fn pat_with_current_next_indicator_zero_is_dropped() {
        let mut demux = Demuxer::new();

        // Build a PAT with current_next_indicator=0 announcing program 1 → 0x100.
        // Byte 5 = 0xC0 → reserved(2)=11, version(5)=0, current_next(1)=0.
        let mut pat = vec![
            0x00, // table_id = PAT
            0xB0, 0x0D, // syntax=1, section_length=0x00D
            0x00, 0x01, // transport_stream_id=1
            0xC0, // reserved=11, version=0, current_next=0
            0x00, 0x00, // section_number, last_section_number
            0x00, 0x01, // program_number=1
            0xE1, 0x00, // reserved=111, pmt_pid=0x100
        ];
        let pat_crc = crate::mpegts::common::crc32::crc32_mpeg2(&pat);
        pat.extend_from_slice(&pat_crc.to_be_bytes());

        let mut ts_pat = [0xFFu8; 188];
        ts_pat[0] = 0x47;
        ts_pat[1] = 0x40; // PUSI=1, pid_high=0
        ts_pat[2] = 0x00; // pid_low=0 → PID 0x0000
        ts_pat[3] = 0x10; // adaptation=01, CC=0
        ts_pat[4] = 0x00; // pointer_field
        ts_pat[5..5 + pat.len()].copy_from_slice(&pat);
        demux.feed(&ts_pat).expect("staged PAT feed");

        // Build a valid current PMT (cn=1) for program 1 announcing one
        // H.264 stream on PID 0x101. Body is 18 bytes (0x12) past byte 3.
        let mut pmt = vec![
            0x02, // table_id = PMT
            0xB0, 0x12, // syntax=1, section_length=0x012
            0x00, 0x01, // program_number=1
            0xC1, // reserved=11, version=0, current_next=1
            0x00, 0x00, // section_number, last_section_number
            0xE1, 0x01, // reserved=111, pcr_pid=0x101
            0xF0, 0x00, // reserved=1111, program_info_length=0
            0x1B, // stream_type = H.264
            0xE1, 0x01, // reserved=111, elementary_pid=0x101
            0xF0, 0x00, // reserved=1111, es_info_length=0
        ];
        let pmt_crc = crate::mpegts::common::crc32::crc32_mpeg2(&pmt);
        pmt.extend_from_slice(&pmt_crc.to_be_bytes());

        let mut ts_pmt = [0xFFu8; 188];
        ts_pmt[0] = 0x47;
        ts_pmt[1] = 0x41; // PUSI=1, pid_high=1
        ts_pmt[2] = 0x00; // pid_low=0 → PID 0x100
        ts_pmt[3] = 0x10; // adaptation=01, CC=0
        ts_pmt[4] = 0x00; // pointer_field
        ts_pmt[5..5 + pmt.len()].copy_from_slice(&pmt);
        demux.feed(&ts_pmt).expect("PMT feed");

        // With the fix, the staged PAT is dropped → PID 0x100 was never
        // installed as a PMT PID → the PMT packet on 0x100 is ignored → no
        // ProgramMap fires. With the bug, the staged PAT installed the
        // program and a ProgramMap fires from the PMT.
        let mut saw_program_map = false;
        while let Some(ev) = demux.next_event() {
            if matches!(ev, DemuxEvent::ProgramMap(_)) {
                saw_program_map = true;
            }
        }
        assert!(
            !saw_program_map,
            "PAT with current_next=0 must be dropped — its PID 0x100 must not \
             be registered, so a subsequent PMT on 0x100 is ignored and no \
             ProgramMap fires"
        );
    }

    /// Per ISO/IEC 13818-1 §2.4.4.4, the PMT case mirrors the PAT case: a PMT
    /// with `current_next_indicator=0` is the next staged table, not the
    /// current one. Processing it bumps `pmt_version`, which then dedupes the
    /// real current PMT — silently dropping streams.
    #[test]
    fn pmt_with_current_next_indicator_zero_is_dropped() {
        let mut demux = Demuxer::new();

        // Seed a normal PAT mapping program 1 → PMT PID 0x100, so the
        // demuxer routes packets on PID 0x100 to handle_pmt_section.
        let mut pat = vec![
            0x00, // table_id
            0xB0, 0x0D, // syntax=1, section_length=0x00D
            0x00, 0x01, // transport_stream_id=1
            0xC1, // reserved=11, version=0, current_next=1
            0x00, 0x00, // section_number, last_section_number
            0x00, 0x01, // program_number=1
            0xE1, 0x00, // reserved=111, pmt_pid=0x100
        ];
        let pat_crc = crate::mpegts::common::crc32::crc32_mpeg2(&pat);
        pat.extend_from_slice(&pat_crc.to_be_bytes());

        let mut ts_pat = [0xFFu8; 188];
        ts_pat[0] = 0x47;
        ts_pat[1] = 0x40;
        ts_pat[2] = 0x00;
        ts_pat[3] = 0x10;
        ts_pat[4] = 0x00;
        ts_pat[5..5 + pat.len()].copy_from_slice(&pat);
        demux.feed(&ts_pat).expect("PAT feed");

        // Drain PAT-induced events (no ProgramMap yet — needs PMT).
        while demux.next_event().is_some() {}

        // Build a PMT with current_next_indicator=0 announcing one H.264
        // video stream on PID 0x101. Byte 5 = 0xC0.
        //   table_id=0x02, syntax=1, section_length, program_number=1,
        //   byte5=0xC0 (current_next=0), section_number=0, last=0,
        //   reserved+pcr_pid=0xE101, reserved+program_info_length=0xF000,
        //   stream_type=0x1B (H.264), reserved+elementary_pid=0xE101,
        //   reserved+es_info_length=0xF000, CRC32.
        // Body length from byte 3: 5 (header tail) + 4 (program_info hdr)
        //   + 5 (stream entry) + 4 (CRC) = 18 = 0x12.
        let mut pmt = vec![
            0x02, // table_id = PMT
            0xB0, 0x12, // syntax=1, section_length=0x012
            0x00, 0x01, // program_number=1
            0xC0, // reserved=11, version=0, current_next=0
            0x00, 0x00, // section_number, last_section_number
            0xE1, 0x01, // reserved=111, pcr_pid=0x101
            0xF0, 0x00, // reserved=1111, program_info_length=0
            0x1B, // stream_type = H.264
            0xE1, 0x01, // reserved=111, elementary_pid=0x101
            0xF0, 0x00, // reserved=1111, es_info_length=0
        ];
        let pmt_crc = crate::mpegts::common::crc32::crc32_mpeg2(&pmt);
        pmt.extend_from_slice(&pmt_crc.to_be_bytes());

        let mut ts_pmt = [0xFFu8; 188];
        ts_pmt[0] = 0x47;
        ts_pmt[1] = 0x41; // PUSI=1, pid_high=1
        ts_pmt[2] = 0x00; // pid_low=0 → PID = 0x100
        ts_pmt[3] = 0x10;
        ts_pmt[4] = 0x00; // pointer_field
        ts_pmt[5..5 + pmt.len()].copy_from_slice(&pmt);
        demux.feed(&ts_pmt).expect("staged PMT feed");

        // The staged PMT must NOT produce a ProgramMap event.
        let mut saw_program_map = false;
        while let Some(ev) = demux.next_event() {
            if matches!(ev, DemuxEvent::ProgramMap(_)) {
                saw_program_map = true;
            }
        }
        assert!(
            !saw_program_map,
            "PMT with current_next=0 must not produce ProgramMap events"
        );
    }

    /// Audit finding (Demux-C): the muxer wraps DVB-sub PES payloads in the
    /// EN 300 743 §6.2 envelope (`0x20 + 0x00 + segments + 0xFF`), so the
    /// demuxer must strip that envelope before surfacing to callers. Without
    /// the strip, libavcodec's `dvbsubdec` rejects the buffer at
    /// `buf_size <= 6 || *buf != 0x0f`.
    #[test]
    fn dvb_sub_demux_strips_pes_data_field_envelope() {
        use crate::mpegts::demux::event::SamplePayload;
        use crate::mpegts::mux::{
            Config, Muxer, SubtitleCodec as MuxSubtitleCodec, VideoCodec as MuxVideoCodec,
        };

        // Configure: one program with one H.264 video stream (PCR carrier)
        // plus one DVB-sub stream. Subtitles can't be the PCR PID, so the
        // video stream is required for a valid program.
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x101, MuxVideoCodec::H264)
            .add_subtitle(
                0x200,
                MuxSubtitleCodec::DvbSubtitling {
                    language: *b"eng",
                    subtitling_type: 0x10,
                    composition_page_id: 0x0001,
                    ancillary_page_id: 0x0002,
                },
            )
            .end_program()
            .build()
            .unwrap();
        let mut mux = Muxer::new(cfg).unwrap();

        // Push a video AU first so PCR fires and PSI emits.
        let mut au = vec![0x00, 0x00, 0x00, 0x01, 0x09, 0x10];
        au.extend(std::iter::repeat_n(0xAB, 64));
        mux.push_video(&au, 9_000, true).unwrap();

        // Push raw DVB-sub segment bytes; muxer auto-prepends §6.2 envelope.
        let segment_bytes = [0x0Fu8, 0x10, 0x00, 0x01, 0x00, 0x02, 0x00, 0x10];
        mux.push_subtitle(9_000, &segment_bytes).unwrap();

        // Drain all queued packets.
        let mut all = Vec::new();
        let mut buf = vec![0u8; 188 * 256];
        loop {
            let n = mux.pull(&mut buf);
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n]);
        }

        let mut demux = Demuxer::new();
        demux.feed(&all).expect("feed");
        demux.flush();

        let mut subtitle_payload: Option<Vec<u8>> = None;
        while let Some(ev) = demux.next_event() {
            if let DemuxEvent::Sample {
                payload:
                    SamplePayload::Subtitle {
                        codec: SubtitleCodec::DvbSubtitling,
                        payload,
                    },
                ..
            } = ev
            {
                subtitle_payload = Some(payload);
                break;
            }
        }
        let payload = subtitle_payload.expect("DVB-sub Sample event not found");

        // Surfaced payload is exactly the segment bytes — no leading
        // 0x20 + 0x00 envelope, no trailing 0xFF marker.
        assert_eq!(
            payload,
            segment_bytes.to_vec(),
            "EN 300 743 §6.2 envelope should be stripped"
        );
    }
}
