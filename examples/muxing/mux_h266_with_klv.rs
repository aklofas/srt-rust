//! H.266 + sync-KLV (ST 1910) flavor of `mux_to_file`.
//!
//! Diffs against `mux_to_file.rs` to show which MuxerConfig knobs flip when
//! switching codec and KLV mode:
//!
//!   - VideoCodec::H264                     -> VideoCodec::H266
//!   - KlvStreamType::PrivateData (default) -> KlvStreamType::SynchronousMetadata
//!   - carries_pts: false (default)         -> carries_pts: true
//!
//! H.266 (VVC) is carried in MPEG-TS under PMT stream_type 0x33 per
//! the ITU-T H.222.0 amendment for VVC; the muxer sets that
//! stream_type automatically when `VideoCodec::H266` is configured.
//!
//!   cargo run -p tst-examples --example mux_h266_with_klv -- /tmp/h266.ts

use std::env;
use std::fs::File;
use std::io::Write;
use tst_core::mpegts::mux::{
    KlvStreamType, Muxer, MuxerConfig, MuxerProgramConfigBuilder, VideoCodec,
};

// PIDs are 13-bit identifiers in the TS header. The reserved well-known
// values are 0x0000 (PAT) and 0x1FFF (null padding); elementary streams
// live in 0x0010..=0x1FFE. Within that range the muxer doesn't care
// which PID a stream sits on as long as PIDs don't collide — the
// receiver discovers them from the PMT.
//
// The specific values 0x1011 (video) and 0x1031 (KLV) are the same as
// `MuxerConfig::default()`'s defaults. We set them explicitly here so the
// diff against `mux_to_file.rs` highlights only the codec/KLV-mode
// knobs and not unrelated PID rearrangement.
const VIDEO_PID: u16 = 0x1011;
const KLV_PID: u16 = 0x1031;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_path = env::args().nth(1).unwrap_or_else(|| "h266.ts".into());

    // -----------------------------------------------------------------
    // Canonical "build a MuxerConfig from scratch" pattern using the
    // standalone sub-builder shape.
    //
    //   1. `MuxerProgramConfigBuilder::new(program_number, pmt_pid)`
    //      constructs a stream-attaching builder for one program.
    //   2. `add_video` / `add_klv` register elementary streams inside
    //      that program; here we configure H.266 (VVC, stream_type
    //      0x33 — set automatically by `VideoCodec::H266`) plus a
    //      sync-KLV stream sharing the program's PCR.
    //   3. `prog.build()` finalizes the program config (a plain owned
    //      value) and is then bound onto the top-level
    //      `MuxerConfig::builder()` via `add_program`.
    //   4. The top-level `b.build()?` runs `MuxerConfig::validate` and
    //      applies defaults for PCR/PSI/buffer intervals.
    //
    // The bind-then-build separation keeps each program description
    // self-contained — useful when you want to construct multiple
    // programs procedurally before assembling the final config.
    // -----------------------------------------------------------------
    let cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(VIDEO_PID, VideoCodec::H266);
        // SynchronousMetadata (PMT stream_type 0x15) is strict
        // sync KLV per MISB ST 1402.2 § 9.4.1 / ITU-T H.222.0
        // V9 § 2.12.4.2. The muxer auto-prepends a 5-byte
        // Metadata_AU_cell header (Tables 2-155+2-156:
        // metadata_service_id + sequence_number + flags +
        // AU_cell_data_length) before each push; pass raw KLV
        // LS bytes, not pre-wrapped bytes. PTS lives in the
        // PES header (per § 2.12.4.1). See
        // `docs/guide-mpegts-mux.md` for the full contract.
        //
        // SynchronousMetadata requires `carries_pts: true` —
        // the PTS is what lets a receiver align each metadata
        // record with the corresponding video frame. The
        // combo `SynchronousMetadata + carries_pts: false` is
        // rejected by `MuxerConfig::validate`.
        prog.add_klv(KLV_PID, KlvStreamType::SynchronousMetadata, true);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build()?
    };

    // `Muxer::new` runs `MuxerConfig::validate` and returns
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

        // Synthetic Annex-B H.266 AU. Per H.266 V4 §7.3.1.2, the NAL
        // header is 2 bytes wide with a different layout from H.265:
        //   byte 0: forbidden_zero(1) | nuh_reserved_zero(1) | nuh_layer_id(6)
        //   byte 1: nal_unit_type(5)  | nuh_temporal_id_plus1(3)
        //
        // i.e. nal_unit_type lives in byte 1 (top 5 bits) for H.266,
        // not byte 0 like H.265. Per H.266 V4 Table 5:
        //   IDR_W_RADL = 7  (key frame, may have leading pics)
        //   TRAIL_NUT  = 0  (regular slice in trailing position)
        //
        // VPS_NUT=14, SPS_NUT=15, PPS_NUT=16, AUD_NUT=20 — not used
        // here (the muxer is opaque to NAL contents and doesn't need
        // parameter sets to pack PES packets), but a real encoder
        // would emit them at GOP boundaries.
        let nal_type: u8 = if key { 7 } else { 0 };
        // byte 0 = 0x00 (forbidden=0, reserved=0, layer_id=0).
        // byte 1 = (nal_type << 3) | 0x01 (temporal_id_plus1=1, i.e.
        // temporal_id=0).
        let mut au = vec![0x00, 0x00, 0x00, 0x01, 0x00, (nal_type << 3) | 0x01];
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

        // Synthetic inner KLV blob — 50 random-looking bytes. Real
        // ST 0601 KLV is built via `tst_core::klv::st0601` (see the
        // `klv_encode_minimal` example); for the muxer demo any
        // bytes will do.
        let inner_klv: Vec<u8> = (0..50).map(|j| (i as u8).wrapping_add(j as u8)).collect();
        // Sync KLV (`SynchronousMetadata` + `carries_pts: true`):
        // the muxer auto-wraps in a 5-byte H.222.0 § 2.12.4.2
        // Metadata_AU_cell header, then puts the AU cell bytes in
        // a PES whose header carries the PTS we pass below. The
        // PTS is what lets the receiver align each metadata record
        // back to the right video frame. With async KLV (the default
        // `PrivateData` + `carries_pts: false`) the receiver gets a
        // stream of KLV records but can't directly correlate each
        // to a specific video AU.
        // `metadata_service_id` goes into the AU cell header per H.222.0
        // §2.12.4.2 / ST 1402.2 App. B Table 2 for SynchronousMetadata
        // streams (stream_type 0x15); it is silently ignored for PrivateData
        // streams (0x06) like the one configured above. The spec default is
        // 0x00 — use a non-zero value only when you have multiple independent
        // metadata services on the same PID (e.g. to mirror a `service_id`
        // byte in a metadata_klva() PMT descriptor supplied at config time).
        mux.push_klv(&inner_klv, pts, 0x00)?;

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
        "wrote {} H.266 frames (sync KLV, {} s) to {}",
        total_frames,
        total_frames / frame_rate,
        out_path
    );
    Ok(())
}
