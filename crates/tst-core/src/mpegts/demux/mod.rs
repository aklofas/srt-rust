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

pub mod demuxer;
pub mod event;
mod payload;
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

pub mod low_level;

pub use demuxer::Demuxer;
pub use event::{
    AudioCodec, DemuxEvent, DiscontinuityKind, KlvLink, LinkSource, MetadataKind, NalUnit,
    NonConformantIssue, Obu, ObuExtension, ProgramMap, SamplePayload, StreamId, StreamInfo,
    StreamKind, SubtitleCodec, VideoCodec, VideoPayload, pts_to_duration,
};
pub use strict::StrictMode;
pub use types::{DemuxerBuilder, DemuxerConfig, DemuxerStats};
