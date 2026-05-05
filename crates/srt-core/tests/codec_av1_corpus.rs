//! Opt-in real-corpus cross-check for `codec::av1`.
//!
//! Skipped unless `SRT_RUN_CORPUS_CROSSCHECK=1` is set. Compares our
//! parser's recovered max_frame_width / max_frame_height to ffprobe's
//! decoded values on any `.ts` files present in `tests/fixtures/av1/`
//! (typically produced by `regen.sh` if `ffmpeg --enable-libaom` is
//! available locally). Same shape as `codec_h266_corpus.rs`.
//!
//! profile string comparison is intentionally skipped — ffprobe's AV1
//! profile strings ("Main"/"High"/"Professional") don't have a stable
//! mapping to seq_profile (0/1/2) we want to assert against here. We
//! surface the parsed numeric profile in the success log line for human
//! inspection.
//!
//! No-ops cleanly when the env var is unset, when the fixtures directory
//! is missing, when ffprobe isn't on PATH, or when no `.ts` files are
//! present (e.g. fresh checkout where libaom-equipped ffmpeg wasn't
//! available to run `regen.sh`).

use srt_core::codec::av1;
use srt_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload};
use std::path::Path;
use std::process::Command;

fn ffprobe_available() -> bool {
    Command::new("ffprobe").arg("-version").output().is_ok()
}

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
fn av1_fixtures_match_ffprobe() {
    if std::env::var("SRT_RUN_CORPUS_CROSSCHECK").is_err() {
        eprintln!("skipping (set SRT_RUN_CORPUS_CROSSCHECK=1 to enable)");
        return;
    }
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/av1");
    if !fixtures_dir.is_dir() {
        eprintln!("skipping (no fixtures at {fixtures_dir:?})");
        return;
    }
    if !ffprobe_available() {
        eprintln!("skipping (ffprobe not available)");
        return;
    }

    let mut checked = 0usize;
    let mut mismatches = Vec::new();

    for entry in std::fs::read_dir(&fixtures_dir).unwrap() {
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

        // Walk events; collect OBUs across all AV1 Sample events, then run
        // the parser once. AV1 sequence headers don't have to land on the
        // first sample — accumulate then pick the first SH.
        let mut all_obus = Vec::new();
        while let Some(ev) = dx.next_event() {
            if let DemuxEvent::Sample {
                payload:
                    SamplePayload::Video {
                        codec: VideoCodec::Av1,
                        payload: VideoPayload::Obus(obus),
                        ..
                    },
                ..
            } = ev
            {
                all_obus.extend(obus);
            }
        }

        let mut got_dims: Option<(u32, u32)> = None;
        let mut got_profile: Option<u8> = None;
        if !all_obus.is_empty() {
            let stream = av1::parse_obu_stream(&all_obus);
            if let Some(sh) = stream.sequence_headers.first() {
                got_dims = Some((sh.max_frame_width, sh.max_frame_height));
                got_profile = Some(sh.profile);
            }
        }

        let Some((fw, fh, profile_str, level_str)) = ffprobe_video_stream(&p) else {
            eprintln!("ffprobe failed on {p:?}, skipping");
            continue;
        };

        match got_dims {
            Some((gw, gh)) if gw == fw && gh == fh => {
                eprintln!(
                    "{}: ok {}x{} (parsed seq_profile={}; ffprobe profile={}, level={})",
                    p.display(),
                    gw,
                    gh,
                    got_profile.unwrap_or(0),
                    profile_str,
                    level_str,
                );
                checked += 1;
            }
            Some((gw, gh)) => {
                mismatches.push(format!(
                    "{}: parsed {}x{} != ffprobe {}x{}",
                    p.display(),
                    gw,
                    gh,
                    fw,
                    fh
                ));
            }
            None => {
                mismatches.push(format!("{}: no SH extracted", p.display()));
            }
        }
    }

    eprintln!("av1 cross-check: {checked} fixtures matched ffprobe");
    assert!(
        mismatches.is_empty(),
        "mismatches:\n{}",
        mismatches.join("\n")
    );
}
