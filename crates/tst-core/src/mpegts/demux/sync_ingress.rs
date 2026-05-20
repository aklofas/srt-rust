//! Demuxer sync-ingress state machine: byte-aligned 188-byte packet
//! detection + sync-recovery buffer compaction + per-packet PCR / CC
//! anomaly checks.
//!
//! Hosts three module-level constants tuned in Phase 4 (`MAX_SYNC_BUF_BYTES`
//! caps adversarial-input memory growth; `SYNC_SEARCH_WINDOW` bounds
//! per-feed sync-hunt work; `PCR_ANOMALY_THRESHOLD` discriminates real PCR
//! jumps from steady-state drift). Per Wave 6.B Decision DB7, constants
//! follow their consumers.
//!
//! Helper methods are `pub(super)` so the `Demuxer` coordinator (`demuxer.rs`)
//! can call them; the module itself is private (`mod sync_ingress` in
//! `mpegts/demux/mod.rs`).

use crate::mpegts::common::{StreamTypeCode, pcr_diff_27mhz};
use crate::mpegts::demux::event::{DemuxEvent, DiscontinuityKind, NonConformantIssue};

/// Maximum bytes the demuxer scans during sync recovery before declaring
/// the stream unrecoverable.
pub(super) const SYNC_SEARCH_WINDOW: usize = crate::mpegts::common::TS_PACKET_SIZE * 32;

/// Hard ceiling on `Demuxer::sync_buf`. `feed` always runs
/// `extend_from_slice` before the inner sync-search-window check fires,
/// so an oversized single-call feed (multi-GB of garbage) would otherwise
/// allocate the whole input before the loop got to bail. The 4 MiB cap
/// matches ffmpeg's `MpegTSSectionFilter` ceiling and is comfortably
/// larger than `SYNC_SEARCH_WINDOW` (~6 KiB), so well-formed streams are
/// unaffected.
pub(super) const MAX_SYNC_BUF_BYTES: usize = 4 << 20;

/// PCR jump threshold beyond which we emit `PcrAnomaly`. 1 second @ 27 MHz.
pub(super) const PCR_ANOMALY_THRESHOLD: i64 = 27_000_000;

/// Sync re-acquisition N-of-M: when a candidate sync byte is found, only
/// declare sync acquired if at least `SYNC_REACQ_N` of the next
/// `SYNC_REACQ_M` strides at +188, +376, ... also carry 0x47. This
/// rejects false sync on a stray 0x47 inside PES payload, descriptors,
/// or other in-band data.
///
/// Values match ffmpeg `libavformat/mpegts.c::mpegts_resync` (5 of 7),
/// which is the long-standing tested value for the same problem. ffmpeg's
/// `MAX_RESYNC_SIZE` is exposed via the `resync_size` AVOption; we match
/// the upstream default. Per H.222.0 §2.4.3.2 a real 188-byte aligned
/// stream MUST satisfy this; a 0x47 byte inside a PES payload almost
/// never aligns 5-of-7 times at 188-byte stride.
pub(super) const SYNC_REACQ_N: usize = 5;
pub(super) const SYNC_REACQ_M: usize = 7;

/// Possible outcomes of the N-of-M re-acquisition check at a candidate
/// `live[0] == 0x47` byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NofMResult {
    /// Candidate matched ≥ `SYNC_REACQ_N` of `SYNC_REACQ_M` strides at
    /// 188-byte boundaries. Accept as the next packet's sync byte.
    Accept,
    /// Candidate matched < `SYNC_REACQ_N` of the full `SYNC_REACQ_M`
    /// window. Reject; advance past this byte and keep searching.
    Reject,
    /// Buffer didn't contain enough bytes to complete the full window,
    /// AND the partial evidence isn't conclusive (matches < N but the
    /// remaining unchecked strides could still bring the total to N or
    /// above). Caller should pause (return `Ok(())`) and wait for more
    /// bytes before deciding.
    NeedMoreBytes,
}

/// Inspect `live` (starting at a candidate 0x47) and decide whether
/// enough 188-byte strides also carry 0x47 to declare sync acquired.
/// See [`NofMResult`] for the three outcomes.
///
/// The decision must be sound under three buffer-fill cases:
///
/// 1. **Full window** (`live.len() >= SYNC_REACQ_M * 188`): count all
///    strides, accept if `matches >= SYNC_REACQ_N`.
/// 2. **Partial window with enough evidence to accept** (matches at the
///    candidate + checked strides already >= N): accept immediately.
///    Avoids waiting on bytes we don't need.
/// 3. **Partial window with enough evidence to reject** (matches +
///    remaining-unchecked < N): reject. Remaining strides can't lift
///    the count above N regardless.
/// 4. **Partial window, undecided**: return `NeedMoreBytes`. Caller
///    keeps the candidate's bytes buffered and re-checks on the next
///    `feed`.
pub(super) fn sync_n_of_m_check(live: &[u8]) -> NofMResult {
    debug_assert_eq!(live.first(), Some(&crate::mpegts::common::TS_SYNC_BYTE));
    let stride = crate::mpegts::common::TS_PACKET_SIZE;
    // The candidate itself counts as the first match.
    let mut matches: usize = 1;
    let mut checked: usize = 1;
    for k in 1..SYNC_REACQ_M {
        let off = k * stride;
        if off >= live.len() {
            break;
        }
        checked += 1;
        if live[off] == crate::mpegts::common::TS_SYNC_BYTE {
            matches += 1;
        }
    }
    if matches >= SYNC_REACQ_N {
        return NofMResult::Accept;
    }
    // Could the remaining unchecked strides push us over N?
    let remaining = SYNC_REACQ_M - checked;
    if matches + remaining < SYNC_REACQ_N {
        return NofMResult::Reject;
    }
    // Indeterminate with current buffer — wait for more bytes.
    NofMResult::NeedMoreBytes
}

impl super::demuxer::Demuxer {
    /// Reclaim the consumed prefix of `sync_buf` once it grows past half
    /// the live size (or 1 MiB, whichever is larger). The half-and-compact
    /// rule keeps total memmove work amortized-linear in bytes fed; the
    /// 1 MiB floor avoids churn on tiny live regions.
    pub(super) fn compact_sync_buf(&mut self) {
        let consumed = self.sync_consumed;
        let live = self.sync_buf.len() - consumed;
        if consumed >= live.max(1 << 20) {
            self.sync_buf.drain(..consumed);
            self.sync_consumed = 0;
        }
    }

    pub(super) fn check_pcr(&mut self, pkt: &crate::mpegts::demux::ts::TsPacket<'_>) {
        // Per ITU-T H.222.0 §2.4.3.5, each program carries its own time base
        // via its declared PCR PID. PCR comparisons MUST stay within a
        // single PID's timeline; comparing across PIDs in a multi-program TS
        // produces spurious PcrAnomaly events (validate-1 B1 / Codex
        // TS-TIME-01). Key the last-seen map by `pkt.pid` (the on-wire PCR
        // PID) — that's the canonical identifier of a time base.
        //
        // Nested if-let (not let-chain) for MSRV 1.85 — let-chains require 1.88.
        if let Some(now) = pkt.pcr_27mhz {
            if let Some(&last) = self.last_pcr_by_pid.get(&pkt.pid) {
                let diff = pcr_diff_27mhz(now, last);
                if diff.abs() > PCR_ANOMALY_THRESHOLD {
                    let issue = NonConformantIssue::PcrAnomaly { delta: diff };
                    if let Some(stream) = self.lookup_stream(pkt.pid) {
                        self.queue_nonconformant(stream, issue);
                    }
                }
            }
            self.last_pcr_by_pid.insert(pkt.pid, now);
        }
    }

    /// Returns `true` if a CC jump was observed AND not suppressed by
    /// `discontinuity_indicator`. The caller (`process_packet`) uses this
    /// signal to gate strict-mode PSI reassembly drops in `handle_psi`.
    ///
    /// Side effect: clears `self.last_psi_cc_jump` at entry, sets it to
    /// `Some((expected, observed))` when a real jump fires. `handle_psi`
    /// drains it via `.take()` when emitting `PsiCcDiscontinuity`.
    pub(super) fn check_continuity(
        &mut self,
        pkt: &crate::mpegts::demux::ts::TsPacket<'_>,
    ) -> bool {
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
                            stream_type: StreamTypeCode::from_byte(
                                super::pmt_classify::stream_type_from_kind(&stream.kind),
                            ),
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
                        stream_type: StreamTypeCode::from_byte(
                            super::pmt_classify::stream_type_from_kind(&stream.kind),
                        ),
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
}
