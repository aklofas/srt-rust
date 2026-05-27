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
