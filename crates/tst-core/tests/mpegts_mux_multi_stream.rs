//! Multi-stream integration tests for `mpegts::mux::Muxer`.
//!
//! Covers the five shapes introduced by the multi-stream lift:
//! dual-video + KLV (EO+IR pod), video + dual-KLV, video-only,
//! KLV-only, and dual-video + dual-KLV (worst case for PMT enumeration).
//!
//! The bit-level wire-format checks live in the existing
//! `mpegts_mux_ffprobe.rs` and `mpegts_mux.rs` files; these tests focus
//! on routing — the right bytes go to the right PIDs.

use tst_core::mpegts::mux::{Config, KlvStreamType, Muxer, VideoCodec};

/// Drain every queued packet from the muxer into a single Vec.
fn drain_all(mux: &mut Muxer) -> Vec<u8> {
    let mut all = Vec::new();
    let mut buf = vec![0u8; 188 * 64];
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        all.extend_from_slice(&buf[..n]);
    }
    all
}

/// Extract the set of PIDs present in a TS byte stream.
fn pids_present(ts: &[u8]) -> std::collections::BTreeSet<u16> {
    let mut s = std::collections::BTreeSet::new();
    for chunk in ts.chunks_exact(188) {
        let pid = (((chunk[1] as u16) & 0x1F) << 8) | (chunk[2] as u16);
        s.insert(pid);
    }
    s
}

/// Minimal Annex-B H.264 NAL — start code + nal_unit_type byte + payload.
fn h264_au(payload: u8) -> Vec<u8> {
    vec![0x00, 0x00, 0x00, 0x01, 0x67, payload, 0xFF]
}

/// Minimal 17-byte KLV (16-byte UL + 1-byte length=0).
fn klv_blob() -> Vec<u8> {
    vec![
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
        0x00, 0x00,
    ]
}

#[test]
fn dual_video_plus_klv_routes_to_three_pids() {
    let cfg = Config::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264) // EO
        .add_video(0x1021, VideoCodec::H264) // IR
        .add_klv(0x1031, KlvStreamType::PrivateData, false)
        .pcr_pid(0x1011)
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();

    let eo = mux.video_stream_handle(0).unwrap();
    let ir = mux.video_stream_handle(1).unwrap();
    let klv_h = mux.klv_stream_handle(0).unwrap();

    // Push three frames — one per stream.
    mux.push_video_to(eo, &h264_au(0xAA), 0, true).unwrap();
    mux.push_video_to(ir, &h264_au(0xBB), 0, true).unwrap();
    mux.push_klv_to(klv_h, &klv_blob(), 0).unwrap();

    let ts = drain_all(&mut mux);
    let pids = pids_present(&ts);
    assert!(pids.contains(&0x0000), "expected PAT (PID 0)");
    assert!(pids.contains(&0x1000), "expected PMT (PID 0x1000)");
    assert!(pids.contains(&0x1011), "expected EO video PID");
    assert!(pids.contains(&0x1021), "expected IR video PID");
    assert!(pids.contains(&0x1031), "expected KLV PID");
}

#[test]
fn video_plus_dual_klv_routes_to_three_pids() {
    let cfg = Config::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264)
        .add_klv(0x1031, KlvStreamType::PrivateData, false) // vehicle telemetry
        .add_klv(0x1041, KlvStreamType::PrivateData, true) // sensor metadata (sync)
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();

    let v = mux.video_stream_handle(0).unwrap();
    let k_async = mux.klv_stream_handle(0).unwrap();
    let k_sync = mux.klv_stream_handle(1).unwrap();

    mux.push_video_to(v, &h264_au(0xAA), 0, true).unwrap();
    mux.push_klv_to(k_async, &klv_blob(), 0).unwrap();
    mux.push_klv_to(k_sync, &klv_blob(), 0).unwrap();

    let pids = pids_present(&drain_all(&mut mux));
    for required in [0x0000u16, 0x1000, 0x1011, 0x1031, 0x1041] {
        assert!(pids.contains(&required), "missing PID 0x{required:04X}");
    }
}

#[test]
fn video_only_emits_video_pid_only() {
    let cfg = Config::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264)
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();
    let v = mux.video_stream_handle(0).unwrap();
    mux.push_video_to(v, &h264_au(0xAA), 0, true).unwrap();

    let pids = pids_present(&drain_all(&mut mux));
    assert!(pids.contains(&0x1011));
    assert!(
        !pids.contains(&0x1031),
        "no KLV stream configured — must not emit on default KLV PID"
    );
}

#[test]
fn klv_only_emits_klv_pid_only() {
    let cfg = Config::builder()
        .add_program(1, 0x1000)
        .add_klv(0x1031, KlvStreamType::PrivateData, true)
        .pcr_pid(0x1031)
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();
    let k = mux.klv_stream_handle(0).unwrap();
    mux.push_klv_to(k, &klv_blob(), 0).unwrap();

    let pids = pids_present(&drain_all(&mut mux));
    assert!(pids.contains(&0x1031));
    assert!(
        !pids.contains(&0x1011),
        "no video stream configured — must not emit on default video PID"
    );
}

#[test]
fn dual_video_plus_dual_klv_pmt_lists_all_four() {
    // PAT PID is 0x0000 by ISO 13818-1 spec.
    const PAT_PID: u16 = 0x0000;

    let cfg = Config::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264)
        .add_video(0x1021, VideoCodec::H265)
        .add_klv(0x1031, KlvStreamType::PrivateData, false)
        .add_klv(0x1041, KlvStreamType::SynchronousMetadata, true)
        .pcr_pid(0x1011)
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();

    // Push to one stream to trigger first PSI emission.
    let v0 = mux.video_stream_handle(0).unwrap();
    mux.push_video_to(v0, &h264_au(0xAA), 0, true).unwrap();

    let ts = drain_all(&mut mux);
    let pids = pids_present(&ts);
    assert!(pids.contains(&PAT_PID));
    // The PMT itself rides PID 0x1000 (default).
    assert!(pids.contains(&0x1000), "PMT PID 0x1000 missing");
    // Find the PMT TS packet — first one with PID 0x1000 — and assert it
    // names all four elementary PIDs in its payload.
    let pmt_pkt = ts
        .chunks_exact(188)
        .find(|p| ((((p[1] as u16) & 0x1F) << 8) | (p[2] as u16)) == 0x1000)
        .expect("at least one PMT packet");
    // Search the entire 188-byte packet for each elementary PID encoded as
    // two consecutive bytes (high << 8 | low) — a coarse but reliable check
    // because PIDs are encoded in big-endian inside the PMT body.
    for elem in [0x1011u16, 0x1021, 0x1031, 0x1041] {
        let hi = (elem >> 8) as u8;
        let lo = elem as u8;
        let found = pmt_pkt
            .windows(2)
            .any(|w| w[0] & 0x1F == hi & 0x1F && w[1] == lo);
        assert!(
            found,
            "PMT body does not reference elementary PID 0x{elem:04X}"
        );
    }
}
