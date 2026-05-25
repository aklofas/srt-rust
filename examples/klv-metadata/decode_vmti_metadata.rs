//! Decode VMTI per-target detections from a `.ts` file.
//!
//! Walks the file via `mpegts::demux::Demuxer`, filters AU-cell
//! metadata payloads (the H.222.0 §2.12.4.2 5-byte wrapper around
//! KLV LS bytes — already peeled by the demuxer, so `payload` is the
//! inner KLV LS), decodes the inner ST 0601 record, then dispatches
//! Tag 74 through `klv::st0903::decode` to surface typed VMTI fields.
//!
//! # Why two decode layers
//!
//! ST 0601 doesn't recurse into Tag 74 — the parent's `vmti` field is
//! deliberately `Option<Vec<u8>>` pass-through bytes. This sibling-
//! layer pattern keeps each typed set independent (you can ship an ST
//! 0601 consumer without VMTI awareness) and makes future spec versions
//! safe (ST 0903.7 changes don't ripple into ST 0601 parsing). The
//! companion example `decode_security_metadata.rs` demonstrates the
//! same pattern for ST 0102 / Tag 48.
//!
//! # Why lenient decode (`decode`, not `decode_strict`)
//!
//! - `decode` is the right call for production ingest: real-world
//!   captures sometimes include malformed sub-fields that fail strict
//!   but contain useful data in the well-formed parts. Per-field
//!   diagnostics accumulate in `field_errors` so the consumer can see
//!   what was lost. Unknown tags survive in `unknown` per ST 0107.5
//!   §6's future-proof skip rule (so an ST 0903.7 stream still decodes
//!   on an ST 0903.6-aware consumer with the new tags surfacing as
//!   pass-through bytes).
//! - `decode_strict` is the right call for capture pipelines that
//!   reject anything non-conformant — uncommon outside compliance
//!   work. Switch the call site below if you want strict semantics.
//!
//! # What's NOT decoded here
//!
//! Each `VTargetPack` carries up to five nested Local Sets — `VMask`,
//! `VObject` (and `VObject` series), `VFeature`, `VTracker`, `VChip`
//! (and `VChip` series). Those stay as `Option<Vec<u8>>` pass-through
//! bytes on the typed surface (see `docs/project/deferred-features.md`); the
//! `target_id` / centroid / bbox / confidence / priority subset
//! printed below is the most analyst-actionable slice and the part
//! consumers ask for first.
//!
//! Run: `cargo run -p tst-examples --example decode_vmti_metadata -- path/to/capture.ts`

use std::env;
use std::fs;
use std::process::ExitCode;

use tst_core::klv::{st0601, st0903};
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, MetadataKind};

fn main() -> ExitCode {
    let path = match env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: decode_vmti_metadata <file.ts>");
            return ExitCode::from(2);
        }
    };

    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {}: {}", path, e);
            return ExitCode::from(1);
        }
    };

    // Lenient defaults: missing PMT descriptors, stream_type drift,
    // PUSI-mid-PES, and PCR anomalies all surface as `NonConformant`
    // events but never fail the demux loop. For dev-tooling /
    // extraction we want maximum recovery — a non-conformant capture
    // should still yield as much extractable VMTI as possible.
    let mut demuxer = Demuxer::new();

    // Single-shot feed of the whole file. The demuxer accepts arbitrary
    // byte slices and recovers TS sync internally — no need to align on
    // 188-byte boundaries. A streaming consumer would call `feed`
    // repeatedly as bytes arrive, then `flush` at end-of-stream; the
    // event queue accumulates across `feed` calls.
    if let Err(e) = demuxer.feed(&bytes) {
        eprintln!("demuxer feed: {:?}", e);
        return ExitCode::from(1);
    }
    // `flush` is the canonical end-of-stream signal. Without it, any
    // PES still mid-reassembly when the file ends is silently dropped
    // — including potentially the last KLV record. Always call `flush`
    // when the input is complete.
    demuxer.flush();

    let mut frames_with_vmti = 0usize;
    let mut total_targets = 0usize;
    let mut frames_without_vmti = 0usize;
    let mut parent_decode_failures = 0usize;
    let mut vmti_decode_failures = 0usize;

    while let Some(event) = demuxer.next_event() {
        // Filter on top-level metadata events. Sync metadata is the ST
        // 0601 / VMTI carriage path on stream_type 0x15 with the AU
        // cell already peeled by the demuxer; async (`KlvAsync`) is
        // bare KLV on a private-data PID with a `KLVA` registration.
        // Both surface KLV LS bytes — accept either so VMTI from a
        // KLVA-async producer also goes through this example.
        let DemuxEvent::Metadata {
            pts, kind, payload, ..
        } = event
        else {
            continue;
        };
        let pts = pts.as_ticks();
        match kind {
            MetadataKind::KlvSyncAuCell { .. } | MetadataKind::KlvAsync => {}
            // Unknown metadata stream_type — not KLV, skip.
            MetadataKind::Unknown(_) => continue,
        }

        // Decode the parent ST 0601 LS (lenient — capture-side
        // robustness over strict rejection). Per-field errors land in
        // `record.field_errors`; we don't surface them per-frame here
        // to keep output focused on VMTI, but a debugging consumer
        // would log them.
        let uas = match st0601::decode(&payload) {
            Ok(uas) => uas,
            Err(_) => {
                parent_decode_failures += 1;
                continue;
            }
        };

        // Tag 74: VMTI Local Set. Pass-through bytes — dispatch to
        // klv::st0903 if present. `as_deref` borrows the inner
        // `Vec<u8>` as `&[u8]`; we don't need ownership.
        let Some(vmti_bytes) = uas.vmti.as_deref() else {
            frames_without_vmti += 1;
            continue;
        };
        let vmti = match st0903::decode(vmti_bytes) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("warn: VMTI decode failed at pts={pts}: {e}");
                vmti_decode_failures += 1;
                continue;
            }
        };

        frames_with_vmti += 1;
        total_targets += vmti.targets.len();

        // Frame-level summary. PTS comes from the parent event (the
        // PES PTS, in 90 kHz ticks); `precision_time_stamp` (Tag 2 in
        // VMTI) is the encoder's notion of when the detections were
        // computed (microseconds since UNIX epoch) and is typically
        // coincident with the parent ST 0601 record's Tag 2 — printed
        // alongside so analysts can correlate.
        println!(
            "pts={} vmti_pts={:?} fov=({:?}, {:?}) {}x{}px sensor={:?} targets={}",
            pts,
            vmti.precision_time_stamp,
            vmti.horizontal_fov,
            vmti.vertical_fov,
            // `frame_width` / `frame_height` are Option<u32>; format
            // with `?` debug to render `Some(N)` / `None` consistently.
            vmti.frame_width
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".into()),
            vmti.frame_height
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".into()),
            vmti.source_sensor,
            vmti.targets.len(),
        );

        for (i, t) in vmti.targets.iter().enumerate() {
            // Per-target one-liner. `centroid_pixel` is the row-major
            // pixel index; `bbox_top_left_pixel` /
            // `bbox_bottom_right_pixel` are the same packing for the
            // bounding box corners. `confidence_level` is a 0–100
            // percentage; `priority` is operator-defined ordinal.
            // Nested LSes (vmask / vtracker / vchip / vobject_series /
            // vchip_series) stay as pass-through bytes — see the
            // module-level doc comment for why.
            println!(
                "  [{}] id={} centroid_px={:?} conf={:?} priority={:?} bbox_tl={:?} bbox_br={:?}",
                i,
                t.target_id,
                t.centroid_pixel,
                t.confidence_level,
                t.priority,
                t.bbox_top_left_pixel,
                t.bbox_bottom_right_pixel,
            );
        }
    }

    // Final summary on stderr so it doesn't mix with the per-frame
    // stdout when the output is piped or grepped.
    eprintln!(
        "summary: {frames_with_vmti} frame(s) with VMTI; {frames_without_vmti} ST 0601 frame(s) without VMTI; {total_targets} target(s) total; {parent_decode_failures} parent decode failure(s); {vmti_decode_failures} VMTI decode failure(s)",
    );
    ExitCode::SUCCESS
}
