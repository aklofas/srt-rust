//! End-to-end round-trip of the H.222.0 §2.12.4.2 Metadata_AU_cell wrapper
//! via the muxer's auto-wrap path and the demuxer's classify_klv recognition.
//!
//! Replaces the obsolete tests/mpegts_mux_st1910.rs + tests/mpegts_demux_st1910.rs
//! which exercised the fictional UL+BER+PTSP wrapper format.

use tst_core::mpegts::au_cell::{CellFragmentIndication, read_metadata_au_cell};
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, MetadataKind};
use tst_core::mpegts::mux::{ConfigBuilder, KlvStreamType, Muxer, VideoCodec};

fn synthetic_klv_ls() -> Vec<u8> {
    let mut v = Vec::new();
    // 16-byte ST 0601 LS UL.
    v.extend_from_slice(&[
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
        0x00,
    ]);
    // BER short-form length = 4.
    v.push(0x04);
    // 4 body bytes (just markers; no real ST 0601 content).
    v.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    v
}

fn drain(mux: &mut Muxer) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1316];
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

#[test]
fn sync_klv_mux_demux_round_trip() {
    let cfg = ConfigBuilder::default()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264)
        .add_klv(
            0x1031,
            KlvStreamType::SynchronousMetadata,
            /*carries_pts=*/ true,
        )
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();

    let inner = synthetic_klv_ls();
    mux.push_klv(&inner, 90_000).unwrap();
    let ts_buf = drain(&mut mux);

    let mut dem = Demuxer::new();
    dem.feed(&ts_buf).unwrap();

    let mut sync_klv_seen = 0;
    while let Some(evt) = dem.next_event() {
        if let DemuxEvent::Metadata {
            kind:
                MetadataKind::KlvSyncAuCell {
                    metadata_service_id,
                    sequence_number,
                    cell_fragment_indication,
                    decoder_config_flag,
                    random_access_indicator,
                },
            payload,
            pts,
            ..
        } = evt
        {
            assert_eq!(payload, inner, "inner KLV must round-trip byte-for-byte");
            assert_eq!(pts, 90_000, "PES PTS must surface unchanged");
            // Mux defaults from Plan #25 Task 2:
            assert_eq!(metadata_service_id, 0x00, "ST 1402.2 App. B default");
            assert_eq!(sequence_number, 0, "first push starts at seq 0");
            assert_eq!(
                cell_fragment_indication,
                CellFragmentIndication::Complete,
                "single-cell AU"
            );
            assert!(!decoder_config_flag, "we never carry decoder config");
            assert!(random_access_indicator, "ST 0601 records are entry points");
            sync_klv_seen += 1;
        }
    }
    assert_eq!(sync_klv_seen, 1, "exactly one sync KLV event expected");
}

#[test]
fn private_data_klv_does_not_auto_wrap() {
    // PrivateData streams are caller-controlled; the muxer must NOT prepend
    // an AU cell header. Push raw bytes; demuxer must surface them as-is
    // via MetadataKind::KlvAsync (the bare SMPTE UL signal).
    let cfg = ConfigBuilder::default()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264)
        .add_klv(
            0x1031,
            KlvStreamType::PrivateData,
            /*carries_pts=*/ false,
        )
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();

    let inner = synthetic_klv_ls();
    mux.push_klv(&inner, 0).unwrap();
    let ts_buf = drain(&mut mux);

    let mut dem = Demuxer::new();
    dem.feed(&ts_buf).unwrap();

    let mut async_klv_seen = 0;
    while let Some(evt) = dem.next_event() {
        if let DemuxEvent::Metadata {
            kind: MetadataKind::KlvAsync,
            payload,
            ..
        } = evt
        {
            assert_eq!(payload, inner);
            async_klv_seen += 1;
        }
    }
    assert_eq!(async_klv_seen, 1, "exactly one async KLV event expected");
}

#[test]
fn sync_klv_sequence_number_increments_across_pushes() {
    // Verifies the muxer's per-stream sequence_number counter increments
    // mod 256 across push_klv calls. Validates by reading the AU cell
    // header out of the emitted PES payload directly.
    let cfg = ConfigBuilder::default()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264)
        .add_klv(0x1031, KlvStreamType::SynchronousMetadata, true)
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();

    let inner = synthetic_klv_ls();

    let next_seq_num = |mux: &mut Muxer| {
        let mut pes_payload = Vec::new();
        let mut buf = vec![0u8; 1316];
        loop {
            let n = mux.pull(&mut buf);
            if n == 0 {
                break;
            }
            for pkt in buf[..n].chunks_exact(188) {
                let pid = ((pkt[1] as u16 & 0x1F) << 8) | pkt[2] as u16;
                if pid != 0x1031 {
                    continue;
                }
                let payload_unit_start = (pkt[1] & 0x40) != 0;
                let adaptation_present = (pkt[3] & 0x20) != 0;
                let mut idx = 4usize;
                if adaptation_present {
                    let af_len = pkt[idx] as usize;
                    idx += 1 + af_len;
                }
                if payload_unit_start && idx + 9 <= 188 {
                    let pes_header_data_length = pkt[idx + 8] as usize;
                    idx += 9 + pes_header_data_length;
                }
                if idx < 188 {
                    pes_payload.extend_from_slice(&pkt[idx..188]);
                }
            }
        }
        let (hdr, _) = read_metadata_au_cell(&pes_payload).unwrap();
        hdr.sequence_number
    };

    mux.push_klv(&inner, 90_000).unwrap();
    assert_eq!(next_seq_num(&mut mux), 0);

    mux.push_klv(&inner, 90_000 * 2).unwrap();
    assert_eq!(next_seq_num(&mut mux), 1);

    mux.push_klv(&inner, 90_000 * 3).unwrap();
    assert_eq!(next_seq_num(&mut mux), 2);
}
