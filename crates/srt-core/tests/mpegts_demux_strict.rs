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
//! The natural shape used here is "async-typed KLV PID carrying sync-shaped
//! AU-cell-wrapped payload": configure `KlvStreamType::PrivateData` (PMT
//! stream_type 0x06; passes payload through unchanged) and push pre-wrapped
//! bytes that form a valid H.222.0 §2.12.4.2 Metadata_AU_cell. The demuxer's
//! `classify_klv` returns `KlvShape::SyncAuCell`, the PMT declared the PID
//! as async, and the linkage builder emits
//! `NonConformantIssue::StreamTypeMismatchSyncOnAsyncPid`.
//!
//! Why this direction (sync-on-async) and not the other (async-on-sync):
//! `KlvStreamType::SynchronousMetadata` triggers automatic AU-cell wrapping
//! in `Muxer::push_klv_to`, so the wire form on a sync-declared PID always
//! matches the declaration — there's no in-API way to produce async-on-sync
//! mismatches anymore. PrivateData streams pass payload through unchanged,
//! so we can hand the muxer pre-wrapped sync-shaped bytes to drive the
//! sync-on-async mismatch path.
//!
//! Note: a 1-video + 1-KLV PMT does NOT trigger
//! `MissingMetadataDescriptor`. With a single video PID in the PMT, the
//! demuxer's linkage builder falls into the "infer from topology" arm
//! (`LinkSource::Inferred`) instead of the "no entry" arm. Triggering
//! `MissingMetadataDescriptor` requires a multi-video PMT, which the
//! current `mpegts::mux::Config::validate` rejects (Path 3 lifts that).

use srt_core::error::DemuxError;
use srt_core::mpegts::au_cell::{AuCellHeader, CellFragmentIndication, write_metadata_au_cell};
use srt_core::mpegts::demux::{DemuxEvent, DemuxerBuilder, NonConformantIssue, StrictMode};
use srt_core::mpegts::mux::{ConfigBuilder, KlvStreamType, Muxer, VideoCodec as MuxVideoCodec};

/// A minimally well-formed bare ST 0601 LS used as the inner payload of a
/// synthetic AU cell: 16-byte UAS Datalink LS UL + short-form BER length 0.
const BARE_KLV_LS: [u8; 17] = [
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

/// Build a TS byte stream that triggers `StreamTypeMismatchSyncOnAsyncPid`:
/// PMT declares the KLV PID as async (stream_type 0x06, PrivateData) but
/// the PES payload is a sync-shaped AU-cell-wrapped KLV. Returns the muxed
/// bytes. PrivateData streams pass payload through unchanged, so the caller
/// can directly emit a sync-shaped wire form.
fn build_mismatched_stream() -> Vec<u8> {
    let cfg = ConfigBuilder::default()
        .add_program(1, 0x1000)
        .add_video(0x100, MuxVideoCodec::H264)
        .add_klv(0x101, KlvStreamType::PrivateData, true)
        .end_program()
        .build()
        .unwrap();
    let mut m = Muxer::new(cfg).unwrap();
    m.push_video(&[0x00, 0x00, 0x00, 0x01, 0x09, 0x10], 0, true)
        .unwrap();
    // Pre-wrap a synthetic Metadata_AU_cell carrying a bare ST 0601 LS, then
    // push as PrivateData so the muxer passes it through as-is. Wire form
    // is `KlvShape::SyncAuCell` while PMT says async (stream_type 0x06) —
    // the mismatch the demuxer surfaces as StreamTypeMismatchSyncOnAsyncPid.
    let mut wrapped = Vec::new();
    let header = AuCellHeader {
        metadata_service_id: 0x00,
        sequence_number: 0,
        cell_fragment_indication: CellFragmentIndication::Complete,
        decoder_config_flag: false,
        random_access_indicator: true,
    };
    write_metadata_au_cell(&mut wrapped, header, &BARE_KLV_LS).unwrap();
    m.push_klv(&wrapped, 0).unwrap();
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
                issue: NonConformantIssue::StreamTypeMismatchSyncOnAsyncPid,
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
        "strict-Full feed should queue StreamTypeMismatchSyncOnAsyncPid event before erroring"
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
                issue: NonConformantIssue::StreamTypeMismatchSyncOnAsyncPid,
                ..
            }
        ) {
            saw_mismatch = true;
        }
    }
    assert!(
        saw_mismatch,
        "lenient mode should queue StreamTypeMismatchSyncOnAsyncPid event (and keep running)"
    );
}
