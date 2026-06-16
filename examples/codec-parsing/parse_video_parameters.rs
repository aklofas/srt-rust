//! Demux a TS file from disk and print typed video parameters as the
//! demuxer surfaces SPS/PPS NALs.
//!
//! Why this exists: the demuxer only emits raw NAL bytes. Consumers that
//! need typed fields (resolution, profile, level, color, frame rate, etc.)
//! at the JNI / UniFFI / C / Python boundary call into the
//! `tst_core::codec::*` parsers explicitly. This is the reference pattern
//! — demux the stream to get NALs, then parse those NALs with the
//! appropriate codec module.
//!
//! Usage:
//!     cargo run -p tst-examples --example parse_video_parameters -- <file.ts>
//!
//! The example reads in 64 KiB chunks, matching typical SRT payload sizing.
//! The demuxer doesn't care about chunk boundaries — it re-syncs on TS
//! packet boundaries internally — so any power-of-two buffer size works here.
//!
//! Per-PID state is tracked and a summary line is printed only when the
//! parsed snapshot CHANGES. A typical IDR-every-2-seconds stream produces
//! one line at startup plus additional lines only when the encoder
//! reconfigures (resolution change, codec re-init, session restart, etc.)
//! — exactly the use case that typed parsing is designed to support.
//!
//! Contrast with `demux_to_events.rs`, which prints every event for triage.
//! Use this example when you want structured metadata, not raw event dumps.

use std::collections::HashMap;
use std::env;
use std::io::Read;
use std::process::ExitCode;

use tst_core::codec::{av1, h264, h265, h266};
use tst_core::mpegts::demux::{
    DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload, split_video,
};

fn main() -> ExitCode {
    let path = match env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: parse_video_parameters <file.ts>");
            return ExitCode::from(2);
        }
    };

    let mut file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("open {path}: {e}");
            return ExitCode::from(1);
        }
    };

    // Per-PID snapshot cache. Kept as strings so change detection is a simple
    // equality check without reaching into typed structs. The string is also
    // exactly what gets printed — one allocation serves both purposes.
    let mut last_summary: HashMap<u16, String> = HashMap::new();
    let mut dx = Demuxer::new();

    // 64 KiB read buffer. The demuxer accumulates TS packets internally, so
    // partial packets at the end of a read call are reassembled on the next
    // call. No alignment or packet-boundary requirement on the caller.
    let mut buf = vec![0u8; 64 * 1024];

    loop {
        let n = match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                eprintln!("read: {e}");
                return ExitCode::from(1);
            }
        };

        if let Err(e) = dx.feed(&buf[..n]) {
            // In lenient mode (the default) this only fires for completely
            // non-TS input — there is no 0x47 sync byte in the entire search
            // window. PSI / PES non-conformance is surfaced as NonConformant
            // events, not errors. We treat this as fatal for an offline tool.
            eprintln!("feed: {e:?}");
            return ExitCode::from(1);
        }

        // Drain events between read calls so the demuxer's internal queue
        // stays bounded on long files. The event queue is unbounded by default
        // — for a 1-hour recording at 30 fps this matters.
        drain_and_print(&mut dx, &mut last_summary);
    }

    // flush commits any trailing PES to the event queue. Video PES packets
    // with PES_packet_length=0 (the standard shape for video AUs that exceed
    // 65535 bytes — i.e. nearly all real video) are only committed when the
    // NEXT PES starts. At end-of-file there is no next PES, so the last AU
    // sits in the reassembler until flush() is called. Without this, the
    // last frame of every clip disappears silently.
    dx.flush();
    drain_and_print(&mut dx, &mut last_summary);

    if last_summary.is_empty() {
        eprintln!(
            "no video streams found — check that the file is MPEG-TS with H.264, H.265, H.266, or AV1 video"
        );
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}

fn drain_and_print(dx: &mut Demuxer, last: &mut HashMap<u16, String>) {
    while let Some(ev) = dx.next_event() {
        // We only care about video samples here. All other event types
        // (ProgramMap, Metadata, Discontinuity, NonConformant) are silently
        // skipped — run demux_to_events.rs for the full annotated event dump.
        let DemuxEvent::Sample {
            stream,
            payload:
                SamplePayload::Video {
                    codec,
                    raw,
                    av1_carriage,
                    ..
                },
            ..
        } = ev
        else {
            continue;
        };

        // Raw-first: the demuxer hands you the encoded access unit. Parsing it
        // into NAL/OBU units is now an OPT-IN call via `split_video` (mirrors
        // how KLV surfaces raw bytes with an opt-in decode). Pass `av1_carriage`
        // from the sample so AV1 interop streams parse correctly; H.26x ignores
        // it. We drop any ES-conformance `_issues`.
        let (payload, _issues) = split_video(&raw, codec, av1_carriage.unwrap_or_default());

        // Two payload shapes ride VideoPayload:
        //   * Nals(Vec<NalUnit>) for H.264 / H.265 / H.266 — NAL-framed codecs
        //     using Annex-B start-code delimiting in the elementary stream.
        //   * Obus(Vec<Obu>) for AV1 — OBU-framed using the AV1 low-overhead
        //     bitstream format inside Length-Delimited OBUs in PES.
        //
        // The demuxer guarantees the (codec, payload) pairing — H.264/H.265/
        // H.266 always arrive as Nals, AV1 always as Obus. The exhaustive
        // match below crosses both axes so any future codec addition forces
        // a compile-time decision; the "impossible" arms return None rather
        // than panicking, so a hypothetical demuxer bug degrades to silent
        // skip rather than a crash in a long-running offline tool.
        //
        // parse_parameter_sets (NAL-shaped codecs) is partial-success-tolerant:
        // individual bad NALs emit a tracing::warn and are skipped. The
        // function returns Err only if EVERY parameter-set NAL in the input
        // failed to parse. Non-parameter-set NALs (P-frame slices, IDR slice
        // data) are always silently skipped, so calling this on a P-frame
        // returns Ok with empty maps — not an error.
        let summary = match payload {
            VideoPayload::Nals(nals) => match codec {
                VideoCodec::H264 => match h264::parse_parameter_sets(&nals) {
                    Ok(ps) => ps.sps_by_id.values().next().map(|sps| {
                        // H.264 level_idc is x10 — level 4.0 is stored as 40,
                        // level 5.1 is stored as 51, etc. Reporting verbatim is
                        // correct; consumers that want "4.0" format it themselves.
                        let color = sps.color.as_ref().map_or_else(
                            || "color=unspecified".to_string(),
                            |c| format!("primaries={:?} transfer={:?}", c.primaries, c.transfer),
                        );
                        let fps = sps.frame_rate.map_or_else(
                            || "fps=unknown".to_string(),
                            |r| {
                                // num/den from the SPS VUI timing_info. Common values:
                                // 60000/1001 ≈ 59.94, 30000/1001 ≈ 29.97, 25/1 = 25.
                                format!("fps={}/{}", r.num, r.den)
                            },
                        );
                        format!(
                            "H.264 {}x{} profile={} level={} {}-bit {:?} {fps} {color}",
                            sps.width,
                            sps.height,
                            sps.profile_idc,
                            sps.level_idc,
                            sps.bit_depth_luma,
                            sps.chroma_format,
                        )
                    }),
                    Err(e) => Some(format!("H.264 parse error: {e}")),
                },
                VideoCodec::H265 => match h265::parse_parameter_sets(&nals) {
                    Ok(ps) => ps.sps_by_id.values().next().map(|sps| {
                        // H.265 general_level_idc is x30 — level 4.0 is stored as 120,
                        // level 5.1 is stored as 153. Same verbatim-report convention
                        // as H.264 above.
                        let color = sps.color.as_ref().map_or_else(
                            || "color=unspecified".to_string(),
                            |c| format!("primaries={:?} transfer={:?}", c.primaries, c.transfer),
                        );
                        let fps = sps.frame_rate.map_or_else(
                            || "fps=unknown".to_string(),
                            |r| format!("fps={}/{}", r.num, r.den),
                        );
                        format!(
                            "H.265 {}x{} profile_idc={} level_idc={} {}-bit {:?} {fps} {color}",
                            sps.width,
                            sps.height,
                            sps.general_profile_idc,
                            sps.general_level_idc,
                            sps.bit_depth_luma,
                            sps.chroma_format,
                        )
                    }),
                    Err(e) => Some(format!("H.265 parse error: {e}")),
                },
                VideoCodec::H266 => match h266::parse_parameter_sets(&nals) {
                    // H266ParameterSets exposes spses as a Vec<H266Sps> (already
                    // BTreeMap-deduped + collected by id internally), so `.first()`
                    // is the right access — no `.values()` indirection like H.264/265.
                    // general_level_idc is per ITU-T H.266 Annex A.4: stored as
                    // a plain u8 (e.g. level 4.0 → 64, level 6.0 → 96). Reporting
                    // verbatim, same convention as H.264/H.265.
                    Ok(sets) => sets.spses.first().map(|sps| {
                        let color = sps.color_info.as_ref().map_or_else(
                            || "color=unspecified".to_string(),
                            |c| format!("primaries={:?} transfer={:?}", c.primaries, c.transfer),
                        );
                        let fps = sps.frame_rate.as_ref().map_or_else(
                            || "fps=unknown".to_string(),
                            |r| format!("fps={}/{}", r.num, r.den),
                        );
                        format!(
                            "H.266 {}x{} profile={} level={} {}-bit {:?} {fps} {color}",
                            sps.width,
                            sps.height,
                            sps.profile_tier_level.general_profile_idc,
                            sps.profile_tier_level.general_level_idc,
                            sps.bit_depth_luma,
                            sps.chroma_format,
                        )
                    }),
                    Err(e) => Some(format!("H.266 parse error: {e}")),
                },
                // The demuxer guarantees AV1 ⇒ Obus, never Nals. If we ever
                // see this combination, that's a demuxer bug; degrade to silent
                // skip rather than crash.
                VideoCodec::Av1 => None,
            },
            VideoPayload::Obus(obus) => match codec {
                VideoCodec::Av1 => {
                    // parse_obu_stream is infallible — bad OBUs are bucketed
                    // into stream.unparseable rather than failing the whole
                    // call. AV1 level is uvlc-coded inside the OBU per
                    // AV1 spec §5.5.4 (operating_parameters_info); the parser
                    // already decoded it to the plain u8 we report here
                    // (e.g. seq.level=8 = level 4.0, =13 = level 5.1).
                    let stream = av1::parse_obu_stream(&obus);
                    if let Some(seq) = stream.sequence_headers.first() {
                        let color = seq.color_info.as_ref().map_or_else(
                            || "color=unspecified".to_string(),
                            |c| format!("primaries={:?} transfer={:?}", c.primaries, c.transfer),
                        );
                        let fps = seq.frame_rate.as_ref().map_or_else(
                            || "fps=unknown".to_string(),
                            |r| format!("fps={}/{}", r.num, r.den),
                        );
                        Some(format!(
                            "AV1 {}x{} profile={} level={} tier={} {}-bit {:?} {fps} {color}",
                            seq.max_frame_width,
                            seq.max_frame_height,
                            seq.profile,
                            seq.level,
                            seq.tier,
                            seq.bit_depth,
                            seq.chroma_format,
                        ))
                    } else if !stream.unparseable.is_empty() {
                        // Sequence Header didn't parse cleanly — surface the
                        // count so a consumer staring at silent output knows
                        // there IS AV1 OBU traffic, just nothing typed yet.
                        Some(format!(
                            "AV1 {} OBU(s), {} unparseable",
                            obus.len(),
                            stream.unparseable.len()
                        ))
                    } else {
                        // No SH yet (just TD / Frame / Tile OBUs in this PES).
                        // Silent until the next keyframe re-emits the SH —
                        // change-driven logging upstream avoids per-frame spam.
                        None
                    }
                }
                // The demuxer guarantees H.264/H.265/H.266 ⇒ Nals, never Obus.
                // Same degrade-to-silent-skip rationale as the AV1-on-Nals arm
                // above.
                VideoCodec::H264 | VideoCodec::H265 | VideoCodec::H266 => None,
            },
        };

        if let Some(s) = summary {
            let pid = stream.pid;
            // Only log on change. Typical streams emit SPS+PPS on every IDR
            // (every 1–4 seconds for a surveillance encoder, every 0.5s for a
            // conferencing encoder). Printing on every IDR would flood the
            // terminal for a 30-minute file. Change-driven logging is the
            // right default for a tool that operators run interactively; apps
            // polling for resolution changes use the same pattern — compare
            // the current parse result against the previous one and act only
            // on a difference.
            if last.get(&pid) != Some(&s) {
                println!("[PID 0x{pid:04X}] {s}");
                last.insert(pid, s);
            }
        }
    }
}
