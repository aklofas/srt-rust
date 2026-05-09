//! End-to-end example: synthetic AUs + KLV → `.ts` file.
//!
//! Demonstrates the canonical sender pattern with `mpegts::mux::Muxer`.
//! Real callers swap the synthetic generator for an encoder and the file
//! writer for `srt::Socket::send`.
//!
//! Usage:
//!   cargo run --example mux_to_file -- <output.ts> [duration_seconds]

use std::env;
use std::fs::File;
use std::io::Write;
use tst_core::mpegts::mux::{MuxerConfig, Muxer};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let out_path = args.get(1).cloned().unwrap_or_else(|| "out.ts".into());
    let duration_s: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);

    let mut mux = Muxer::new(MuxerConfig::default()).expect("valid config");
    let mut out = File::create(&out_path)?;

    let frame_rate = 30;
    let total_frames = duration_s * frame_rate;
    let pts_increment_per_frame = 90_000 / frame_rate as i64; // 90 kHz
    let mut buf = [0u8; 1316];

    for i in 0..total_frames {
        let pts = (i as i64) * pts_increment_per_frame;
        let key = i % (frame_rate * 2) == 0; // IDR every 2 seconds

        // Synthetic Annex-B H.264 AU: start code + NAL header + filler bytes.
        let nal_type: u8 = if key { 5 } else { 1 };
        let nri: u8 = if key { 0b11 } else { 0b10 };
        let mut au = vec![0x00, 0x00, 0x00, 0x01, (nri << 5) | nal_type];
        au.resize(800 + (i as usize % 200), 0xA5);
        mux.push_video(&au, pts, key).expect("push_video");

        // 1 KLV blob per frame, async (no PTS).
        let klv: Vec<u8> = (0..50).map(|j| (i as u8).wrapping_add(j as u8)).collect();
        // `metadata_service_id` goes into the AU cell header per H.222.0
        // §2.12.4.2 / ST 1402.2 App. B Table 2 for SynchronousMetadata
        // streams (stream_type 0x15); silently ignored for PrivateData
        // streams (0x06) like the one configured here. The spec default
        // is 0x00.
        mux.push_klv(&klv, pts, 0x00).expect("push_klv");

        // Drain into the file.
        loop {
            let n = mux.pull(&mut buf);
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
        }
    }

    println!(
        "wrote {} frames ({} s) to {}",
        total_frames, duration_s, out_path
    );
    Ok(())
}
