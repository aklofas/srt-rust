//! Strict-mode integration tests for `mpegts::demux::Demuxer`.
//!
//! `StrictMode` converts selected `NonConformantIssue` categories into a
//! fatal `DemuxError::StrictRejection` returned out of `Demuxer::feed`,
//! instead of just queuing a `NonConformant` event for the caller to
//! inspect. These tests cover both directions:
//!
//! * `StrictMode::Full` — every issue category rejects.
//! * `StrictMode::Off` (default) — nothing rejects; the loop survives.
//!
//! The natural shape used here is "sync-typed KLV PID carrying bare async
//! KLV": configure `KlvStreamType::SynchronousMetadata` (PMT stream_type
//! 0x15) and push a raw bare ST 0601 LS via `Muxer::push_klv` (callers are
//! responsible for ST 1910 AU-cell wrapping; `push_klv` does not do it).
//! The demuxer's `classify_klv` returns `KlvShape::Async`, the PMT
//! declared the PID as sync, and the linkage builder emits
//! `NonConformantIssue::StreamTypeMismatchAsyncOnSyncPid`.
//!
//! Note: a 1-video + 1-KLV PMT does NOT trigger
//! `MissingMetadataDescriptor`. With a single video PID in the PMT, the
//! demuxer's linkage builder falls into the "infer from topology" arm
//! (`LinkSource::Inferred`) instead of the "no entry" arm. Triggering
//! `MissingMetadataDescriptor` requires a multi-video PMT, which the
//! current `mpegts::mux::Config::validate` rejects (Path 3 lifts that).

use srt_core::error::DemuxError;
use srt_core::mpegts::demux::{DemuxEvent, DemuxerBuilder, NonConformantIssue, StrictMode};
use srt_core::mpegts::mux::{ConfigBuilder, KlvStreamType, Muxer, VideoCodec as MuxVideoCodec};

/// A minimally well-formed bare ST 0601 LS: 16-byte UAS Datalink LS UL +
/// short-form BER length 0 + empty value. The demuxer's `classify_klv`
/// recognizes the SMPTE UL prefix `06 0E 2B 34` and returns
/// `KlvShape::Async`; the actual KLV payload contents don't matter for
/// classification.
const BARE_ASYNC_KLV: [u8; 17] = [
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00, 0x00,
    0x00,
];

fn drain(m: &mut Muxer) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1316];
    loop {
        let n = m.pull(&mut buf);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

/// Build a TS byte stream that triggers `StreamTypeMismatchAsyncOnSyncPid`:
/// PMT declares the KLV PID as sync (stream_type 0x15) but the PES payload
/// on that PID is bare async KLV. Returns the muxed bytes.
fn build_mismatched_stream() -> Vec<u8> {
    let cfg = ConfigBuilder::default()
        .add_program(1, 0x1000)
        .add_video(0x100, MuxVideoCodec::H264)
        .add_klv(0x101, KlvStreamType::SynchronousMetadata, true)
        .end_program()
        .build()
        .unwrap();
    let mut m = Muxer::new(cfg).unwrap();
    m.push_video(&[0x00, 0x00, 0x00, 0x01, 0x09, 0x10], 0, true)
        .unwrap();
    // Push raw bare KLV on a SynchronousMetadata-declared PID. `push_klv`
    // does NOT auto-wrap in an ST 1910 AU cell — callers are responsible
    // for that — so the wire payload is `KlvShape::Async` while the PMT
    // says sync, which is the mismatch the demuxer surfaces.
    m.push_klv(&BARE_ASYNC_KLV, 0).unwrap();
    drain(&mut m)
}

#[test]
fn strict_full_rejects_stream_type_mismatch() {
    let bytes = build_mismatched_stream();
    let mut d = DemuxerBuilder::new().strict(StrictMode::Full).build();
    let res = d.feed(&bytes);
    // The implementation queues the `NonConformant` event first, then
    // drains `fatal` at end of the packet loop and returns
    // `Err(StrictRejection)`. Both halves must hold: feed errors AND the
    // structured issue is retrievable from the queue after the error.
    let mut saw_mismatch = false;
    while let Some(ev) = d.next_event() {
        if matches!(
            ev,
            DemuxEvent::NonConformant {
                issue: NonConformantIssue::StreamTypeMismatchAsyncOnSyncPid,
                ..
            }
        ) {
            saw_mismatch = true;
        }
    }
    assert!(
        matches!(res, Err(DemuxError::StrictRejection(_))),
        "strict-Full feed should return Err(StrictRejection), got: {:?}",
        res
    );
    assert!(
        saw_mismatch,
        "strict-Full feed should queue StreamTypeMismatchAsyncOnSyncPid event before erroring"
    );
}

#[test]
fn strict_off_emits_event_keeps_running() {
    let bytes = build_mismatched_stream();
    let mut d = DemuxerBuilder::new().build(); // default = StrictMode::Off
    d.feed(&bytes).unwrap(); // should not error in StrictMode::Off

    // The lenient contract has two halves: feed returns Ok AND the
    // non-conformance is surfaced as an event. Asserting both prevents a
    // regression where the demuxer silently swallows issues.
    let mut saw_mismatch = false;
    while let Some(ev) = d.next_event() {
        if matches!(
            ev,
            DemuxEvent::NonConformant {
                issue: NonConformantIssue::StreamTypeMismatchAsyncOnSyncPid,
                ..
            }
        ) {
            saw_mismatch = true;
        }
    }
    assert!(
        saw_mismatch,
        "lenient mode should queue StreamTypeMismatchAsyncOnSyncPid event (and keep running)"
    );
}
