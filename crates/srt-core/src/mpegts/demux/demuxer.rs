// crates/srt-core/src/mpegts/demux/demuxer.rs
//! Top-level `Demuxer` state machine.
//!
//! Filled in by Tasks 8–10. This file currently exposes only the public
//! type signatures so callers can compile against the API; the bodies
//! return `unimplemented!()` until Task 8.

use crate::error::DemuxError;
use crate::mpegts::demux::event::DemuxEvent;
use crate::mpegts::demux::strict::StrictMode;
use std::collections::HashMap;

#[allow(dead_code)] // wired up by Task 8.
const DEFAULT_PES_CAP_PER_PID: usize = 4 * 1024 * 1024;
#[allow(dead_code)] // wired up by Task 8.
const DEFAULT_PES_CAP_TOTAL: usize = 64 * 1024 * 1024;

/// Caller-supplied overrides for the demuxer.
#[derive(Debug, Clone, Default)]
pub struct DemuxerOptions {
    pub strict: StrictMode,
    pub pes_cap_per_pid: Option<usize>,
    pub pes_cap_total: Option<usize>,
    pub klv_link_overrides: Vec<(u16, u16)>,
    pub stream_kind_overrides: HashMap<u16, crate::mpegts::demux::event::StreamKind>,
}

#[derive(Debug)]
pub struct Demuxer {
    // Filled in by Task 8.
    _options: DemuxerOptions,
}

impl Demuxer {
    pub fn new() -> Self {
        Self {
            _options: DemuxerOptions::default(),
        }
    }

    pub fn with_options(options: DemuxerOptions) -> Self {
        Self { _options: options }
    }

    /// Feed bytes into the demuxer. Bytes need not be 188-aligned; the
    /// demuxer handles TS sync recovery internally.
    pub fn feed(&mut self, _bytes: &[u8]) -> Result<(), DemuxError> {
        unimplemented!("filled in by Task 8")
    }

    /// Pull the next available event. Returns `None` if no event is
    /// currently queued — feed more bytes and try again.
    pub fn next_event(&mut self) -> Option<DemuxEvent> {
        unimplemented!("filled in by Task 8")
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

    pub fn treat_as(mut self, pid: u16, kind: crate::mpegts::demux::event::StreamKind) -> Self {
        self.options.stream_kind_overrides.insert(pid, kind);
        self
    }

    pub fn build(self) -> Demuxer {
        Demuxer::with_options(self.options)
    }
}

#[allow(dead_code)] // wired up by Task 8.
pub(crate) const fn default_pes_cap_per_pid() -> usize {
    DEFAULT_PES_CAP_PER_PID
}

#[allow(dead_code)] // wired up by Task 8.
pub(crate) const fn default_pes_cap_total() -> usize {
    DEFAULT_PES_CAP_TOTAL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_carries_defaults() {
        let d = DemuxerBuilder::new().build();
        assert_eq!(d._options.strict, StrictMode::Off);
        assert_eq!(d._options.pes_cap_per_pid, None);
    }

    #[test]
    fn builder_overrides_apply() {
        let d = DemuxerBuilder::new()
            .strict(StrictMode::TimingOnly)
            .pes_cap_per_pid(1 << 20)
            .pes_cap_total(8 << 20)
            .link_klv(0x100, 0x101)
            .build();
        assert_eq!(d._options.strict, StrictMode::TimingOnly);
        assert_eq!(d._options.pes_cap_per_pid, Some(1 << 20));
        assert_eq!(d._options.pes_cap_total, Some(8 << 20));
        assert_eq!(d._options.klv_link_overrides, vec![(0x100, 0x101)]);
    }

    #[test]
    fn default_caps_match_plan_decision() {
        // Spec §11.2 closure: 4 MiB / 64 MiB.
        assert_eq!(default_pes_cap_per_pid(), 4 * 1024 * 1024);
        assert_eq!(default_pes_cap_total(), 64 * 1024 * 1024);
    }
}
