// crates/srt-core/src/mpegts/demux/mod.rs
//! Receiver-side MPEG-TS demuxer.
//!
//! Drive a [`Demuxer`] with 188-byte TS packets (or arbitrary bytes that
//! contain TS packets — the demuxer handles sync recovery). Pull events
//! out via `next_event`. The event stream is decoupled-pairing: the
//! demuxer extracts and timestamps streams independently and never pairs
//! sync-KLV with video AUs. See `srt-rust/docs/cookbook.md` for canonical
//! pairing recipes.
//!
//! Lenient by default — non-conformance surfaces as
//! `DemuxEvent::NonConformant` events so the receive loop keeps running.
//! `DemuxerBuilder::strict(StrictMode::*)` opts in to hard-fail
//! categories for compliance / ingest workflows.

pub mod demuxer;
pub mod event;
pub mod payload;
pub mod pes;
pub mod psi;
pub mod strict;
pub mod ts;

pub use demuxer::{Demuxer, DemuxerBuilder, DemuxerOptions, DemuxerStats};
pub use event::{
    AudioCodec, DemuxEvent, DiscontinuityKind, KlvLink, LinkSource, MetadataKind, NalUnit,
    NonConformantIssue, ProgramMap, SamplePayload, StreamId, StreamInfo, StreamKind, SubtitleCodec,
    VideoCodec, pts_to_duration,
};
pub use strict::StrictMode;
