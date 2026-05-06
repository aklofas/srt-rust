//! Smoke test for the `file` feature helpers.

#![cfg(feature = "file")]

use tst_core::io_file::{DemuxFromFile, demux_file, write_mux_to_file};
use tst_core::mpegts::mux::{Config, Muxer};

/// Build a minimal single-program Muxer with one H.264 video stream.
fn minimal_muxer() -> Muxer {
    Muxer::new(Config::default()).expect("Muxer::new")
}

/// Push one IDR frame and one KLV blob so the muxer has output to drain.
fn push_one_keyframe(mux: &mut Muxer) {
    // Synthetic Annex-B H.264 IDR NAL (start code + IDR header byte).
    let nal: Vec<u8> = {
        let mut v = vec![0x00, 0x00, 0x00, 0x01];
        v.push(0x65); // forbidden_zero=0, nal_ref_idc=3, nal_unit_type=5 (IDR)
        v.extend(vec![0xA5u8; 64]);
        v
    };
    mux.push_video(&nal, 0, true).expect("push_video");
}

#[test]
fn demux_file_round_trip() {
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");

    let mut mux = minimal_muxer();
    push_one_keyframe(&mut mux);
    write_mux_to_file(&mut mux, tmp.path()).expect("write_mux_to_file");

    let events = demux_file(tmp.path()).expect("demux_file");
    assert!(!events.is_empty(), "expected at least one DemuxEvent");
}

#[test]
fn demux_from_file_streaming() {
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");

    let mut mux = minimal_muxer();
    push_one_keyframe(&mut mux);
    write_mux_to_file(&mut mux, tmp.path()).expect("write_mux_to_file");

    let count = DemuxFromFile::open(tmp.path())
        .expect("DemuxFromFile::open")
        .count();
    assert!(count > 0, "expected at least one event from DemuxFromFile");
}

#[test]
fn write_mux_creates_nonempty_file() {
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");

    let mut mux = minimal_muxer();
    push_one_keyframe(&mut mux);
    write_mux_to_file(&mut mux, tmp.path()).expect("write_mux_to_file");

    let metadata = std::fs::metadata(tmp.path()).expect("metadata");
    assert!(metadata.len() > 0, "output file must not be empty");
    // TS files are multiples of 188 bytes.
    assert_eq!(metadata.len() % 188, 0, "TS file length must be a multiple of 188");
}
