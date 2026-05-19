//! Stats accounting + nonconformant event queueing.
//!
//! Hosts 4 helper methods on `Demuxer`:
//!
//! - `queue_nonconformant(stream, issue)` — pushes a `NonConformant`
//!   event onto the queue and captures the first strict-rejected issue
//!   per `feed` call (the `fatal` field on `Demuxer`).
//! - `bump_video_counters(pid, nals_or_obus_delta, ra_delta)` — lazily
//!   creates a video `StreamCodecCounters` entry for a PID on first
//!   event, accumulates delta counters.
//! - `bump_klv_counters(pid, records_delta)` — same shape, KLV-keyed.
//! - `bump_audio_counters(pid, frames_delta)` — same shape, audio-keyed.
//!
//! All `pub(super)`. The 3 public stats accessors (`stats`, `reset_stats`,
//! `stream_codec_stats`) stay in `demuxer.rs` (the coordinator) since
//! they're part of the `Demuxer` public surface.
//!
//! Per Wave 6.B Decision DB3, no new struct wrapper — the audit's
//! `DemuxStatsRecorder` proposal is deferred to a future ergonomics pass.

use crate::mpegts::demux::event::{DemuxEvent, NonConformantIssue, StreamId};

impl super::demuxer::Demuxer {
    pub(super) fn queue_nonconformant(&mut self, stream: StreamId, issue: NonConformantIssue) {
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

    pub(super) fn bump_video_counters(&mut self, pid: u16, nals_or_obus_delta: u64, ra_delta: u64) {
        let c = self
            .stream_codec_counters
            .entry(pid)
            .or_insert_with(crate::mpegts::stats::StreamCodecCounters::new_video);
        c.nals_or_obus = c.nals_or_obus.saturating_add(nals_or_obus_delta);
        c.random_access_aus = c.random_access_aus.saturating_add(ra_delta);
    }

    pub(super) fn bump_klv_counters(&mut self, pid: u16, records_delta: u64) {
        let c = self
            .stream_codec_counters
            .entry(pid)
            .or_insert_with(crate::mpegts::stats::StreamCodecCounters::new_klv);
        c.records = c.records.saturating_add(records_delta);
    }

    pub(super) fn bump_audio_counters(&mut self, pid: u16, frames_delta: u64) {
        let c = self
            .stream_codec_counters
            .entry(pid)
            .or_insert_with(crate::mpegts::stats::StreamCodecCounters::new_audio);
        c.frames = c.frames.saturating_add(frames_delta);
    }
}
