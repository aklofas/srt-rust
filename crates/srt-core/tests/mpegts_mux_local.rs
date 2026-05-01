//! Real-corpus integration tests.
//!
//! Looks for `.ts` files in `tests/fixtures/local/` (gitignored). For each
//! file: extracts video AUs and KLV blobs, re-muxes them through our
//! `Muxer`, parses the output, asserts structural agreement.
//!
//! Skipped silently when `local/` is absent or empty — matches the existing
//! `klv` local-fixtures pattern.

mod common;

use common::ts_parser;
use srt_core::mpegts::mux::{Config, Muxer, VideoCodec};
use std::fs;
use std::path::Path;

const FIXTURES: &str = "tests/fixtures/local";

fn list_ts_files() -> Vec<std::path::PathBuf> {
    match fs::read_dir(FIXTURES) {
        Ok(rd) => rd
            .filter_map(|r| r.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ts"))
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn drain_all(mux: &mut Muxer) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1316];
    loop {
        let n = mux.pull(&mut buf).unwrap();
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&buf[..n]);
    }
}

#[test]
fn corpus_replay_structural_match() {
    let files = list_ts_files();
    if files.is_empty() {
        eprintln!("[skip] no fixtures in {}", FIXTURES);
        return;
    }
    for path in files {
        process_one(&path);
    }
}

fn process_one(path: &Path) {
    eprintln!("processing {}", path.display());
    let data = fs::read(path).expect("read fixture");

    // Use the existing parser to find what's in the original.
    let original = ts_parser::parse(&data);
    let video_stream = match original
        .streams
        .iter()
        .find(|s| s.stream_type == 0x1B || s.stream_type == 0x24)
    {
        Some(s) => s,
        None => {
            eprintln!("  no video stream — skipping");
            return;
        }
    };
    let klv_stream = original.streams.iter().find(|s| s.klva);

    let codec = if video_stream.stream_type == 0x1B {
        VideoCodec::H264
    } else {
        VideoCodec::H265
    };

    // Re-mux with our Muxer.
    let cfg = Config {
        video_pid: 0x1011,
        klv_pid: 0x1031,
        video_codec: codec,
        buffer_packets: 200_000,
        ..Default::default()
    };
    let mut mux = Muxer::new(cfg).unwrap();

    if let Some(orig_video) = original.pes_by_pid.get(&video_stream.pid) {
        for (i, (pts, body)) in orig_video.iter().enumerate() {
            // We can't perfectly detect IDR without parsing slice headers —
            // assume every 30th frame is a key frame for the replay.
            let key = i % 30 == 0;
            if !body.starts_with(&[0x00, 0x00, 0x00, 0x01])
                && !body.starts_with(&[0x00, 0x00, 0x01])
            {
                eprintln!("  skipping non-Annex-B AU at index {}", i);
                continue;
            }
            mux.push_video(body, pts.unwrap_or(0) as i64, key)
                .expect("push_video");
        }
    }

    if let Some(ks) = klv_stream {
        if let Some(orig_klv) = original.pes_by_pid.get(&ks.pid) {
            for (pts, body) in orig_klv {
                mux.push_klv(body, pts.unwrap_or(0) as i64)
                    .expect("push_klv");
            }
        }
    }

    let bytes = drain_all(&mut mux);
    let recovered = ts_parser::parse(&bytes);

    // Structural assertions:
    assert!(recovered.pmt_pid.is_some(), "PMT not present in output");
    assert!(
        recovered
            .streams
            .iter()
            .any(|s| s.stream_type == video_stream.stream_type),
        "video stream_type mismatch: orig {:#x}",
        video_stream.stream_type
    );
    if let Some(ks) = klv_stream {
        let our_klv = recovered.streams.iter().find(|s| s.klva);
        assert!(our_klv.is_some(), "KLVA descriptor missing in our output");
        let orig_count = original
            .pes_by_pid
            .get(&ks.pid)
            .map(|v| v.len())
            .unwrap_or(0);
        let our_count = recovered
            .pes_by_pid
            .get(&our_klv.unwrap().pid)
            .map(|v| v.len())
            .unwrap_or(0);
        assert_eq!(orig_count, our_count, "KLV PES count differs");
    }
}
