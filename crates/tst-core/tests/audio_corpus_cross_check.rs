//! Opt-in corpus cross-check. Walks `tests/fixtures/local/*.ts` if the
//! directory exists (gitignored real-world capture corpus), parses every
//! audio PES with the new frame iterators, and asserts no panic. Tallies
//! each outcome bucket for visibility.
//!
//! Run:
//!   cargo test --test audio_corpus_cross_check -- --include-ignored
//!
//! The corpus has known shapes the parser must survive cleanly:
//! - Silent audio (header-valid frames carrying constant samples) →
//!   counted as `frames_parsed_ok`.
//! - "MP3" PIDs that are actually mislabeled private bytes → mostly
//!   `errors_bad_sync` with sporadic `frames_parsed_ok` from coincidental
//!   sync alignment.
//! - Empty PES payloads → no events, no errors.
//!
//! The test does NOT assert frames > 0 per PID; it asserts no panic
//! and prints a per-codec tally.

use std::fs;
use std::path::Path;
use tst_core::codec;
use tst_core::mpegts::demux::{AudioCodec, DemuxEvent, Demuxer, SamplePayload};

#[derive(Default, Debug)]
struct Tally {
    frames_parsed_ok: u64,
    errors_bad_sync: u64,
    errors_truncated: u64,
    errors_reserved: u64,
    errors_forbidden: u64,
    errors_other: u64,
}

impl Tally {
    fn record(&mut self, err: &codec::CodecParseError) {
        match err {
            codec::CodecParseError::BadSyncWord { .. } => self.errors_bad_sync += 1,
            codec::CodecParseError::Truncated { .. } => self.errors_truncated += 1,
            codec::CodecParseError::ReservedValue { .. } => self.errors_reserved += 1,
            codec::CodecParseError::Forbidden { .. } => self.errors_forbidden += 1,
            _ => self.errors_other += 1,
        }
    }
}

fn walk_file(path: &Path, tally_mp: &mut Tally, tally_aac: &mut Tally) {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(_) => return,
    };
    let mut demuxer = Demuxer::new();
    if demuxer.feed(&bytes).is_err() {
        return;
    }
    demuxer.flush();
    while let Some(ev) = demuxer.next_event() {
        if let DemuxEvent::Sample {
            payload: SamplePayload::Audio { codec, frames, .. },
            ..
        } = ev
        {
            match codec {
                AudioCodec::Mp2 => {
                    for r in codec::mpegaudio::frames(&frames) {
                        match r {
                            Ok(_) => tally_mp.frames_parsed_ok += 1,
                            Err(e) => tally_mp.record(&e),
                        }
                    }
                }
                AudioCodec::Aac => {
                    for r in codec::aac::frames(&frames) {
                        match r {
                            Ok(_) => tally_aac.frames_parsed_ok += 1,
                            Err(e) => tally_aac.record(&e),
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

#[test]
#[ignore]
fn audio_corpus_cross_check() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let corpus_dir = manifest_dir.join("tests/fixtures/local");
    if !corpus_dir.exists() {
        eprintln!("skipping: no corpus at {}", corpus_dir.display());
        return;
    }

    let mut tally_mp = Tally::default();
    let mut tally_aac = Tally::default();
    let mut files_seen = 0u32;

    for entry in walkdir_simple(&corpus_dir) {
        if entry.extension().and_then(|s| s.to_str()) == Some("ts") {
            walk_file(&entry, &mut tally_mp, &mut tally_aac);
            files_seen += 1;
        }
    }

    eprintln!("=== audio corpus cross-check ===");
    eprintln!("files: {}", files_seen);
    eprintln!("mpegaudio: {:?}", tally_mp);
    eprintln!("aac:       {:?}", tally_aac);
    // No assertion on counts — see header rustdoc for why.
}

/// Tiny recursive walker (avoid pulling in walkdir as a dev-dep).
fn walkdir_simple(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(it) => it,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}
