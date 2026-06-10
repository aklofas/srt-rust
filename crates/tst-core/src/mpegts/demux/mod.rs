//! DemuxReceiver-side MPEG-TS demuxer.
//!
//! Drive a [`Demuxer`] with 188-byte TS packets (or arbitrary bytes that
//! contain TS packets — the demuxer handles sync recovery). Pull events
//! out via `next_event`. The event stream is decoupled-pairing: the
//! demuxer extracts and timestamps streams independently and never pairs
//! sync-KLV with video AUs. See the cookbook for canonical pairing recipes.
//!
//! Lenient by default — non-conformance surfaces as
//! `DemuxEvent::NonConformant` events so the receive loop keeps running.
//! `DemuxerBuilder::strict(StrictMode::*)` opts in to hard-fail
//! categories for compliance / ingest workflows.

pub(crate) mod au_reassemble;
pub mod demuxer;
pub mod event;
pub(crate) mod payload;
mod pes;
mod psi;
pub(crate) mod psi_assembler;
pub mod strict;
// `pub(super)` (not `mod`) so `crate::mpegts::mux::mod.rs` round-trip tests
// can call `crate::mpegts::demux::ts::parse_ts_packet`. Not part of the
// public API surface — invisible outside the `mpegts` parent module.
pub(super) mod ts;
mod types;
mod sync_ingress;
mod pmt_classify;
mod psi_topology;
mod pes_emit;
mod stats_recorder;

pub mod low_level;

/// Test-only re-export. NOT a stable API; used by `tests/mpegts_au_cell_round_trip.rs`.
#[doc(hidden)]
pub mod payload_test_hooks {
    pub use super::payload::iter_au_cells;
}

pub use demuxer::Demuxer;
pub use event::{
    AudioCodec, Av1ObuHeaderKind, DemuxEvent, DiscontinuityKind, KlvLink, LinkSource, MetadataKind,
    NalHeaderKind, NalUnit, NonConformantIssue, Obu, ObuExtension, PcrMalformedKind,
    PesHeaderMalformedKind, ProgramMap, SamplePayload, StreamId, StreamInfo, StreamKind,
    StreamKindTag, SubtitleCodec, VideoCodec, VideoPayload, pts_to_duration,
};
pub use payload::{split_video, split_video_strict};
pub use strict::StrictMode;
pub use types::{DemuxerBuilder, DemuxerConfig, DemuxerStats};
