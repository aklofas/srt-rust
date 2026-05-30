//! Per-PID Metadata AU cell reassembler.
//!
//! State machine for accumulating fragmented sync-metadata AU cells per
//! H.222.0 V9 §2.12.4.2 Table 2-157 (`cell_fragment_indication` =
//! First / Middle / Last) into complete Access Units. Single-cell
//! (`Complete`) cells pass through unchanged. Failure modes surface as
//! [`MultiCellAuReason`] variants — Orphan / SequenceGap /
//! ConcurrentFirst / Overflow.
//!
//! Owned by the demuxer (one instance per [`super::Demuxer`]). Keyed by
//! PID, because PIDs are unique within a TS even when shared across
//! PMTs. Cleared wholesale on `Demuxer::reset_sync()` and on PMT
//! version change.

use crate::mpegts::au_cell::{AuCellHeader, CellFragmentIndication};
use alloc::vec::Vec;
use hashbrown::HashMap;

use super::event::MultiCellAuReason;

/// In-flight reassembly state for one PID.
#[derive(Debug)]
struct PidReassemblyState {
    /// Header from the `First` cell. Surfaced on emit (CFI forced to
    /// `Complete`, see [`AuCellReassembler::process_cell`]).
    first_header: AuCellHeader,
    /// Accumulated inner-byte payload (concatenated AU cell
    /// `AU_cell_data_byte` regions).
    buf: Vec<u8>,
    /// Number of cells accumulated so far (≥ 1 once a `First` is seen).
    cell_count: u32,
}

impl PidReassemblyState {
    fn next_expected_seq(&self) -> u8 {
        self.first_header
            .sequence_number
            .wrapping_add(self.cell_count as u8)
    }
}

/// Outcome of feeding one AU cell into the reassembler.
#[derive(Debug)]
pub(crate) enum ReassembleOutcome<'a> {
    /// A complete AU is ready to emit. `header` is the first cell's
    /// header (with `cell_fragment_indication` forced to `Complete`),
    /// `payload` is the concatenated inner bytes (zero-copy borrow into
    /// the reassembler's internal buffer for the multi-cell case, or a
    /// borrow into the caller's slice for the single-cell case),
    /// `cell_count` is how many cells contributed (1 for Complete,
    /// ≥ 2 for reassembled).
    Emit {
        header: AuCellHeader,
        payload: &'a [u8],
        cell_count: u32,
    },
    /// Fragment buffered; no emit yet. Wait for more cells.
    Buffered,
    /// Reassembly failed. Buffer (if any) has been dropped. The caller
    /// SHOULD emit `NonConformantIssue::MultiCellAu { reason, dropped_bytes, .. }`.
    Failure {
        reason: MultiCellAuReason,
        dropped_bytes: usize,
    },
}

#[derive(Debug)]
pub(crate) struct AuCellReassembler {
    per_pid: HashMap<u16, PidReassemblyState>,
    cap_per_pid: usize,
}

impl AuCellReassembler {
    pub(crate) fn new(cap_per_pid: usize) -> Self {
        Self {
            per_pid: HashMap::new(),
            cap_per_pid,
        }
    }

    /// Feed one AU cell into the reassembler. Returns the outcome.
    ///
    /// For `ConcurrentFirst` failures the caller MUST emit the failure
    /// event AND then re-call `process_cell` with the same cell — the
    /// internal state is now empty so the second call follows the
    /// empty-state row of the state table and either buffers (if the
    /// new cell is `First`) or emits (if `Complete`).
    pub(crate) fn process_cell<'a>(
        &'a mut self,
        pid: u16,
        header: AuCellHeader,
        payload: &'a [u8],
    ) -> ReassembleOutcome<'a> {
        match (
            self.per_pid.contains_key(&pid),
            header.cell_fragment_indication,
        ) {
            // Empty + Complete → emit directly.
            (false, CellFragmentIndication::Complete) => ReassembleOutcome::Emit {
                header,
                payload,
                cell_count: 1,
            },

            // Empty + First → open buffer.
            (false, CellFragmentIndication::First) => {
                if payload.len() > self.cap_per_pid {
                    return ReassembleOutcome::Failure {
                        reason: MultiCellAuReason::Overflow,
                        dropped_bytes: payload.len(),
                    };
                }
                self.per_pid.insert(
                    pid,
                    PidReassemblyState {
                        first_header: header,
                        buf: payload.to_vec(),
                        cell_count: 1,
                    },
                );
                ReassembleOutcome::Buffered
            }

            // Empty + Middle/Last → orphan.
            (false, CellFragmentIndication::Middle | CellFragmentIndication::Last) => {
                ReassembleOutcome::Failure {
                    reason: MultiCellAuReason::Orphan,
                    dropped_bytes: payload.len(),
                }
            }

            // Buffering + Complete → drop buffer (concurrent), then caller re-enters.
            (true, CellFragmentIndication::Complete) => {
                let dropped = self.per_pid.remove(&pid).unwrap();
                let _ = header;
                ReassembleOutcome::Failure {
                    reason: MultiCellAuReason::ConcurrentFirst,
                    dropped_bytes: dropped.buf.len(),
                }
            }

            // Buffering + First → drop buffer (concurrent). Caller re-calls.
            (true, CellFragmentIndication::First) => {
                let dropped = self.per_pid.remove(&pid).unwrap();
                ReassembleOutcome::Failure {
                    reason: MultiCellAuReason::ConcurrentFirst,
                    dropped_bytes: dropped.buf.len(),
                }
            }

            // Buffering + Middle → seq check + append.
            (true, CellFragmentIndication::Middle) => {
                let state = self.per_pid.get_mut(&pid).unwrap();
                if header.sequence_number != state.next_expected_seq() {
                    let dropped = self.per_pid.remove(&pid).unwrap();
                    return ReassembleOutcome::Failure {
                        reason: MultiCellAuReason::SequenceGap,
                        dropped_bytes: dropped.buf.len() + payload.len(),
                    };
                }
                if state.buf.len() + payload.len() > self.cap_per_pid {
                    let dropped = self.per_pid.remove(&pid).unwrap();
                    return ReassembleOutcome::Failure {
                        reason: MultiCellAuReason::Overflow,
                        dropped_bytes: dropped.buf.len() + payload.len(),
                    };
                }
                state.buf.extend_from_slice(payload);
                state.cell_count += 1;
                ReassembleOutcome::Buffered
            }

            // Buffering + Last → seq check + append + emit.
            (true, CellFragmentIndication::Last) => {
                let state = self.per_pid.get_mut(&pid).unwrap();
                if header.sequence_number != state.next_expected_seq() {
                    let dropped = self.per_pid.remove(&pid).unwrap();
                    return ReassembleOutcome::Failure {
                        reason: MultiCellAuReason::SequenceGap,
                        dropped_bytes: dropped.buf.len() + payload.len(),
                    };
                }
                if state.buf.len() + payload.len() > self.cap_per_pid {
                    let dropped = self.per_pid.remove(&pid).unwrap();
                    return ReassembleOutcome::Failure {
                        reason: MultiCellAuReason::Overflow,
                        dropped_bytes: dropped.buf.len() + payload.len(),
                    };
                }
                state.buf.extend_from_slice(payload);
                state.cell_count += 1;
                let state_ref = self.per_pid.get(&pid).unwrap();
                let mut first_header = state_ref.first_header;
                first_header.cell_fragment_indication = CellFragmentIndication::Complete;
                ReassembleOutcome::Emit {
                    header: first_header,
                    payload: &state_ref.buf,
                    cell_count: state_ref.cell_count,
                }
            }
        }
    }

    /// Clear the buffer for one PID (operational reset, not a wire-format
    /// violation; no NonConformant emit). Reserved for future hooks; not
    /// called from the current pes_emit / reset_sync paths (those use
    /// [`Self::reset_all`]).
    #[allow(dead_code)]
    pub(crate) fn reset_pid(&mut self, pid: u16) {
        self.per_pid.remove(&pid);
    }

    /// Clear all buffers. Called from `Demuxer::reset_sync()` and on PMT
    /// version change.
    pub(crate) fn reset_all(&mut self) {
        self.per_pid.clear();
    }

    /// On `Emit` of a multi-cell AU the caller drains the buffer via this
    /// method (since `Emit`'s `&[u8]` borrow prevents `&mut self`
    /// operations during the emit). Call AFTER copying / consuming the
    /// `Emit::payload` slice.
    pub(crate) fn clear_after_emit(&mut self, pid: u16) {
        self.per_pid.remove(&pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(cfi: CellFragmentIndication, seq: u8) -> AuCellHeader {
        AuCellHeader {
            metadata_service_id: 0,
            sequence_number: seq,
            cell_fragment_indication: cfi,
            decoder_config_flag: false,
            random_access_indicator: true,
        }
    }

    #[test]
    fn complete_cell_emits_directly() {
        let mut r = AuCellReassembler::new(1024);
        let out = r.process_cell(
            0x100,
            hdr(CellFragmentIndication::Complete, 42),
            &[0xAA, 0xBB],
        );
        match out {
            ReassembleOutcome::Emit {
                header,
                payload,
                cell_count,
            } => {
                assert_eq!(header.sequence_number, 42);
                assert_eq!(payload, &[0xAA, 0xBB]);
                assert_eq!(cell_count, 1);
            }
            _ => panic!("expected Emit, got {:?}", out),
        }
    }

    #[test]
    fn first_middle_last_reassembles() {
        let mut r = AuCellReassembler::new(1024);
        assert!(matches!(
            r.process_cell(0x100, hdr(CellFragmentIndication::First, 10), &[0x11; 3]),
            ReassembleOutcome::Buffered
        ));
        assert!(matches!(
            r.process_cell(0x100, hdr(CellFragmentIndication::Middle, 11), &[0x22; 3]),
            ReassembleOutcome::Buffered
        ));
        let out = r.process_cell(0x100, hdr(CellFragmentIndication::Last, 12), &[0x33; 3]);
        match out {
            ReassembleOutcome::Emit {
                header,
                payload,
                cell_count,
            } => {
                assert_eq!(
                    header.cell_fragment_indication,
                    CellFragmentIndication::Complete
                );
                assert_eq!(header.sequence_number, 10);
                assert_eq!(payload.len(), 9);
                assert_eq!(&payload[..3], &[0x11; 3]);
                assert_eq!(&payload[3..6], &[0x22; 3]);
                assert_eq!(&payload[6..], &[0x33; 3]);
                assert_eq!(cell_count, 3);
            }
            _ => panic!("expected Emit, got {:?}", out),
        }
        r.clear_after_emit(0x100);
        assert!(!r.per_pid.contains_key(&0x100));
    }

    #[test]
    fn orphan_middle_fails() {
        let mut r = AuCellReassembler::new(1024);
        let out = r.process_cell(0x100, hdr(CellFragmentIndication::Middle, 5), &[0xAA; 10]);
        assert!(matches!(
            out,
            ReassembleOutcome::Failure {
                reason: MultiCellAuReason::Orphan,
                dropped_bytes: 10
            }
        ));
    }

    #[test]
    fn orphan_last_fails() {
        let mut r = AuCellReassembler::new(1024);
        let out = r.process_cell(0x100, hdr(CellFragmentIndication::Last, 5), &[0xAA; 7]);
        assert!(matches!(
            out,
            ReassembleOutcome::Failure {
                reason: MultiCellAuReason::Orphan,
                dropped_bytes: 7
            }
        ));
    }

    #[test]
    fn sequence_gap_in_middle_fails_and_drops_buffer() {
        let mut r = AuCellReassembler::new(1024);
        r.process_cell(0x100, hdr(CellFragmentIndication::First, 10), &[0x11; 3]);
        let out = r.process_cell(0x100, hdr(CellFragmentIndication::Middle, 12), &[0x22; 3]);
        assert!(matches!(
            out,
            ReassembleOutcome::Failure {
                reason: MultiCellAuReason::SequenceGap,
                dropped_bytes: 6,
            }
        ));
        assert!(!r.per_pid.contains_key(&0x100));
    }

    #[test]
    fn concurrent_first_drops_old_buffer() {
        let mut r = AuCellReassembler::new(1024);
        r.process_cell(0x100, hdr(CellFragmentIndication::First, 10), &[0x11; 5]);
        let out = r.process_cell(0x100, hdr(CellFragmentIndication::First, 20), &[0x22; 5]);
        assert!(matches!(
            out,
            ReassembleOutcome::Failure {
                reason: MultiCellAuReason::ConcurrentFirst,
                dropped_bytes: 5,
            }
        ));
        assert!(matches!(
            r.process_cell(0x100, hdr(CellFragmentIndication::First, 20), &[0x22; 5]),
            ReassembleOutcome::Buffered
        ));
    }

    #[test]
    fn complete_while_buffering_drops_old_then_passes_through() {
        // State-table row 5: (Buffering, Complete) → Failure(ConcurrentFirst),
        // caller re-enters with the same Complete cell against now-empty state
        // (which then matches row 1 → Emit).
        let mut r = AuCellReassembler::new(1024);
        r.process_cell(0x100, hdr(CellFragmentIndication::First, 10), &[0x11; 5]);
        let out = r.process_cell(0x100, hdr(CellFragmentIndication::Complete, 20), &[0x22; 5]);
        assert!(matches!(
            out,
            ReassembleOutcome::Failure {
                reason: MultiCellAuReason::ConcurrentFirst,
                dropped_bytes: 5,
            }
        ));
        // Re-entry: same Complete on now-empty state → Emit (cell_count=1).
        let reentry = r.process_cell(0x100, hdr(CellFragmentIndication::Complete, 20), &[0x22; 5]);
        match reentry {
            ReassembleOutcome::Emit {
                header,
                payload,
                cell_count,
            } => {
                assert_eq!(header.sequence_number, 20);
                assert_eq!(payload, &[0x22; 5]);
                assert_eq!(cell_count, 1);
            }
            _ => panic!("expected Emit on re-entry, got {:?}", reentry),
        }
    }

    #[test]
    fn overflow_drops_buffer() {
        let mut r = AuCellReassembler::new(10);
        assert!(matches!(
            r.process_cell(0x100, hdr(CellFragmentIndication::First, 0), &[0xAA; 5]),
            ReassembleOutcome::Buffered
        ));
        let out = r.process_cell(0x100, hdr(CellFragmentIndication::Middle, 1), &[0xBB; 10]);
        assert!(matches!(
            out,
            ReassembleOutcome::Failure {
                reason: MultiCellAuReason::Overflow,
                dropped_bytes: 15,
            }
        ));
        assert!(!r.per_pid.contains_key(&0x100));
    }

    #[test]
    fn sequence_wrap_mod_256_is_not_a_gap() {
        let mut r = AuCellReassembler::new(1024);
        r.process_cell(0x100, hdr(CellFragmentIndication::First, 254), &[0x11; 3]);
        r.process_cell(0x100, hdr(CellFragmentIndication::Middle, 255), &[0x22; 3]);
        let out = r.process_cell(0x100, hdr(CellFragmentIndication::Last, 0), &[0x33; 3]);
        assert!(matches!(out, ReassembleOutcome::Emit { cell_count: 3, .. }));
    }

    #[test]
    fn multi_pid_isolation() {
        let mut r = AuCellReassembler::new(1024);
        r.process_cell(0x100, hdr(CellFragmentIndication::First, 0), &[0xAA; 3]);
        r.process_cell(0x200, hdr(CellFragmentIndication::First, 100), &[0xBB; 3]);
        let out_a = r.process_cell(0x100, hdr(CellFragmentIndication::Last, 1), &[0xCC; 3]);
        assert!(matches!(out_a, ReassembleOutcome::Emit { .. }));
        r.clear_after_emit(0x100);
        let out_b = r.process_cell(0x200, hdr(CellFragmentIndication::Last, 101), &[0xDD; 3]);
        assert!(matches!(out_b, ReassembleOutcome::Emit { .. }));
        r.clear_after_emit(0x200);
        assert!(r.per_pid.is_empty());
    }

    #[test]
    fn reset_all_clears_buffers_silently() {
        let mut r = AuCellReassembler::new(1024);
        r.process_cell(0x100, hdr(CellFragmentIndication::First, 0), &[0xAA; 3]);
        r.process_cell(0x200, hdr(CellFragmentIndication::First, 0), &[0xBB; 3]);
        r.reset_all();
        assert!(r.per_pid.is_empty());
    }
}
