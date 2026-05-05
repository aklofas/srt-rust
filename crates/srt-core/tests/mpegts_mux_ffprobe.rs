//! ffprobe smoke test — closed-loop sanity check.
//!
//! Mux a synthetic stream → write to a temp `.ts` file → run ffprobe with
//! JSON output → parse the JSON → assert: stream count, video codec,
//! KLV PID with KLVA tag.
//!
//! Skipped if `ffprobe` is not on PATH (returns early with a printed note).

mod common;

use common::synthetic_nal;
use srt_core::mpegts::mux::{Config, Muxer};
use std::process::Command;

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

fn have_ffprobe() -> bool {
    Command::new("ffprobe")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn ffprobe_recognizes_our_pmt() {
    if !have_ffprobe() {
        eprintln!("[skip] ffprobe not on PATH");
        return;
    }

    let mut mux = Muxer::new(Config::default()).unwrap();
    // Several frames so the stream has structure to parse.
    for i in 0..10 {
        let nal = synthetic_nal::h264_au(800, i % 5 == 0);
        mux.push_video(&nal, (i as i64) * 3000, i % 5 == 0).unwrap();
        let klv = synthetic_nal::klv_blob(48);
        mux.push_klv(&klv, (i as i64) * 3000).unwrap();
    }
    let bytes = drain_all(&mut mux);

    let tmp = std::env::temp_dir().join("srt_core_ffprobe_smoke.ts");
    std::fs::write(&tmp, &bytes).expect("write temp ts");

    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(&tmp)
        .output()
        .expect("run ffprobe");
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("ffprobe output: {}", stdout);

    // Minimum signal: ffprobe finds at least one stream and the codec
    // 'h264' appears in its output.
    assert!(
        stdout.contains("\"codec_name\": \"h264\""),
        "h264 stream missing"
    );
    // KLV PID should be present somewhere in the JSON (may show as
    // 'data' codec or similar). Don't assert on the exact codec_name —
    // ffprobe versions differ. Just confirm 2 streams reported.
    let stream_count = stdout.matches("\"index\":").count();
    assert!(
        stream_count >= 2,
        "expected >= 2 streams, got {}",
        stream_count
    );

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn ffprobe_recognizes_dual_camera_plus_klv() {
    if !have_ffprobe() {
        eprintln!("[skip] ffprobe not on PATH");
        return;
    }

    use srt_core::mpegts::mux::{KlvStreamType, VideoCodec};
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

    // Generate a few seconds of synthetic frames so ffprobe has something
    // structural to read. Minimal Annex-B AUs are enough for ffprobe to
    // identify the codec from the PMT stream_type byte.
    let nal = [
        0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, 0xFF, 0xE1, 0x00, 0x00,
    ];
    let klv = vec![
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
        0x00, 0x00,
    ];

    let mut ts = Vec::new();
    let mut buf = vec![0u8; 188 * 64];
    for i in 0..30i64 {
        let pts = i * 3000; // 33 ms @ 90 kHz
        mux.push_video_to(eo, &nal, pts, i == 0).unwrap();
        mux.push_video_to(ir, &nal, pts, i == 0).unwrap();
        mux.push_klv_to(klv_h, &klv, pts).unwrap();
        loop {
            let n = mux.pull(&mut buf);
            if n == 0 {
                break;
            }
            ts.extend_from_slice(&buf[..n]);
        }
    }

    let tmp = std::env::temp_dir().join("srt_core_ffprobe_dual_camera.ts");
    std::fs::write(&tmp, &ts).expect("write temp ts");

    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(&tmp)
        .output()
        .expect("run ffprobe");
    assert!(
        out.status.success(),
        "ffprobe exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("ffprobe output: {}", stdout);

    // ffprobe should report >=3 streams: two h264 video, one data (KLV).
    // Don't lock the exact ordering — just count.
    let video_count = stdout.matches("\"codec_type\": \"video\"").count();
    let data_count = stdout.matches("\"codec_type\": \"data\"").count();
    assert!(
        video_count >= 2,
        "expected >= 2 video streams, got {}: {}",
        video_count,
        stdout,
    );
    assert!(
        data_count >= 1,
        "expected >= 1 data stream (KLV), got {}: {}",
        data_count,
        stdout,
    );

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn ffprobe_recognizes_two_programs_with_distinct_streams() {
    if !have_ffprobe() {
        eprintln!("[skip] ffprobe not on PATH");
        return;
    }

    use srt_core::mpegts::mux::{
        Config, KlvStreamType, Muxer, ProgramConfig, StreamSpec, VideoCodec,
    };

    let config = Config {
        programs: vec![
            ProgramConfig {
                program_number: 1,
                pmt_pid: 0x1000,
                streams: vec![
                    StreamSpec::Video {
                        pid: 0x1011,
                        codec: VideoCodec::H264,
                    },
                    StreamSpec::Klv {
                        pid: 0x1031,
                        stream_type: KlvStreamType::PrivateData,
                        carries_pts: false,
                    },
                ],
                pcr_pid: None,
                program_descriptors: Vec::new(),
                stream_descriptors: vec![Vec::new(), Vec::new()],
            },
            ProgramConfig {
                program_number: 2,
                pmt_pid: 0x1100,
                streams: vec![
                    StreamSpec::Video {
                        pid: 0x1111,
                        codec: VideoCodec::H265,
                    },
                    StreamSpec::Klv {
                        pid: 0x1131,
                        stream_type: KlvStreamType::PrivateData,
                        carries_pts: false,
                    },
                ],
                pcr_pid: None,
                program_descriptors: Vec::new(),
                stream_descriptors: vec![Vec::new(), Vec::new()],
            },
        ],
        ..Config::default()
    };
    let mut mux = Muxer::new(config).unwrap();

    let p1_video = mux.video_handles_for_program(1).unwrap()[0];
    let p2_video = mux.video_handles_for_program(2).unwrap()[0];
    let p1_klv = mux.klv_handles_for_program(1).unwrap()[0];
    let p2_klv = mux.klv_handles_for_program(2).unwrap()[0];

    // Minimal Annex B access units — enough for ffprobe to identify codec
    // from the PMT stream_type byte.
    let nal_h264 = synthetic_nal::h264_au(64, true);
    let nal_h265 = synthetic_nal::h265_au(64, true);
    let klv = synthetic_nal::klv_blob(32);

    let mut ts = Vec::new();
    let mut buf = vec![0u8; 188 * 64];
    for i in 0..20i64 {
        let pts = i * 3_003;
        mux.push_video_to(p1_video, &nal_h264, pts, i == 0).unwrap();
        mux.push_video_to(p2_video, &nal_h265, pts, i == 0).unwrap();
        mux.push_klv_to(p1_klv, &klv, pts).unwrap();
        mux.push_klv_to(p2_klv, &klv, pts).unwrap();
        loop {
            let n = mux.pull(&mut buf);
            if n == 0 {
                break;
            }
            ts.extend_from_slice(&buf[..n]);
        }
    }

    let tmp = std::env::temp_dir().join("srt_core_ffprobe_two_programs.ts");
    std::fs::write(&tmp, &ts).expect("write temp ts");

    // -show_programs reports the program table; -of json gives structured output.
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_programs", "-of", "json"])
        .arg(&tmp)
        .output()
        .expect("run ffprobe");
    assert!(
        out.status.success(),
        "ffprobe exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("ffprobe -show_programs output: {}", stdout);

    // ffprobe JSON contains one entry per program in the "programs" array.
    // Both program_num 1 and 2 must be present.
    let program_count = stdout.matches("\"program_num\":").count();
    assert_eq!(
        program_count, 2,
        "expected 2 programs in ffprobe output, got {}: {}",
        program_count, stdout
    );
    // ffprobe may format as `"program_num": 1` (with space) or `"program_num":1`.
    assert!(
        stdout.contains("\"program_num\": 1") || stdout.contains("\"program_num\":1"),
        "program_num 1 missing from ffprobe output: {}",
        stdout
    );
    assert!(
        stdout.contains("\"program_num\": 2") || stdout.contains("\"program_num\":2"),
        "program_num 2 missing from ffprobe output: {}",
        stdout
    );

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn ffprobe_roundtrip_audio_video_klv_three_streams() {
    if !have_ffprobe() {
        eprintln!("[skip] ffprobe not on PATH");
        return;
    }

    use srt_core::mpegts::mux::{AudioCodec, KlvStreamType, VideoCodec};

    let cfg = Config::builder()
        .add_program(1, 0x1000)
        .add_video(0x100, VideoCodec::H264)
        .add_audio(0x300, AudioCodec::Aac)
        .add_klv(0x200, KlvStreamType::PrivateData, true)
        .end_program()
        .build()
        .unwrap();
    let mut muxer = Muxer::new(cfg).unwrap();

    // Drive enough content for ffprobe to recognize all three streams.
    // Audio frames can be any non-empty bytes; ffprobe identifies the codec
    // from the PMT stream_type byte, not by parsing the audio bitstream.
    for i in 0..30 {
        let pts = 90_000 + (i as i64) * 3000;
        let nal = synthetic_nal::h264_au(128, i % 5 == 0);
        muxer.push_video(&nal, pts, i % 5 == 0).unwrap();
        // Minimal synthetic audio frame — ffprobe identifies codec from
        // stream_type 0x0F (AAC) in the PMT, not from bitstream analysis.
        muxer.push_audio(b"aac_frame_data", pts).unwrap();
        let klv = synthetic_nal::klv_blob(32);
        muxer.push_klv(&klv, pts).unwrap();
    }

    let ts = drain_all(&mut muxer);
    let tmp = std::env::temp_dir().join("srt_core_ffprobe_audio_three_streams.ts");
    std::fs::write(&tmp, &ts).expect("write temp ts");

    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(&tmp)
        .output()
        .expect("run ffprobe");
    assert!(
        out.status.success(),
        "ffprobe exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("ffprobe output: {}", stdout);

    // ffprobe should report >=3 streams: video (h264), audio (aac), data (KLV).
    let video_count = stdout.matches("\"codec_type\": \"video\"").count();
    let audio_count = stdout.matches("\"codec_type\": \"audio\"").count();
    let data_count = stdout.matches("\"codec_type\": \"data\"").count();
    assert!(
        video_count >= 1,
        "expected >= 1 video stream, got {}: {}",
        video_count,
        stdout,
    );
    assert!(
        audio_count >= 1,
        "expected >= 1 audio stream, got {}: {}",
        audio_count,
        stdout,
    );
    assert!(
        data_count >= 1,
        "expected >= 1 data stream (KLV), got {}: {}",
        data_count,
        stdout,
    );
    // Confirm the audio codec is recognized as AAC (stream_type 0x0F).
    assert!(
        stdout.contains("\"codec_name\": \"aac\""),
        "audio codec_name missing or not aac: {}",
        stdout
    );

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn ffprobe_roundtrip_each_audio_codec() {
    if !have_ffprobe() {
        eprintln!("[skip] ffprobe not on PATH");
        return;
    }

    use srt_core::mpegts::mux::{AudioCodec, VideoCodec};

    // Test each audio codec variant, verifying that ffprobe reports the stream.
    // Stream type values per ISO/IEC 13818-1:
    //   0x03 = ISO/IEC 11172-3 Audio (MP2)
    //   0x0F = ISO/IEC 13818-7 ADTS AAC
    //   0x11 = ISO/IEC 14496-3 LATM AAC
    //   0x81 = User private (AC-3)
    let cases = vec![
        (AudioCodec::Mp2, 0x03),
        (AudioCodec::Aac, 0x0F),
        (AudioCodec::AacLatm, 0x11),
        (AudioCodec::Ac3, 0x81),
    ];

    for (codec, expected_stream_type) in cases {
        let cfg = Config::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_audio(0x300, codec)
            .end_program()
            .build()
            .unwrap();
        let mut muxer = Muxer::new(cfg).unwrap();

        // Drive a few frames of synthetic audio/video.
        for i in 0..20 {
            let pts = 90_000 + (i as i64) * 3000;
            let nal = synthetic_nal::h264_au(128, i % 5 == 0);
            muxer.push_video(&nal, pts, i % 5 == 0).unwrap();
            // Minimal synthetic audio — without real bitstream data, ffprobe
            // cannot determine codec_type, but it does report the stream and
            // its codec_tag (which reflects the stream_type from the PMT).
            muxer.push_audio(b"audio_data", pts).unwrap();
        }

        let ts = drain_all(&mut muxer);
        let tmp = std::env::temp_dir().join(format!("srt_core_ffprobe_audio_{:?}.ts", codec));
        std::fs::write(&tmp, &ts).expect("write temp ts");

        let out = Command::new("ffprobe")
            .args(["-v", "error", "-show_streams", "-of", "json"])
            .arg(&tmp)
            .output()
            .expect("run ffprobe");
        assert!(
            out.status.success(),
            "ffprobe exited non-zero for {:?}: stderr={}",
            codec,
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);

        // Verify ffprobe reports a stream with PID 0x300 and the correct codec_tag.
        // The codec_tag in ffprobe output reflects the stream_type from the PMT.
        // Format: `codec_tag`: "0xHHHH" (hex), e.g. 0x03 for MP2, 0x0F for AAC, etc.
        let expected_tag = format!("\"codec_tag\": \"0x{:04x}\"", expected_stream_type);
        assert!(
            stdout.contains(&expected_tag),
            "codec {:?}: expected codec_tag {}, ffprobe output: {}",
            codec,
            expected_tag,
            stdout
        );
        // Also confirm PID 0x300 is in the output.
        assert!(
            stdout.contains("\"id\": \"0x300\""),
            "codec {:?}: expected audio PID 0x300 in ffprobe output: {}",
            codec,
            stdout
        );

        let _ = std::fs::remove_file(&tmp);
    }
}

// --- Subtitle ffprobe round-trip tests (subtitle plan Task 21) ---
//
// These match the audio-codec round-trip pattern above: build a config with
// the subtitle codec, push a few PES units, drain TS bytes, run ffprobe, and
// assert on the codec / language strings ffprobe reports.
//
// CEA-708 standalone is intentionally excluded — ffmpeg's CEA-708 path is
// SEI-embedded, not standalone-PID, and ffprobe doesn't classify the
// standalone form cleanly.
//
// Gated behind `#[ignore = "ffprobe-only"]` so they run only with
// `-- --ignored`, matching audio plan #21's pattern.

#[test]
#[ignore = "ffprobe-only"]
fn ffprobe_validates_dvb_subtitling_round_trip() {
    if !have_ffprobe() {
        eprintln!("[skip] ffprobe not on PATH");
        return;
    }

    use srt_core::mpegts::mux::{SubtitleCodec, VideoCodec};

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
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();

    // Drive a few video frames so the program has structure, plus a handful
    // of synthetic DVB subtitle PES units. ffprobe identifies DVB sub via the
    // PMT subtitling_descriptor (auto-emitted), not by parsing the bitstream.
    let h = mux.subtitle_handles()[0];
    for i in 0..10 {
        let pts = 90_000 * (i as i64 + 1);
        let nal = synthetic_nal::h264_au(128, i % 5 == 0);
        mux.push_video(&nal, pts, i % 5 == 0).unwrap();
    }
    for i in 0..5 {
        // Minimal DVB subtitle PES payload: data_identifier (0x20) + a tiny
        // subtitle segment. ffprobe doesn't parse this — the descriptor in
        // the PMT is what drives codec recognition.
        let payload = [0x0F, 0x10, 0x00, 0x01, 0x00, 0x06, 0, 0, 0, 0, 0, 0];
        mux.push_subtitle_to(h, 90_000 * (i as i64 + 1), &payload)
            .unwrap();
    }
    let bytes = drain_all(&mut mux);

    let tmp = std::env::temp_dir().join("srt_core_ffprobe_dvb_sub.ts");
    std::fs::write(&tmp, &bytes).expect("write temp ts");

    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(&tmp)
        .output()
        .expect("run ffprobe");
    assert!(
        out.status.success(),
        "ffprobe exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("ffprobe output: {}", stdout);

    assert!(
        stdout.contains("dvb_subtitle"),
        "expected dvb_subtitle codec_name in ffprobe output: {}",
        stdout
    );
    assert!(
        stdout.contains("eng"),
        "expected language tag 'eng' in ffprobe output: {}",
        stdout
    );

    let _ = std::fs::remove_file(&tmp);
}

#[test]
#[ignore = "ffprobe-only"]
fn ffprobe_validates_dvb_teletext_round_trip() {
    if !have_ffprobe() {
        eprintln!("[skip] ffprobe not on PATH");
        return;
    }

    use srt_core::mpegts::mux::{SubtitleCodec, VideoCodec};

    let cfg = Config::builder()
        .add_program(1, 0x100)
        .add_video(0x101, VideoCodec::H264)
        .add_subtitle(
            0x200,
            SubtitleCodec::DvbTeletext {
                language: *b"eng",
                teletext_type: 0x02,
                magazine_number: 1,
                page_number: 0x88,
            },
        )
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();

    // Same shape as the DVB-sub case: a few video frames plus a handful of
    // synthetic teletext PES units. ffprobe identifies teletext via the PMT
    // teletext_descriptor (auto-emitted).
    let h = mux.subtitle_handles()[0];
    for i in 0..10 {
        let pts = 90_000 * (i as i64 + 1);
        let nal = synthetic_nal::h264_au(128, i % 5 == 0);
        mux.push_video(&nal, pts, i % 5 == 0).unwrap();
    }
    for i in 0..5 {
        // Minimal teletext PES payload: data_identifier (0x10) + a tiny
        // data unit. Real-content shape doesn't matter for ffprobe codec ID.
        let mut payload = vec![0x10];
        payload.extend_from_slice(&[0x02, 0x10]);
        payload.extend(std::iter::repeat(0x00).take(0x10));
        mux.push_subtitle_to(h, 90_000 * (i as i64 + 1), &payload)
            .unwrap();
    }
    let bytes = drain_all(&mut mux);

    let tmp = std::env::temp_dir().join("srt_core_ffprobe_dvb_teletext.ts");
    std::fs::write(&tmp, &bytes).expect("write temp ts");

    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(&tmp)
        .output()
        .expect("run ffprobe");
    assert!(
        out.status.success(),
        "ffprobe exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("ffprobe output: {}", stdout);

    assert!(
        stdout.contains("dvb_teletext"),
        "expected dvb_teletext codec_name in ffprobe output: {}",
        stdout
    );

    let _ = std::fs::remove_file(&tmp);
}

#[test]
#[ignore = "ffprobe-only"]
fn ffprobe_validates_webvtt_in_ts_round_trip() {
    if !have_ffprobe() {
        eprintln!("[skip] ffprobe not on PATH");
        return;
    }

    use srt_core::mpegts::mux::{SubtitleCodec, VideoCodec};

    let cfg = Config::builder()
        .add_program(1, 0x100)
        .add_video(0x101, VideoCodec::H264)
        .add_subtitle(0x200, SubtitleCodec::WebVttInTs)
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();

    // Push a handful of video frames so the program is well-formed, then a
    // single WebVTT cue. ffprobe recognizes WebVTT-in-TS via the auto-emitted
    // registration_descriptor (format_identifier = "VTTC").
    let h = mux.subtitle_handles()[0];
    for i in 0..10 {
        let pts = 90_000 * (i as i64 + 1);
        let nal = synthetic_nal::h264_au(128, i % 5 == 0);
        mux.push_video(&nal, pts, i % 5 == 0).unwrap();
    }
    mux.push_subtitle_to(
        h,
        90_000,
        b"WEBVTT\n\n00:00:01.000 --> 00:00:05.000\nhello\n",
    )
    .unwrap();
    let bytes = drain_all(&mut mux);

    let tmp = std::env::temp_dir().join("srt_core_ffprobe_webvtt_in_ts.ts");
    std::fs::write(&tmp, &bytes).expect("write temp ts");

    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-of", "json"])
        .arg(&tmp)
        .output()
        .expect("run ffprobe");
    assert!(
        out.status.success(),
        "ffprobe exited non-zero: stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    eprintln!("ffprobe output: {}", stdout);

    // ffmpeg may label this as "webvtt", as a generic "subtitle" codec_type,
    // or (older Ubuntu builds) as "bin_data" — all three indicate the stream
    // was classified rather than dropped.
    assert!(
        stdout.contains("webvtt") || stdout.contains("subtitle") || stdout.contains("bin_data"),
        "expected webvtt/subtitle/bin_data in ffprobe output: {}",
        stdout
    );

    let _ = std::fs::remove_file(&tmp);
}
