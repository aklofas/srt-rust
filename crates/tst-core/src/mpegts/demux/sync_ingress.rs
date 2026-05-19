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
                                super::demuxer::stream_type_from_kind(&stream.kind),
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
                            super::demuxer::stream_type_from_kind(&stream.kind),
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
