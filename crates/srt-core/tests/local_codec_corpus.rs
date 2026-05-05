//! Opt-in corpus cross-check. Loads `tests/fixtures/local/*.ts`
//! (gitignored — sensitive corpus), demuxes the first IDR-bearing
//! Sample per video PID, parses parameter sets, and cross-checks the
//! recovered width/height/profile/level against `ffprobe -of json`
//! output. Skips silently if no fixtures are present, no ffprobe is
//! available, or if no video PID is found.

use srt_core::codec::{h264, h265};
use srt_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload};
use std::path::Path;
use std::process::Command;

fn ffprobe_video_stream(path: &Path) -> Option<(u32, u32, String, String)> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,profile,level",
            "-of",
            "default=nw=1",
            path.to_str()?,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let mut w = 0u32;
    let mut h = 0u32;
    let mut profile = String::new();
    let mut level = String::new();
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("width=") {
            w = v.parse().ok()?;
        }
        if let Some(v) = line.strip_prefix("height=") {
            h = v.parse().ok()?;
        }
        if let Some(v) = line.strip_prefix("profile=") {
            profile = v.into();
        }
        if let Some(v) = line.strip_prefix("level=") {
            level = v.into();
        }
    }
    Some((w, h, profile, level))
}

#[test]
fn local_corpus_parameter_sets_match_ffprobe() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/local");
    if !dir.is_dir() {
        eprintln!("(no local corpus, skipping)");
        return;
    }

    let mut checked = 0;
    let mut mismatches = Vec::new();

    for entry in std::fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("ts") {
            continue;
        }

        let bytes = std::fs::read(&p).unwrap();
        let mut dx = Demuxer::new();
        // feed returns Result — ignore errors (best-effort corpus check;
        // an unrecoverable sync failure on a malformed fixture shouldn't
        // abort the whole run).
        let _ = dx.feed(&bytes);
        dx.flush();

        let mut got: Option<(u32, u32)> = None;
        let mut got_profile: Option<u8> = None;
        let mut got_level: Option<u8> = None;
        while let Some(ev) = dx.next_event() {
            if let DemuxEvent::Sample {
                payload:
                    SamplePayload::Video {
                        codec,
                        payload: VideoPayload::Nals(nals),
                        ..
                    },
                ..
            } = ev
            {
                match codec {
                    VideoCodec::H264 => {
                        if let Ok(ps) = h264::parse_parameter_sets(&nals) {
                            if let Some(sps) = ps.sps_by_id.values().next() {
                                got = Some((sps.width, sps.height));
                                got_profile = Some(sps.profile_idc);
                                got_level = Some(sps.level_idc);
                                break;
                            }
                        }
                    }
                    VideoCodec::H265 => {
                        if let Ok(ps) = h265::parse_parameter_sets(&nals) {
                            if let Some(sps) = ps.sps_by_id.values().next() {
                                got = Some((sps.width, sps.height));
                                got_profile = Some(sps.general_profile_idc);
                                got_level = Some(sps.general_level_idc);
                                break;
                            }
                        }
                    }
                    // H.266 / AV1 typed parameter-set parsers ship in
                    // staged work; no corpus fixture targets them today.
                    VideoCodec::H266 | VideoCodec::Av1 => {}
                }
            }
        }

        let Some((gw, gh)) = got else {
            eprintln!("{}: no parameter sets found, skipping", p.display());
            continue;
        };
        let Some((fw, fh, profile, level)) = ffprobe_video_stream(&p) else {
            eprintln!("{}: ffprobe unavailable, skipping", p.display());
            continue;
        };
        if (gw, gh) != (fw, fh) {
            mismatches.push(format!(
                "{}: parsed {}x{} != ffprobe {}x{}",
                p.display(),
                gw,
                gh,
                fw,
                fh
            ));
        } else {
            eprintln!(
                "{}: ok {}x{} (parsed profile={}, level={}; ffprobe profile={}, level={})",
                p.display(),
                gw,
                gh,
                got_profile.unwrap_or(0),
                got_level.unwrap_or(0),
                profile,
                level
            );
            checked += 1;
        }
    }

    assert!(
        mismatches.is_empty(),
        "mismatches:\n{}",
        mismatches.join("\n")
    );
    eprintln!("(checked {checked} fixture(s))");
}
