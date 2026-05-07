//! Nearest-PTS pairing state machine. Implementation lands in Tasks 2 + 3.

use super::{MatchMode, PairerOutput};
use tst_core::mpegts::demux::DemuxEvent;

pub(super) struct NearestState {
    // Fields filled in Task 2.
    _video_pid: u16,
    _klv_pid: u16,
    _tolerance_ticks: i64,
    _max_klv_history: usize,
    _mode: MatchMode,
}

impl NearestState {
    pub(super) fn new(
        video_pid: u16,
        klv_pid: u16,
        tolerance_ticks: i64,
        max_klv_history: usize,
        mode: MatchMode,
    ) -> Self {
        Self {
            _video_pid: video_pid,
            _klv_pid: klv_pid,
            _tolerance_ticks: tolerance_ticks,
            _max_klv_history: max_klv_history,
            _mode: mode,
        }
    }

    pub(super) fn feed(&mut self, _event: DemuxEvent) -> Vec<PairerOutput> {
        todo!("Task 2: implement nearest-PTS Realtime feed")
    }

    pub(super) fn flush(&mut self) -> Vec<PairerOutput> {
        todo!("Task 6: implement nearest flush")
    }
}
