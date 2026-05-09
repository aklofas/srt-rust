//! Round-trip integration tests for `mpegts::mux::Muxer`.
//!
//! Always-on, hermetic: synthetic input → mux → in-process TS parser →
//! recovered video AU + KLV blob → byte-equality assertions. Covers the
//! H.264/H.265 axis × the four KLV stream_type/PTS axes.

mod common;

use common::synthetic_nal;
use common::ts_parser;
use tst_core::mpegts::mux::{KlvStreamType, Muxer, MuxerConfig, StreamSpec, VideoCodec};

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
fn h264_async_klv_roundtrip() {
    let mut mux = Muxer::new(MuxerConfig::default()).unwrap();
    let video = synthetic_nal::h264_au(800, true);
    let klv = synthetic_nal::klv_blob(64);
    mux.push_video(&video, 0, true).unwrap();
    mux.push_klv(&klv, 0, 0x00).unwrap();
    let bytes = drain_all(&mut mux);
    assert!(!bytes.is_empty());

    let parsed = ts_parser::parse(&bytes);
    assert_eq!(parsed.pmt_pid, Some(0x1000));
    assert_eq!(parsed.streams.len(), 2);
    let video_stream = parsed
        .streams
        .iter()
        .find(|s| s.stream_type == 0x1B)
        .unwrap();
    let klv_stream = parsed
        .streams
        .iter()
        .find(|s| s.stream_type == 0x06)
        .unwrap();
    assert!(klv_stream.klva, "KLV stream must have KLVA descriptor");
    let video_pes = parsed.pes_by_pid.get(&video_stream.pid).unwrap();
    let klv_pes = parsed.pes_by_pid.get(&klv_stream.pid).unwrap();
    assert_eq!(video_pes.len(), 1);
    assert_eq!(klv_pes.len(), 1);
    // Async KLV: no PTS in PES.
    assert_eq!(klv_pes[0].0, None);
    assert_eq!(klv_pes[0].1, klv);
    assert_eq!(video_pes[0].1, video);
}

#[test]
fn h265_async_klv_roundtrip() {
    let cfg = MuxerConfig::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H265)
        .add_klv(0x1031, KlvStreamType::PrivateData, false)
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();
    let video = synthetic_nal::h265_au(1200, true);
    let klv = synthetic_nal::klv_blob(50);
    mux.push_video(&video, 0, true).unwrap();
    mux.push_klv(&klv, 0, 0x00).unwrap();
    let bytes = drain_all(&mut mux);

    let parsed = ts_parser::parse(&bytes);
    let video_stream = parsed
        .streams
        .iter()
        .find(|s| s.stream_type == 0x24)
        .unwrap();
    assert_eq!(
        parsed.pes_by_pid.get(&video_stream.pid).unwrap()[0].1,
        video
    );
}

#[test]
fn h264_klv_with_pts_keeps_pts() {
    let mut cfg = MuxerConfig::default();
    if let Some(StreamSpec::Klv { carries_pts, .. }) = cfg.programs[0]
        .streams
        .iter_mut()
        .find(|s| matches!(s, StreamSpec::Klv { .. }))
    {
        *carries_pts = true;
    }
    let mut mux = Muxer::new(cfg).unwrap();
    let video = synthetic_nal::h264_au(800, true);
    let klv = synthetic_nal::klv_blob(64);
    mux.push_video(&video, 90_000, true).unwrap();
    mux.push_klv(&klv, 90_000, 0x00).unwrap();
    let bytes = drain_all(&mut mux);

    let parsed = ts_parser::parse(&bytes);
    let klv_stream = parsed
        .streams
        .iter()
        .find(|s| s.stream_type == 0x06)
        .unwrap();
    let klv_pes = &parsed.pes_by_pid.get(&klv_stream.pid).unwrap()[0];
    assert_eq!(klv_pes.0, Some(90_000));
    assert_eq!(klv_pes.1, klv);
}

#[test]
fn h264_sync_metadata_stream_type() {
    let mut cfg = MuxerConfig::default();
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
    cfg.validate().unwrap();
    let mut mux = Muxer::new(cfg).unwrap();
    mux.push_video(&synthetic_nal::h264_au(500, true), 0, true)
        .unwrap();
    mux.push_klv(&synthetic_nal::klv_blob(40), 0, 0x00).unwrap();
    let bytes = drain_all(&mut mux);

    let parsed = ts_parser::parse(&bytes);
    let klv_stream = parsed
        .streams
        .iter()
        .find(|s| s.stream_type == 0x15)
        .unwrap();
    // SynchronousMetadata KLV PIDs also receive an auto-emitted KLVA
    // Registration descriptor — ffmpeg mpegtsenc.c:817-818 emits KLVA on
    // the 0x15 path too; receivers gate KLV classification on the
    // descriptor regardless of stream_type.
    assert!(klv_stream.klva);
}

#[test]
fn multiple_video_aus_recovered_in_order() {
    let mut mux = Muxer::new(MuxerConfig::default()).unwrap();
    let frames: Vec<Vec<u8>> = (0..5)
        .map(|i| synthetic_nal::h264_au(400 + i * 100, i == 0))
        .collect();
    for (i, f) in frames.iter().enumerate() {
        mux.push_video(f, (i as i64) * 3000, i == 0).unwrap();
    }
    let bytes = drain_all(&mut mux);
    let parsed = ts_parser::parse(&bytes);
    let video_stream = parsed
        .streams
        .iter()
        .find(|s| s.stream_type == 0x1B)
        .unwrap();
    let recovered = parsed.pes_by_pid.get(&video_stream.pid).unwrap();
    assert_eq!(recovered.len(), 5);
    for (i, (_pts, body)) in recovered.iter().enumerate() {
        assert_eq!(*body, frames[i], "AU {}", i);
    }
}

#[test]
fn psi_re_emitted_after_interval() {
    let cfg = MuxerConfig {
        psi_interval_ms: 100,
        ..MuxerConfig::default()
    };
    let mut mux = Muxer::new(cfg).unwrap();
    // Three video frames, 200 ms apart in PTS — should trigger 3 PSI emissions.
    for i in 0..3 {
        let nal = synthetic_nal::h264_au(500, i == 0);
        mux.push_video(&nal, (i as i64) * 200 * 90, i == 0).unwrap();
    }
    let bytes = drain_all(&mut mux);
    // Count PAT (PID 0) occurrences.
    let pat_count = bytes
        .chunks_exact(188)
        .filter(|p| {
            let pid = (((p[1] as u16) & 0x1F) << 8) | (p[2] as u16);
            pid == 0x0000
        })
        .count();
    assert_eq!(
        pat_count, 3,
        "expected 3 PAT emissions for 3 frames at 200ms apart"
    );
}

#[test]
fn pcr_pid_pinned_to_video_is_declared_in_pmt() {
    // Caller pins PCR to the video PID explicitly; muxer must reflect this
    // in the PMT's PCR_PID field so receivers know where to look.
    let mut cfg = MuxerConfig::default();
    let video_pid = cfg.programs[0]
        .streams
        .iter()
        .find_map(|s| match s {
            StreamSpec::Video { pid, .. } => Some(*pid),
            _ => None,
        })
        .unwrap();
    cfg.programs[0].pcr_pid = Some(video_pid);
    cfg.validate().unwrap();
    let mut mux = Muxer::new(cfg).unwrap();
    mux.push_video(&synthetic_nal::h264_au(500, true), 0, true)
        .unwrap();
    mux.push_klv(&synthetic_nal::klv_blob(64), 0, 0x00).unwrap();
    let bytes = drain_all(&mut mux);

    let parsed = ts_parser::parse(&bytes);
    assert_eq!(
        parsed.pcr_pid,
        Some(video_pid),
        "PMT should declare PCR_PID = video_pid when pcr_pid is pinned to it"
    );
}

#[test]
fn pcr_is_carried_on_video_pid_packets_by_default() {
    // PCR follows the video PID when no explicit pcr_pid is configured
    // (fallback chain: video > KLV > audio). Real TS packets on the video
    // PID must carry the PCR adaptation field; KLV packets must not.
    let cfg = MuxerConfig::default();
    let video_pid = cfg.programs[0]
        .streams
        .iter()
        .find_map(|s| match s {
            StreamSpec::Video { pid, .. } => Some(*pid),
            _ => None,
        })
        .unwrap();
    let klv_pid = cfg.programs[0]
        .streams
        .iter()
        .find_map(|s| match s {
            StreamSpec::Klv { pid, .. } => Some(*pid),
            _ => None,
        })
        .unwrap();
    cfg.validate().unwrap();
    let mut mux = Muxer::new(cfg).unwrap();
    for i in 0..3 {
        mux.push_video(&synthetic_nal::h264_au(500, true), i * 3_600_000, true)
            .unwrap();
        mux.push_klv(&synthetic_nal::klv_blob(64), i * 3_600_000, 0x00)
            .unwrap();
    }
    let bytes = drain_all(&mut mux);

    let parsed = ts_parser::parse(&bytes);
    let on_video = parsed
        .pcr_samples
        .iter()
        .filter(|(pid, _)| *pid == video_pid)
        .count();
    let on_klv = parsed
        .pcr_samples
        .iter()
        .filter(|(pid, _)| *pid == klv_pid)
        .count();
    assert!(
        on_video > 0,
        "expected PCR on video_pid={video_pid:#06x}; pcr_samples={:?}",
        parsed.pcr_samples
    );
    assert_eq!(
        on_klv, 0,
        "PCR follows video by default; no PCR should appear on klv_pid={klv_pid:#06x}; pcr_samples={:?}",
        parsed.pcr_samples
    );
}
