//! Sync-KLV round-trip: ST 1910 AU cell wrap on send, demux unwrap on receive.

use srt_core::klv::st0605::{PrecisionTimeStampPack, TimeStatus};
use srt_core::klv::st1910::wrap_au_cell;
use srt_core::mpegts::demux::{DemuxEvent, Demuxer, MetadataKind};
use srt_core::mpegts::mux::{ConfigBuilder, KlvStreamType, Muxer, VideoCodec as MuxVideoCodec};

#[test]
fn sync_klv_au_cell_unwraps_on_receive() {
    // Sync KLV per ST 1402 §8 / ST 1910: SynchronousMetadata stream_type
    // with PTS in PES and ST 1910 AU cell wrap on the payload.
    let cfg = ConfigBuilder::default()
        .add_video(0x100, MuxVideoCodec::H264)
        .add_klv(0x101, KlvStreamType::SynchronousMetadata, true)
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();
    mux.push_video(&[0x00, 0x00, 0x00, 0x01, 0x09, 0x10], 90_000, true)
        .unwrap();

    // Inner ST 0601 LS — UL + BER short-form length + 5-byte body.
    let inner_klv = vec![
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
        0x00, 0x02, 0x01, 0x00, 0x01, 0x00,
    ];
    let pack = PrecisionTimeStampPack {
        time_status: TimeStatus(0xFF),
        timestamp_us: 1_700_000_000_000_000,
    };
    // Caller-side wrap is mandatory for SynchronousMetadata — `push_klv`
    // does NOT auto-wrap (see CLAUDE.md "Important muxer invariant").
    let wrapped = wrap_au_cell(&inner_klv, pack);
    mux.push_klv(&wrapped, 90_000).unwrap();

    let mut bytes = Vec::new();
    let mut buf = vec![0u8; 1316];
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
    }

    let mut d = Demuxer::new();
    d.feed(&bytes).unwrap();
    let mut found = false;
    while let Some(ev) = d.next_event() {
        if let DemuxEvent::Metadata {
            kind: MetadataKind::KlvSyncAuCell,
            payload,
            ..
        } = ev
        {
            assert_eq!(
                payload, inner_klv,
                "demuxer should unwrap AU cell and surface the inner KLV LS"
            );
            found = true;
        }
    }
    assert!(
        found,
        "expected at least one DemuxEvent::Metadata with KlvSyncAuCell"
    );
}
