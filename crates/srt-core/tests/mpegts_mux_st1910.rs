//! Sync-mode round-trip: mux + klv::st1910 AU cell wrapping.

mod common;

use common::synthetic_nal;
use common::ts_parser;
use srt_core::klv::st0605::{PrecisionTimeStampPack, TimeStatus};
use srt_core::klv::st1910;
use srt_core::mpegts::mux::{Config, KlvStreamType, Muxer, StreamSpec};

fn drain_all(mux: &mut Muxer) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1316];
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&buf[..n]);
    }
}

#[test]
fn sync_klv_with_st1910_wrapper_roundtrip() {
    let mut cfg = Config::default();
    if let Some(StreamSpec::Klv { carries_pts, .. }) = cfg.programs[0]
        .streams
        .iter_mut()
        .find(|s| matches!(s, StreamSpec::Klv { .. }))
    {
        *carries_pts = true;
    }
    let mut mux = Muxer::new(cfg).unwrap();

    let video = synthetic_nal::h264_au(800, true);
    mux.push_video(&video, 0, true).unwrap();

    let inner_klv = synthetic_nal::klv_blob(120);
    let micros: u64 = 1_700_000_000_000_000;
    let pack = PrecisionTimeStampPack {
        time_status: TimeStatus(0xFF),
        timestamp_us: micros,
    };
    let wrapped = st1910::wrap_au_cell(&inner_klv, pack);
    mux.push_klv(&wrapped, 90_000).unwrap();

    let bytes = drain_all(&mut mux);
    let parsed = ts_parser::parse(&bytes);

    let klv_stream = parsed.streams.iter().find(|s| s.klva).unwrap();
    let pes = parsed.pes_by_pid.get(&klv_stream.pid).unwrap();
    assert_eq!(pes.len(), 1);
    let (pts, body) = &pes[0];
    assert_eq!(*pts, Some(90_000));

    let (recovered_inner, recovered_pack) = st1910::unwrap_au_cell(body).unwrap();
    assert_eq!(recovered_inner, &inner_klv[..]);
    assert_eq!(recovered_pack.timestamp_us, micros);
}

#[test]
fn sync_metadata_stream_type_with_wrapped_klv() {
    let mut cfg = Config::default();
    if let Some(StreamSpec::Klv {
        stream_type,
        carries_pts,
        ..
    }) = cfg.programs[0]
        .streams
        .iter_mut()
        .find(|s| matches!(s, StreamSpec::Klv { .. }))
    {
        *stream_type = KlvStreamType::SynchronousMetadata;
        *carries_pts = true;
    }
    let mut mux = Muxer::new(cfg).unwrap();
    mux.push_video(&synthetic_nal::h264_au(500, true), 0, true)
        .unwrap();

    let inner_klv = synthetic_nal::klv_blob(80);
    let pack = PrecisionTimeStampPack {
        time_status: TimeStatus(0xFF),
        timestamp_us: 1_700_000_001_000_000,
    };
    let wrapped = st1910::wrap_au_cell(&inner_klv, pack);
    mux.push_klv(&wrapped, 0).unwrap();

    let bytes = drain_all(&mut mux);
    let parsed = ts_parser::parse(&bytes);
    // stream_type 0x15 + KLVA descriptor + AU cell payload.
    let klv = parsed
        .streams
        .iter()
        .find(|s| s.stream_type == 0x15)
        .unwrap();
    let pes = parsed.pes_by_pid.get(&klv.pid).unwrap();
    let (_pts, body) = &pes[0];
    let (rec_inner, _) = st1910::unwrap_au_cell(body).unwrap();
    assert_eq!(rec_inner, &inner_klv[..]);
}
