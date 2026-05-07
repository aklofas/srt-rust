//! Sample-and-hold (last-before-PTS) pairing state machine.
//! Implementation lands in Task 4.

use super::PairerOutput;
use tst_core::mpegts::demux::DemuxEvent;

pub(super) struct LastBeforeState {
    _video_pid: u16,
    _klv_pid: u16,
    _freshness_ticks: Option<i64>,
}

impl LastBeforeState {
    pub(super) fn new(
        video_pid: u16,
        klv_pid: u16,
        freshness_ticks: Option<i64>,
    ) -> Self {
        Self {
            _video_pid: video_pid,
            _klv_pid: klv_pid,
            _freshness_ticks: freshness_ticks,
        }
    }

    pub(super) fn feed(&mut self, _event: DemuxEvent) -> Vec<PairerOutput> {
        todo!("Task 4: implement last-before feed")
    }

    pub(super) fn flush(&mut self) -> Vec<PairerOutput> {
        todo!("Task 6: implement last-before flush")
    }
}
