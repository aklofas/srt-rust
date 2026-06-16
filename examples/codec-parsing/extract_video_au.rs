//! Extract per-AU video binaries from an MPEG-TS file using the public
//! `tst_core::mpegts::demux::Demuxer` API.
//!
//! Usage:
//!   cargo run -p tst-examples --example extract_video_au -- <input.ts> [out_dir]
//!
//! Output:
//!   <out_dir>/au_NNNN_<pts>.bin — one Annex-B access unit per file.
//!   <out_dir>/manifest.txt      — one line per AU (idx, PTS, size, codec).
//!
//! Companion to `examples/extract_klv.rs`. Both pull per-frame data from
//! real MPEG-TS captures so integration tests and downstream tools can
//! replay against production-shaped inputs.
//!
//! The demuxer hands each video AU back as raw encoded bytes; this
//! example opt-in `split_video`s them into a `Vec<NalUnit>` (start codes
//! stripped) and re-emits the start codes so the resulting `.bin` files
//! are directly playable by an Annex-B decoder (or `ffmpeg -f h264 -i
//! au_0000_*.bin ...` / `-f hevc ...`).

use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use tst_core::mpegts::demux::{
    DemuxEvent, Demuxer, NalUnit, SamplePayload, VideoPayload, split_video,
};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: extract_video_au <input.ts> [out_dir]");
        std::process::exit(2);
    }
    let input = PathBuf::from(&args[1]);
    let out_dir = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "video_aus".into()));
    fs::create_dir_all(&out_dir)?;

    let bytes = fs::read(&input)?;

    // Lenient-default Demuxer: the same posture as `extract_klv`. Any
    // non-conformance surfaces as `NonConformant` events that this
    // example simply ignores — the goal is to extract as many AUs as
    // possible from real-world captures, even imperfect ones.
    let mut d = Demuxer::new();
    d.feed(&bytes).expect("demuxer recovered");
    // `flush` ensures the trailing AU isn't lost. Without it, any PES
    // still mid-reassembly when the file ends would be silently dropped
    // — for video that's typically the last frame. Always call `flush`
    // at end-of-stream.
    d.flush();

    // Manifest: one row per AU, columns separated by spaces. Useful for
    // downstream tooling that wants per-AU metadata (PTS, size, codec)
    // without re-parsing the TS — e.g., a script that picks just the
    // keyframe-spaced AUs for a sparse decode test.
    let mut manifest = File::create(out_dir.join("manifest.txt"))?;
    writeln!(manifest, "# idx pts size codec")?;

    let mut idx = 0usize;
    while let Some(event) = d.next_event() {
        // Match the video Sample shape. Audio / Subtitle / Unknown
        // payloads are valid `Sample` variants too — they're explicitly
        // unhandled here because this example is video-only.
        // Raw-first: the demuxer hands you the encoded access unit. Parsing it
        // into NAL units is now an OPT-IN call via `split_video` (mirrors how
        // KLV surfaces raw bytes with an opt-in decode). We split here, ignore
        // any ES-conformance `_issues`, and reconstruct Annex-B from the NALs.
        if let DemuxEvent::Sample {
            pts,
            payload:
                SamplePayload::Video {
                    codec,
                    raw,
                    av1_carriage,
                    ..
                },
            ..
        } = event
        {
            let (payload, _issues) = split_video(&raw, codec, av1_carriage.unwrap_or_default());
            let VideoPayload::Nals(nals) = payload else {
                // OBU-shaped video (AV1) — not handled by this Annex-B example.
                continue;
            };
            let pts = pts.as_ticks();
            // Reassemble Annex-B bytes from the typed NAL units. The
            // opt-in `split_video` stripped start codes during the
            // H.264/H.265 NAL split; consumers writing back to disk for an
            // Annex-B decoder need them re-emitted.
            //
            // The H.264/H.265 specs allow either a 3-byte
            // (`00 00 01`) or 4-byte (`00 00 00 01`) start code. Both
            // are spec-valid; we pick 4-byte for consistency with the
            // form the `mpegts::mux` Muxer emits and with what most
            // production encoders produce. Decoders accept either —
            // the choice doesn't affect downstream playback.
            //
            // Emulation prevention bytes (the `0x03` insertion that
            // breaks up bit patterns that look like start codes) are
            // preserved in `nal.payload` per the demuxer contract — we
            // pass them through unchanged. The decoder strips them.
            let mut buf = Vec::new();
            for nal in &nals {
                buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                match nal {
                    NalUnit::H264 {
                        nal_type,
                        ref_idc,
                        payload,
                    } => {
                        // H.264 NAL header is 1 byte (§7.3.1):
                        //   forbidden_zero_bit(1) = 0
                        //   nal_ref_idc(2)
                        //   nal_unit_type(5)
                        //
                        // The mask-and-shift below assumes
                        // forbidden_zero_bit=0 (always true for
                        // well-formed NALs); `ref_idc` and `nal_type`
                        // come pre-masked from the demuxer but we mask
                        // again defensively.
                        let header = ((ref_idc & 0x03) << 5) | (nal_type & 0x1F);
                        buf.push(header);
                        buf.extend_from_slice(payload);
                    }
                    NalUnit::H265 {
                        nal_type,
                        layer_id,
                        temporal_id_plus1,
                        payload,
                    } => {
                        // H.265 NAL header is 2 bytes (§7.3.1.2):
                        //   byte 0:
                        //     forbidden_zero_bit(1) = 0
                        //     nal_unit_type(6)
                        //     nuh_layer_id high bit(1)
                        //   byte 1:
                        //     nuh_layer_id low 5 bits(5)
                        //     nuh_temporal_id_plus1(3)
                        //
                        // `layer_id` is 6 bits total; we split it
                        // across the two bytes per the field layout.
                        let h0 = ((nal_type & 0x3F) << 1) | ((layer_id >> 5) & 0x01);
                        let h1 = ((layer_id & 0x1F) << 3) | (temporal_id_plus1 & 0x07);
                        buf.push(h0);
                        buf.push(h1);
                        buf.extend_from_slice(payload);
                    }
                    NalUnit::H266 {
                        nal_type,
                        layer_id,
                        temporal_id_plus1,
                        payload,
                    } => {
                        // H.266 NAL header is 2 bytes (V4 §7.3.1.2) but
                        // the field layout differs from H.265:
                        //   byte 0:
                        //     forbidden_zero_bit(1) = 0
                        //     nuh_reserved_zero_bit(1) = 0
                        //     nuh_layer_id(6)
                        //   byte 1:
                        //     nal_unit_type(5)
                        //     nuh_temporal_id_plus1(3)
                        //
                        // Note `nal_type` is in byte 1 (top 5 bits), not
                        // byte 0 — easy mistake when adapting from H.265.
                        let h0 = layer_id & 0x3F;
                        let h1 = ((nal_type & 0x1F) << 3) | (temporal_id_plus1 & 0x07);
                        buf.push(h0);
                        buf.push(h1);
                        buf.extend_from_slice(payload);
                    }
                }
            }

            // PTS is on the 90 kHz MPEG-TS clock. Including it in the
            // filename makes the timestamps human-readable when
            // sorting; the manifest carries the canonical record.
            let path = out_dir.join(format!("au_{idx:04}_{pts}.bin"));
            File::create(&path)?.write_all(&buf)?;
            writeln!(manifest, "{idx} {pts} {} {codec:?}", buf.len())?;
            idx += 1;
        }
    }

    println!("extracted {} access units to {}", idx, out_dir.display());
    Ok(())
}
