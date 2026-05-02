//! H.265 + sync-KLV (ST 1910) flavor of `mux_to_file`.
//!
//! Diffs against `mux_to_file.rs` to show which Config knobs flip when
//! switching codec and KLV mode:
//!
//!   - VideoCodec::H264                     -> VideoCodec::H265
//!   - KlvStreamType::PrivateData (default) -> KlvStreamType::SynchronousMetadata
//!   - carries_pts: false (default)         -> carries_pts: true
//!
//!   cargo run --example mux_h265_with_klv -- /tmp/h265.ts

use srt_core::mpegts::mux::{Config, KlvStreamType, Muxer, StreamSpec, VideoCodec};
use std::env;
use std::fs::File;
use std::io::Write;

// PIDs are 13-bit identifiers in the TS header. The reserved well-known
// values are 0x0000 (PAT) and 0x1FFF (null padding); elementary streams
// live in 0x0010..=0x1FFE. Within that range the muxer doesn't care
// which PID a stream sits on as long as PIDs don't collide — the
// receiver discovers them from the PMT.
//
// The specific values 0x1011 (video) and 0x1031 (KLV) are the same as
// `Config::default()`'s defaults. We set them explicitly here so the
// diff against `mux_to_file.rs` highlights only the codec/KLV-mode
// knobs and not unrelated PID rearrangement.
const VIDEO_PID: u16 = 0x1011;
const KLV_PID: u16 = 0x1031;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_path = env::args().nth(1).unwrap_or_else(|| "h265.ts".into());

    // -----------------------------------------------------------------
    // Canonical "build a Config from scratch" pattern. The
    // `streams: Vec<StreamSpec>` field is multi-stream-shaped from day
    // one — v0 enforces "at most one Video and at most one Klv" via
    // `Config::validate`, but the layout is already what Path 3 needs
    // (multiple video / multiple KLV) so future expansion lands
    // additively without breaking ABI for v0 callers.
    //
    // The `..Config::default()` form preserves the PCR/PSI/buffer
    // defaults (`pcr_interval_ms: 40`, `psi_interval_ms: 100`,
    // `buffer_packets: 10_000`, `pcr_pid: None`) while letting us
    // override just the streams.
    // -----------------------------------------------------------------
    let cfg = Config {
        streams: vec![
            StreamSpec::Video {
                pid: VIDEO_PID,
                // H.265 maps to PMT stream_type 0x24 (vs. H.264's 0x1B).
                // Receivers signal which decoder to instantiate from
                // this byte — TSDuck/ffprobe will report "HEVC video"
                // in the resulting stream.
                codec: VideoCodec::H265,
            },
            StreamSpec::Klv {
                pid: KLV_PID,
                // SynchronousMetadata (PMT stream_type 0x15) is strict
                // ST 1402 sync KLV. The muxer wraps each KLV blob in
                // an ST 1910 AU cell header so receivers can recover
                // metadata-frame boundaries even though the PES
                // payload is opaque KLV bytes.
                stream_type: KlvStreamType::SynchronousMetadata,
                // SynchronousMetadata requires `carries_pts: true` —
                // the PTS is what lets a receiver align each metadata
                // record with the corresponding video frame. The
                // combo `SynchronousMetadata + carries_pts: false` is
                // rejected by `Config::validate`.
                carries_pts: true,
            },
        ],
        ..Config::default()
    };

    // `Muxer::new` runs `Config::validate` and returns
    // `MuxError::InvalidConfig` if anything is wrong (duplicate PIDs,
    // PSI interval below 10 ms, `SynchronousMetadata` with
    // `carries_pts: false`, etc.). The `?` propagates the error to
    // `main`'s `Box<dyn Error>` return.
    let mut mux = Muxer::new(cfg)?;
    let mut out = File::create(&out_path)?;

    // 30 fps live-video shape over 5 seconds wall-clock = 150 frames.
    // PTS is on the 90 kHz MPEG-TS clock, so 90_000/30 = 3000 ticks
    // per frame — `pts = i * pts_increment` walks the clock cleanly
    // at 30 fps cadence.
    let frame_rate = 30;
    let total_frames = 5 * frame_rate;
    let pts_increment = 90_000 / frame_rate as i64;

    // 1316 == `SrtTransport::DEFAULT_PAYLOAD` — also a typical SRT
    // payload size, big enough to hold exactly 7 TS packets
    // (7 * 188 = 1316). The muxer doesn't care about this size; it
    // just dictates how many TS packets `pull` returns at a time.
    // Sized here so the example's drain loop mirrors what an
    // SRT-bound caller would use.
    let mut buf = [0u8; 1316];

    for i in 0..total_frames {
        let pts = (i as i64) * pts_increment;
        // One IDR every 2 seconds (every 60th frame at 30 fps) — a
        // common GOP cadence for live encoders.
        let key = i % (frame_rate * 2) == 0;

        // Synthetic Annex-B H.265 AU. H.265's NAL header is 2 bytes
        // wide (vs. H.264's 1):
        //   byte 0: forbidden_zero(1) | nal_unit_type(6) | nuh_layer_id_high(1)
        //   byte 1: nuh_layer_id_low(5) | nuh_temporal_id_plus1(3)
        //
        // Type 19 = IDR_W_RADL (key frame); type 1 = TRAIL_N
        // (non-key reference). The `(nal_type << 1)` shift packs the
        // 6-bit type into the top bits of byte 0 with
        // forbidden_zero=0 and the high layer-id bit=0. The 0x01 in
        // byte 1 encodes nuh_layer_id=0 plus
        // nuh_temporal_id_plus1=1 (i.e. temporal_id 0).
        let nal_type: u8 = if key { 19 } else { 1 };
        let mut au = vec![0x00, 0x00, 0x00, 0x01, (nal_type << 1), 0x01];
        // Filler bytes after the NAL header. The muxer is opaque to
        // NAL contents — it just packs whatever bytes you give it
        // into PES packets. The size variation (1000..1200) mimics
        // real bitstream variability so downstream tooling sees a
        // believable shape.
        au.resize(1000 + (i as usize % 200), 0xA5);
        // One AU = one PES packet by construction. The `key` flag
        // drives the TS adaptation field's `random_access_indicator`
        // bit, which receivers use to identify seek points.
        mux.push_video(&au, pts, key)?;

        // Synthetic KLV blob — 50 random-looking bytes. Real ST 0601
        // KLV is built via `srt_core::klv::st0601` (see the
        // `klv_encode_minimal` example); for the muxer demo any
        // bytes will do.
        let klv: Vec<u8> = (0..50).map(|j| (i as u8).wrapping_add(j as u8)).collect();
        // Same PTS as the video frame. With sync KLV
        // (`SynchronousMetadata` + `carries_pts: true`) the receiver
        // can align each metadata record back to the right video
        // frame. With async KLV (the default `PrivateData` +
        // `carries_pts: false`) the receiver gets a stream of KLV
        // records but can't directly correlate each to a specific
        // video AU. The muxer additionally wraps the bytes in an
        // ST 1910 AU cell header on the way to TS packets.
        mux.push_klv(&klv, pts)?;

        // Standard pull pattern: drain after every push so muxer
        // memory stays bounded. `pull` returns 0 when there's nothing
        // more to emit right now, otherwise a multiple of 188 (one
        // or more whole TS packets that fit in `buf`).
        loop {
            let n = mux.pull(&mut buf);
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
        }
    }

    println!(
        "wrote {} H.265 frames (sync KLV, {} s) to {}",
        total_frames,
        total_frames / frame_rate,
        out_path
    );
    Ok(())
}
