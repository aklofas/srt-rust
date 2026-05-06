//! Integration tests for subtitle / caption sender side
//! (`mpegts::mux`).

use tst_core::error::MuxError;
use tst_core::mpegts::mux::{Config, KlvStreamType, Muxer, SubtitleCodec, VideoCodec};

/// Drain every queued packet from the muxer into a single Vec.
fn drain_all(mux: &mut Muxer) -> Vec<u8> {
    let mut all = Vec::new();
    let mut buf = vec![0u8; 188 * 256];
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        all.extend_from_slice(&buf[..n]);
    }
    all
}

fn webvtt_cue(pts: i64, text: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(b"WEBVTT\n\n");
    buf.extend_from_slice(
        format!(
            "{:02}:{:02}:{:02}.{:03} --> {:02}:{:02}:{:02}.{:03}\n",
            0, 0, 0, 0, 0, 0, 5, 0
        )
        .as_bytes(),
    );
    buf.extend_from_slice(text.as_bytes());
    buf.push(b'\n');
    let _ = pts;
    buf
}

#[test]
fn mux_webvtt_in_single_program() {
    let cfg = Config::builder()
        .add_program(1, 0x100)
        .add_video(0x101, VideoCodec::H264)
        .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();
    let h = mux.subtitle_handles()[0];
    mux.push_subtitle_to(h, 90_000, &webvtt_cue(90_000, "POI #1"))
        .unwrap();
    mux.push_subtitle_to(h, 90_000 * 5, &webvtt_cue(90_000 * 5, "POI #2"))
        .unwrap();
    let out = drain_all(&mut mux);
    // PSI + 2 subtitle PES at minimum.
    assert!(out.len() > 188 * 3);
    // Subtitle PID must appear in TS packets.
    assert!(
        out.chunks_exact(188)
            .any(|p| { ((((p[1] & 0x1F) as u16) << 8) | (p[2] as u16)) == 0x200 })
    );
}

#[test]
fn mux_dvb_subtitling_multiple_languages() {
    let cfg = Config::builder()
        .add_program(1, 0x100)
        .add_video(0x101, VideoCodec::H264)
        .add_subtitle(
            0x200,
            SubtitleCodec::DvbSubtitling {
                language: *b"eng",
                subtitling_type: 0x10,
                composition_page_id: 1,
                ancillary_page_id: 1,
            },
        )
        .add_subtitle(
            0x201,
            SubtitleCodec::DvbSubtitling {
                language: *b"spa",
                subtitling_type: 0x10,
                composition_page_id: 2,
                ancillary_page_id: 2,
            },
        )
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();
    assert_eq!(mux.subtitle_handles().len(), 2);
    let handles = mux.subtitle_handles();
    for (i, h) in handles.iter().enumerate() {
        mux.push_subtitle_to(*h, 90_000 * (i as i64 + 1), b"DVBSUB segment")
            .unwrap();
    }
    let out = drain_all(&mut mux);
    assert!(!out.is_empty());
}

#[test]
fn mux_subtitle_with_klv_same_program_no_classification_collision() {
    let cfg = Config::builder()
        .add_program(1, 0x100)
        .add_video(0x101, VideoCodec::H264)
        .add_klv(
            0x300,
            KlvStreamType::PrivateData,
            /* carries_pts = */ false,
        )
        .add_subtitle(0x400, SubtitleCodec::WebVttInTs)
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();
    mux.push_klv(b"\x06\x0E\x2B\x34short", 90_000).unwrap();
    let h = mux.subtitle_handles()[0];
    mux.push_subtitle_to(h, 90_000, b"WEBVTT\n").unwrap();
    let out = drain_all(&mut mux);
    // Both PIDs distinct in TS packet stream.
    let pids: std::collections::HashSet<u16> = out
        .chunks_exact(188)
        .map(|p| (((p[1] & 0x1F) as u16) << 8) | (p[2] as u16))
        .collect();
    assert!(pids.contains(&0x300));
    assert!(pids.contains(&0x400));
}

#[test]
fn mux_multi_program_webvtt_and_klv() {
    let cfg = Config::builder()
        .add_program(1, 0x100)
        .add_video(0x101, VideoCodec::H264)
        .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
        .end_program()
        .add_program(2, 0x300)
        .add_video(0x301, VideoCodec::H265)
        .add_klv(0x400, KlvStreamType::PrivateData, false)
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();
    let h = mux.subtitle_handles()[0];
    mux.push_subtitle_to(h, 90_000, b"WEBVTT\n").unwrap();
    let out = drain_all(&mut mux);
    assert!(!out.is_empty());
}

#[test]
fn mux_push_subtitle_too_large_rejected() {
    let cfg = Config::builder()
        .add_program(1, 0x100)
        .add_video(0x101, VideoCodec::H264)
        .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();
    let too_big = vec![0u8; 70_000];
    let err = mux.push_subtitle(0, &too_big).unwrap_err();
    assert!(matches!(err, MuxError::SubtitleTooLarge { .. }));
}
