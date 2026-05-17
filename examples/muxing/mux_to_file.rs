//! mux_to_file — mux a synthetic H.264 + KLV stream to a `.ts` file with no SRT.
//!
//! This example is the simplest possible muxer pipeline; useful for verifying
//! the muxer shape without bringing up a transport. The output `.ts` file is
//! playable with `ffmpeg -i out.ts -f null -` or inspectable with `tsduck`.
//!
//! See `mux_h265_with_klv.rs` for a richer multi-stream example, and
//! `pipeline_send_to_socket.rs` for the same data flowing over SRT.
//!
//! Usage: `cargo run -p tst-examples --example mux_to_file -- <out.ts> <duration_secs>`

use std::env;
use std::fs::File;
use std::io::Write;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{Muxer, MuxerConfig};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let out_path = args.get(1).cloned().unwrap_or_else(|| "out.ts".into());
    let duration_s: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);

    // `MuxerConfig::default()` opens program 1 with PMT PID 0x1000, video PID
    // 0x1011 (H.264), and KLV PID 0x1031 (PrivateData / async). The defaults
    // are picked so most callers can use them as-is — see
    // `mux_h265_with_klv.rs` for the explicit-builder pattern that overrides
    // codec/KLV-mode. PCR cadence (40 ms) and PSI cadence (100 ms) come from
    // the same defaults; both are ETSI TR 101 290 §5 conformant.
    //
    // `Muxer::new` runs `MuxerConfig::validate`. For a default config that
    // can't fail, but we still propagate via `expect` rather than `unwrap` so
    // the panic message names the source if a future default flip breaks it.
    let mut mux = Muxer::new(MuxerConfig::default()).expect("valid config");
    let mut out = File::create(&out_path)?;

    // 30 fps live-video shape. PTS is on the 90 kHz MPEG-TS clock, so
    // 90_000 / 30 = 3000 ticks per frame; `pts = i * pts_increment` walks the
    // clock cleanly at 30 fps cadence. Real encoders pass through whatever
    // PTS their upstream produced.
    let frame_rate = 30;
    let total_frames = duration_s * frame_rate;
    let pts_increment_per_frame = 90_000 / frame_rate as i64;

    // 1316 == `SrtTransport::DEFAULT_PAYLOAD` — a typical SRT payload size,
    // big enough to hold exactly 7 TS packets (7 * 188 = 1316). The muxer
    // itself doesn't care about this size; it just dictates how many TS
    // packets `pull` returns at a time. Sized here so the example's drain
    // loop mirrors what an SRT-bound caller would use.
    let mut buf = [0u8; 1316];

    for i in 0..total_frames {
        let pts = (i as i64) * pts_increment_per_frame;
        // One IDR every 2 seconds (every 60th frame at 30 fps) — a common GOP
        // cadence for live encoders. The `key` flag drives the TS adaptation
        // field's `random_access_indicator` bit, which receivers use to
        // identify seek points.
        let key = i % (frame_rate * 2) == 0;

        // Synthetic Annex-B H.264 AU. `push_video` expects Annex-B byte
        // stream (start codes 0x000001 / 0x00000001 + NAL units), not
        // length-prefixed RBSP. The muxer is opaque to NAL contents — it
        // just packs whatever bytes you give it into PES packets.
        //
        // Byte 0 of the NAL header packs:
        //   forbidden_zero(1) | nal_ref_idc(2) | nal_unit_type(5)
        // type 5 = IDR slice, type 1 = non-IDR slice. nal_ref_idc=3 (0b11)
        // for IDR (highest priority), 2 (0b10) for non-IDR reference.
        let nal_type: u8 = if key { 5 } else { 1 };
        let nri: u8 = if key { 0b11 } else { 0b10 };
        let mut au = vec![0x00, 0x00, 0x00, 0x01, (nri << 5) | nal_type];
        // Filler bytes after the NAL header. The size variation (800..1000)
        // mimics real bitstream variability so downstream tooling sees a
        // believable shape.
        au.resize(800 + (i as usize % 200), 0xA5);
        mux.push_video(&au, Pts90khz::new(pts), key)
            .expect("push_video");

        // 1 KLV blob per frame, async (PrivateData stream_type 0x06; no PTS
        // alignment guarantee). Real ST 0601 KLV is built via
        // `tst_core::klv::st0601` (see `klv_encode_minimal`); for the muxer
        // demo any bytes will do — the muxer is opaque to KLV contents.
        let klv: Vec<u8> = (0..50).map(|j| (i as u8).wrapping_add(j as u8)).collect();
        // `metadata_service_id` goes into the AU cell header per H.222.0
        // §2.12.4.2 / ST 1402.2 App. B Table 2 for SynchronousMetadata
        // streams (stream_type 0x15); silently ignored for PrivateData
        // streams (0x06) like the one configured here. The spec default
        // is 0x00.
        mux.push_klv(&klv, Pts90khz::new(pts), 0x00)
            .expect("push_klv");

        // Standard pull pattern: drain after every push so muxer memory stays
        // bounded. `pull` returns 0 when there's nothing more to emit right
        // now, otherwise a multiple of 188 (one or more whole TS packets that
        // fit in `buf`). The output is raw TS — exact same bytes a receiver
        // would feed into `Demuxer::push`.
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
