//! End-to-end: MuxPublisher<HlsPublisher> → ffmpeg pulls /playlist.m3u8 → TS byte-identity.
//!
//! Skips gracefully if `ffmpeg` is not on $PATH.

#![cfg(feature = "hls")]

use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
use tst_core::publisher::Publisher;
use tst_pipeline::MuxPublisher;
use tst_tcp::hls::{HlsMode, HlsPublisherBuilder};

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg").arg("-version").output().is_ok()
}

fn tmpdir(label: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "hls-e2e-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn synthetic_h264_au() -> Vec<u8> {
    let mut v = vec![0x00, 0x00, 0x00, 0x01, 0x09, 0x10];
    v.extend([0x00, 0x00, 0x00, 0x01, 0x65]);
    v.extend(std::iter::repeat(0xab).take(200));
    v
}

#[test]
fn hls_pipeline_event_mode_writes_three_segments_locally() {
    let dir = tmpdir("event-3");
    let publisher = HlsPublisherBuilder::new()
        .bind("127.0.0.1:0".parse().unwrap())
        .output_dir(&dir)
        .segment_duration(Duration::from_secs(10))
        .playlist_window(6)
        .mode(HlsMode::Event)
        .build()
        .unwrap();

    let mux_cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x100, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.psi_interval_ms(10);
        b.build().unwrap()
    };

    let pub_shell = MuxPublisher::with_config(publisher, mux_cfg).unwrap();
    let au = synthetic_h264_au();
    for i in 0i64..3 {
        let pts = Pts90khz::new(i * 9001);
        pub_shell.send_video(&au, pts, true).unwrap();
    }

    let publisher = pub_shell.finish().unwrap();
    publisher.finish().unwrap();

    let pl = std::fs::read_to_string(dir.join("playlist.m3u8")).unwrap();
    assert!(pl.contains("#EXT-X-PLAYLIST-TYPE:EVENT"));
    assert!(pl.contains("#EXT-X-ENDLIST"));
    assert!(dir.join("segment_00000.ts").exists());
}

#[test]
fn hls_extinf_is_media_derived_and_target_is_immutable() {
    let dir = tmpdir("media-extinf");
    let publisher = HlsPublisherBuilder::new()
        .bind("127.0.0.1:0".parse().unwrap())
        .output_dir(&dir)
        .segment_duration(Duration::from_secs(4)) // configured target → ceil(4) = 4
        .playlist_window(6)
        .mode(HlsMode::Event)
        .build()
        .unwrap();

    let mux_cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x100, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.psi_interval_ms(10);
        b.build().unwrap()
    };

    let pub_shell = MuxPublisher::with_config(publisher, mux_cfg).unwrap();
    let au = synthetic_h264_au();
    // GOP cadence: IDR@0, P@60000, P@120000, IDR@180000, P@240000, P@300000, IDR@360000.
    //
    // Segment bookkeeping (segment_start_pts set on first push after a cut,
    // reset to None immediately after each keyframe cut):
    //   IDR@0:       start None→0; span(0,0)=0 → seg0 Duration::ZERO; reset.
    //   P@60000:     start None→60000.
    //   P@120000:    start stays 60000.
    //   IDR@180000:  span(60000,180000)=120000 ticks = 120000×1e9/90000 ns
    //                = 1_333_333_333 ns ≈ 1.333 s; reset.
    //   P@240000:    start None→240000.
    //   P@300000:    start stays 240000.
    //   IDR@360000:  span(240000,360000)=120000 ticks = 1.333 s; reset.
    let idr_ticks = [0i64, 180_000, 360_000];
    for (gop, &idr) in idr_ticks.iter().enumerate() {
        pub_shell.send_video(&au, Pts90khz::new(idr), true).unwrap();
        if gop + 1 < idr_ticks.len() {
            pub_shell
                .send_video(&au, Pts90khz::new(idr + 60_000), false)
                .unwrap();
            pub_shell
                .send_video(&au, Pts90khz::new(idr + 120_000), false)
                .unwrap();
        }
    }

    let _ = pub_shell.publisher_stats(); // touch stats path

    let publisher = pub_shell.finish().unwrap();
    let rendered = publisher.render_playlist(true);
    publisher.finish().unwrap();

    // Target duration is the immutable ceiling (4 s), not the actual segment
    // duration; it must not change across reloads.
    assert!(
        rendered.contains("#EXT-X-TARGETDURATION:4"),
        "playlist:\n{rendered}"
    );
    // Each non-degenerate segment carries a media-derived EXTINF of 1.333 s
    // (120000-tick PTS span).  The old wall-clock code would have produced
    // ~0.000 for fast ingestion.
    assert!(
        rendered.contains("#EXTINF:1.333,"),
        "expected a media-derived 1.333 s EXTINF, got:\n{rendered}"
    );
}

#[test]
fn hls_pipeline_via_ffmpeg_validates_playlist() {
    if !ffmpeg_available() {
        eprintln!("ffmpeg not on PATH — skipping HLS e2e validation");
        return;
    }

    let dir = tmpdir("ffmpeg");
    let publisher = HlsPublisherBuilder::new()
        .bind("127.0.0.1:0".parse().unwrap())
        .output_dir(&dir)
        .segment_duration(Duration::from_secs(10))
        .playlist_window(6)
        .mode(HlsMode::Event)
        .build()
        .unwrap();
    let addr = publisher.local_addr().unwrap();

    let mux_cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x100, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.psi_interval_ms(10);
        b.build().unwrap()
    };

    let pub_shell = MuxPublisher::with_config(publisher, mux_cfg).unwrap();
    let au = synthetic_h264_au();
    for i in 0i64..3 {
        let pts = Pts90khz::new(i * 9001);
        pub_shell.send_video(&au, pts, true).unwrap();
    }

    thread::sleep(Duration::from_millis(100));

    let url = format!("http://{}/playlist.m3u8", addr);
    let out = Command::new("ffmpeg")
        .args(["-y", "-i", &url, "-c", "copy", "-f", "mpegts", "-t", "1"])
        .arg(dir.join("ffmpeg_out.ts"))
        .output()
        .expect("ffmpeg failed to spawn");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Stream #0") || stderr.contains("Input #0"),
        "ffmpeg didn't see any stream — stderr:\n{stderr}"
    );

    let publisher = pub_shell.finish().unwrap();
    publisher.finish().unwrap();
}
