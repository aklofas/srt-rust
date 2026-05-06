//! Mux H.264 video + WebVTT-in-MPEG-TS subtitle cues into a `.ts`
//! file. Demonstrates the POI (Point of Interest) sparse-cue
//! injection pattern: the caller pushes a subtitle PES every time a
//! POI fires, with PTS aligned to the live-stream wall-clock.
//!
//! This example exercises [`Muxer::push_subtitle_to`] directly. A
//! consumer driving the SRT pipeline would build a `MuxSender<T>` over
//! the muxer config and call `MuxSender::send_subtitle_to` with the
//! same handle — the sender wraps the muxer's TS output in SRT and
//! drives the same `push_subtitle_to` path internally.
//!
//! Usage:
//!     cargo run --example mux_with_webvtt_subtitles -- output.ts
//!
//! Open the resulting file with `ffprobe` to verify:
//!     ffprobe -show_streams output.ts

use std::env;
use std::fs::File;
use std::io::Write;
use std::time::Duration;

use tst_core::mpegts::mux::{Config, Muxer, SubtitleCodec, VideoCodec};

// PIDs are 13-bit identifiers in the TS header. The reserved
// well-known values are 0x0000 (PAT) and 0x1FFF (null padding);
// elementary streams live in 0x0010..=0x1FFE. Within that range the
// muxer doesn't care which PID a stream sits on as long as PIDs don't
// collide — the receiver discovers them from the PMT.
const PCR_PID: u16 = 0x100;
const VIDEO_PID: u16 = 0x101;
const SUBTITLE_PID: u16 = 0x200;

/// Drain every queued packet from the muxer into a single `Vec<u8>`.
///
/// Mirrors the helper in `gen_subtitle_fixtures.rs` — there's no
/// public `drain_output` on `Muxer`, so we pull in chunks until
/// `pull` returns 0 (queue empty). Sized at 188 * 256 to amortize the
/// per-call cost; the muxer doesn't care about chunk size as long as
/// it's a non-trivial multiple of 188.
fn drain_all(mux: &mut Muxer) -> Vec<u8> {
    let mut all = Vec::new();
    let mut chunk = vec![0u8; 188 * 256];
    loop {
        let n = mux.pull(&mut chunk);
        if n == 0 {
            break;
        }
        all.extend_from_slice(&chunk[..n]);
    }
    all
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args().nth(1).unwrap_or_else(|| "output.ts".into());

    // Configure a single program with H.264 video on PID 0x101 and
    // WebVTT-in-TS subtitles on PID 0x200. The library auto-emits the
    // registration_descriptor with format_identifier "VTTC" on the
    // PMT — this is what hls.js + ffmpeg recognize as
    // WebVTT-in-MPEG-TS. Without that descriptor, the receiver would
    // see a `stream_type 0x06` PID and have no way to disambiguate it
    // from KLV (which also uses 0x06).
    //
    // WebVTT-in-TS is param-less at the codec level: the cue text and
    // timing live INSIDE the PES payload (per Apple's HLS
    // WebVTT-in-TS draft), so the codec marker is just `WebVttInTs`
    // with no struct fields — unlike `DvbSubtitling` /
    // `DvbTeletext`, which carry language + page-id metadata that
    // surfaces in the PMT's subtitling/teletext descriptor.
    let cfg = Config::builder()
        .add_program(/*program_number=*/ 1, /*pcr_pid=*/ PCR_PID)
        .add_video(VIDEO_PID, VideoCodec::H264)
        .add_subtitle(SUBTITLE_PID, SubtitleCodec::WebVttInTs)
        .end_program()
        .build()?;

    // `Muxer::new` runs `Config::validate` and returns
    // `MuxError::InvalidConfig` if anything is wrong (duplicate PIDs,
    // PSI interval below 10 ms, etc.). The `?` propagates that to
    // `main`'s `Box<dyn Error>` return.
    let mut mux = Muxer::new(cfg)?;
    let mut out = File::create(&path)?;

    // Subtitle handle for our single configured stream. With more
    // than one subtitle stream `subtitle_handles_for_program(N)`
    // gives a per-program slice — but with one, `[0]` is fine.
    // Handles are opaque tokens; the muxer maps them to the
    // configured (program_index, within_program_index) pair.
    let sub_handle = mux.subtitle_handles()[0];

    // Wall-clock POI offsets. Subtitle PTS values are 90 kHz ticks
    // since the program-clock origin; we generate a few POIs at 1 s,
    // 5 s, and 10 s. Real callers would use the same PTS axis their
    // video frames ride on — the receiver matches subtitle cues to
    // video frames via PTS.
    let pois = [
        (Duration::from_secs(1), "POI #1: passing waypoint A"),
        (Duration::from_secs(5), "POI #2: target acquired"),
        (Duration::from_secs(10), "POI #3: returning to base"),
    ];

    for (offset, text) in pois {
        // 90_000 == MPEG-TS clock rate (90 kHz). Multiply seconds by
        // 90_000 to convert to ticks. The cast to `i64` matches the
        // `pts_90khz` parameter shape on `push_subtitle_to`.
        let pts_90khz = (offset.as_secs_f64() * 90_000.0) as i64;

        // Build a self-contained WebVTT cue. Per Apple's HLS
        // WebVTT-in-TS draft, each PES carries one or more cues and
        // each cue is a complete (header + blank line + timing line +
        // payload + trailing newline) chunk — NOT the WebVTT-file
        // shape of one header followed by many cues. We emit one cue
        // per PES for simplicity; receivers can process each cue
        // independently because of `data_alignment_indicator` (set
        // automatically by the muxer on subtitle PES).
        //
        // Cue duration is offset+5s; the format follows WebVTT's
        // `HH:MM:SS.mmm --> HH:MM:SS.mmm` timestamp shape.
        let cue = format!(
            "WEBVTT\n\n{:02}:{:02}:{:02}.{:03} --> {:02}:{:02}:{:02}.{:03}\n{}\n",
            offset.as_secs() / 3600,
            (offset.as_secs() / 60) % 60,
            offset.as_secs() % 60,
            offset.subsec_millis(),
            (offset.as_secs() + 5) / 3600,
            ((offset.as_secs() + 5) / 60) % 60,
            (offset.as_secs() + 5) % 60,
            offset.subsec_millis(),
            text,
        );

        // `push_subtitle_to` wraps the cue bytes in a
        // `private_stream_1` PES (stream_id 0xBD) with a PTS-only
        // header — no DTS, no B-frame reorder. The muxer is opaque
        // to cue contents; it just packs whatever bytes the caller
        // hands it into the PES payload. The PES
        // `data_alignment_indicator` is set because each PES carries
        // one logical subtitle unit (the cue).
        mux.push_subtitle_to(sub_handle, pts_90khz, cue.as_bytes())?;
    }

    // Drain queued TS packets to the file. With no video pushed,
    // the only TS output is the PSI (PAT + PMT, auto-emitted on
    // first push) plus the three subtitle PES packets — small but
    // still a valid `.ts`.
    out.write_all(&drain_all(&mut mux))?;
    println!("wrote {} ({} POI cues)", path, pois.len());
    Ok(())
}
