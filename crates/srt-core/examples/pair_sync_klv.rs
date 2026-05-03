//! Pair sync-KLV records with video access units by nearest PTS.
//!
//! Why this exists: by design, `mpegts::demux` does NOT pair sync-KLV
//! with video AUs — it emits them as independent stream-tagged events
//! with full timing info. The library stays out of the pairing
//! decision because real-world tolerances vary (sample-and-hold,
//! interpolation, strict timestamp match, etc.) and any built-in
//! policy would be wrong for someone. This example is the canonical
//! consumer-side recipe for the most common policy: nearest PTS within
//! a tolerance window.
//!
//! Usage: `cargo run --example pair_sync_klv -- <input.ts>`
//!
//! Two important real-world wrinkles that this example handles:
//!
//! 1. **Both `KlvSyncAuCell` AND `KlvAsync` are paired here.** The
//!    natural intuition is "sync KLV is the kind that needs pairing,"
//!    and that's how the underlying ISO 13818-1 structure presents it
//!    — `stream_type=0x15` (synchronous metadata) carries an ST 1910
//!    AU cell whose Precision Time Stamp Pack drives the pairing PTS.
//!    But many production ISR encoders emit a `stream_type=0x15` PID
//!    whose AU cell wraps an *async-shape* inner UL (no AU cell wrap
//!    on the inner KLV). The demuxer peels the outer AU cell wrap and,
//!    if the inner UL doesn't look like another sync record, surfaces
//!    the bytes as `KlvAsync` with the AU cell's PTS preserved on the
//!    parent event. That `KlvAsync` is still PTS-aligned with video and
//!    still wants pairing; matching only `KlvSyncAuCell` here would
//!    silently drop the most common shape we see in the field.
//!
//! 2. **Tolerance window matters.** Even an encoder that intends to
//!    align KLV PTS exactly with video PTS will rarely hit the same
//!    90 kHz tick — different reference clocks, different rounding.
//!    A tolerance of a few hundred milliseconds catches the intended
//!    pair while still rejecting a coincidental near-match from the
//!    *next* GOP.
//!
//! What to look for in the output: `paired=N unpaired=M`. For a
//! conformant ISR capture you'd expect every video AU paired and
//! `unpaired=0`. A few unpaired AUs at the very start are normal (the
//! KLV history hasn't filled yet); systematic `unpaired > 0` mid-stream
//! suggests a clock drift or the tolerance is set too tight.

use srt_core::mpegts::demux::{DemuxEvent, Demuxer, MetadataKind, SamplePayload};
use std::collections::VecDeque;
use std::env;
use std::fs;

// 0.3 seconds at the 90 kHz MPEG-TS clock. This is wide enough to absorb
// typical encoder timestamp drift between the video and metadata clocks
// (anywhere from microseconds to ~100ms in practice) while staying well
// inside one frame's worth of inter-AU spacing at 1 fps (the slowest KLV
// cadence we'd expect to pair frame-by-frame). Tighten this for a strict
// conformance check; widen it for sample-and-hold style consumers that
// just want the most recent KLV regardless of how stale.
const PAIRING_TOLERANCE_TICKS: i64 = 3 * 9_000;

// 32 entries of KLV history. At 30 fps video + 1 KLV/frame that's just
// over 1 second of lookback, comfortably more than the tolerance window
// above. With 1 Hz async KLV cadence (more typical for some platforms)
// it's 32 seconds of history, which covers any realistic frame-to-KLV
// reorder. Tune for your stream's KLV rate; bigger means more memory
// and slower nearest-search, smaller means a window-edge KLV record
// can age out before its paired video AU arrives.
const KLV_HISTORY_LEN: usize = 32;

fn main() {
    let path = env::args().nth(1).expect("usage: pair_sync_klv <input.ts>");
    let bytes = fs::read(&path).expect("read input");

    // Lenient demuxer — same posture as `demux_to_events`. We want every
    // recoverable record, even from imperfect captures. Lenient mode
    // never errors on PSI / PES non-conformance, but it can still
    // return `Unrecoverable` (no TS sync byte within the search
    // window) or `MalformedPes` — both fatal for offline triage.
    let mut d = Demuxer::new();
    d.feed(&bytes)
        .expect("input could not be decoded as MPEG-TS");
    // End-of-stream flush recovers the trailing AU (PES with length=0
    // is only committed when the next PES starts; without `flush` the
    // last frame is silently dropped). Run before draining events.
    d.flush();

    // Ring buffer of `(pts, payload)` for recent KLV records. We could
    // store just `pts` since this example only counts pairings, but
    // keeping the payload around makes the example trivially extensible
    // — a real consumer would feed the matched payload to
    // `klv::st0601::decode` and emit a paired (frame, telemetry) record.
    let mut klv_history: VecDeque<(i64, Vec<u8>)> = VecDeque::with_capacity(KLV_HISTORY_LEN);
    let mut paired = 0usize;
    let mut unpaired = 0usize;

    while let Some(e) = d.next_event() {
        match e {
            // Pair on either flavor — see point (1) in the module doc
            // above. Both carry MISB ST 0601 LS bytes; either can be
            // PTS-paired with video. The payload is already AU-cell-
            // unwrapped for `KlvSyncAuCell` — feed it directly to
            // `klv::st0601::decode` if you want the typed fields.
            DemuxEvent::Metadata {
                pts,
                kind: MetadataKind::KlvSyncAuCell | MetadataKind::KlvAsync,
                payload,
                ..
            } => {
                klv_history.push_back((pts, payload));
                if klv_history.len() > KLV_HISTORY_LEN {
                    klv_history.pop_front();
                }
            }
            DemuxEvent::Sample {
                pts,
                payload: SamplePayload::Video { .. },
                ..
            } => {
                // Linear scan for nearest. With 32 entries the linear
                // scan is faster than a sorted-structure binary search
                // by a wide margin (cache-friendly). For much larger
                // histories switch to a `BTreeMap<i64, Vec<u8>>` and
                // use `range(..=pts).next_back()` + `range(pts..).next()`
                // to bracket the target.
                let nearest = klv_history
                    .iter()
                    .min_by_key(|(kpts, _)| (kpts - pts).abs());
                match nearest {
                    Some((kpts, _)) if (kpts - pts).abs() <= PAIRING_TOLERANCE_TICKS => {
                        paired += 1;
                    }
                    _ => {
                        unpaired += 1;
                    }
                }
            }
            // ProgramMap, Discontinuity, NonConformant, and other
            // metadata flavors (Unknown stream types) are explicitly
            // ignored — this example is laser-focused on the pairing
            // recipe. A real pipeline would log discontinuities and
            // non-conformance for observability.
            _ => {}
        }
    }

    println!("paired={paired} unpaired={unpaired}");
}
