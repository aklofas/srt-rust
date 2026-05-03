# MPEG-TS Demuxer Guide

## Introduction

This guide covers `srt_core::mpegts::demux` — the receiver-side MPEG-TS
demuxer. `Demuxer` takes raw bytes off the wire (or out of a `.ts` file),
recovers TS packet alignment, parses PSI (PAT / PMT), reassembles PES
packets, splits H.264 / H.265 NAL units, peels ST 1910 AU cell wrappers
off sync KLV, and emits a typed event stream — `DemuxEvent::ProgramMap`,
`Sample`, `Metadata`, `Discontinuity`, `NonConformant`. Bytes need not
be 188-aligned; the demuxer handles sync recovery internally.

This is the symmetric pair to [guide-mpegts-mux.md](guide-mpegts-mux.md).
The muxer goes from typed inputs (NAL units + KLV blobs) to TS bytes;
the demuxer goes from TS bytes back to typed events. They share the
same vocabulary — `VideoCodec`, `KlvStreamType` ↔ `MetadataKind`, PSI
cadence — but the demuxer's contract is bigger because it has to cope
with the messy reality of real-world captures.

**Decoupled pairing.** The demuxer does **NOT** pair sync-KLV records
with video access units. Each KLV record and each video AU surfaces as
an independent stream-tagged event with full timing info; pairing
(nearest-PTS, sample-and-hold, multi-stream routing) is a consumer-domain
decision. See "Pairing is a consumer concern" below and the three
cookbook recipes (12, 13, 14) for the canonical patterns.

**Lenient by default.** Real-world ISR captures routinely omit
`metadata_descriptor`, mix sync/async stream types incorrectly, or jump
PCR. The demuxer tolerates all of these by default, surfacing them as
`DemuxEvent::NonConformant` events so the receive loop keeps running.
Opt into hard-fail behaviour per category via `StrictMode`.

## Quick example

Read a `.ts` file, feed it to a `Demuxer`, drain events:

```rust,no_run
use srt_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload};
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read("input.ts")?;
    let mut d = Demuxer::new();
    d.feed(&bytes)?;
    d.flush(); // recover trailing PES at end-of-file
    while let Some(e) = d.next_event() {
        match e {
            DemuxEvent::ProgramMap(m) => {
                println!("PMT: {} streams", m.streams.len());
            }
            DemuxEvent::Sample { stream, pts, payload, .. } => {
                if let SamplePayload::Video { nals, .. } = payload {
                    println!("video PID 0x{:04X} pts={pts} nals={}", stream.pid, nals.len());
                }
            }
            DemuxEvent::Metadata { stream, pts, payload, .. } => {
                println!("klv PID 0x{:04X} pts={pts} bytes={}", stream.pid, payload.len());
            }
            _ => {}
        }
    }
    Ok(())
}
```

Runnable: [../crates/srt-core/examples/demux_to_events.rs](../crates/srt-core/examples/demux_to_events.rs).

## Public surface

| Type / function | What it is |
| --- | --- |
| `Demuxer` | Stateful TS demuxer. `feed` bytes in, `next_event` events out, `flush` at stream end. |
| `DemuxerBuilder` | Fluent builder for the demuxer's options. |
| `DemuxerOptions` | Plain struct of options if you'd rather build a config than chain. |
| `DemuxEvent` | Top-level event enum: `ProgramMap`, `Sample`, `Metadata`, `Discontinuity`, `NonConformant`. |
| `StreamId` | `{ pid: u16, kind: StreamKind }` — identifies the source stream of every event. |
| `StreamKind` | `Video(VideoCodec)`, `Audio(AudioCodec)`, `Subtitle(SubtitleCodec)`, `KlvSync { declared_link }`, `KlvAsync`, `Unknown(u8)`. |
| `VideoCodec` | `H264`, `H265`. |
| `AudioCodec` | Reserved variant placeholder; typed audio codec values land additively. |
| `SubtitleCodec` | Reserved variant placeholder; typed subtitle codec values land additively. |
| `SamplePayload` | `Video { codec, nals }`, `Audio { codec, frames }`, `Subtitle { codec, payload }`, `Unknown { stream_type, raw }`. |
| `NalUnit` | `H264 { nal_type, ref_idc, payload }` / `H265 { nal_type, layer_id, temporal_id_plus1, payload }`. RBSP bytes; Annex-B start codes stripped. |
| `MetadataKind` | `KlvSyncAuCell` (AU cell unwrapped), `KlvAsync` (bare LS), `Unknown(u8)`. |
| `ProgramMap` | `{ program_number, pcr_pid, streams: Vec<StreamInfo>, klv_links: Vec<KlvLink> }`. |
| `StreamInfo` | `{ pid, stream_type, kind }` — one row per declared stream in the PMT. |
| `KlvLink` | `{ klv_pid, video_pid, source: LinkSource }`. |
| `LinkSource` | `Declared` (PMT `metadata_descriptor`), `Inferred` (single video + single KLV topology), `Override` (`DemuxerBuilder::link_klv`). |
| `NonConformantIssue` | `StreamTypeMismatchSyncOnAsyncPid`, `StreamTypeMismatchAsyncOnSyncPid`, `MissingMetadataDescriptor`, `PcrAnomaly { delta }`, `PsiChecksumMismatch { pid }`, `PusiMidPes`, `Other(String)`. |
| `DiscontinuityKind` | `ContinuityJump { expected, observed }`, `PesOversize { pid }`, `PesTotalOversize`, `AdaptationFieldFlag`. |
| `StrictMode` | `Off` (default), `TimingOnly`, `DescriptorsOnly`, `Full`. |
| `pts_to_duration(pts_90khz: i64) -> Duration` | Convenience: 90 kHz ticks to `std::time::Duration`. Diagnostic / test use. |

The complete enum / struct definitions live in
[../crates/srt-core/src/mpegts/demux/event.rs](../crates/srt-core/src/mpegts/demux/event.rs).

### `Demuxer` methods

```text
Demuxer::new()                                          -> Demuxer
Demuxer::with_options(options: DemuxerOptions)          -> Demuxer
Demuxer::feed(&mut self, bytes: &[u8])                  -> Result<(), DemuxError>
Demuxer::next_event(&mut self)                          -> Option<DemuxEvent>
Demuxer::flush(&mut self)                               -> ()
```

`feed` accepts arbitrary byte slices — the demuxer handles sync
recovery internally. It can return `DemuxError::Unrecoverable` (no TS
sync byte within the search window — ~6 KiB by default — which usually
means the input isn't TS at all), `DemuxError::MalformedPes` (a PES
header that doesn't validate), or `DemuxError::StrictRejection` (a
strict-mode-rejected `NonConformant` issue surfaced as a fatal error).

`next_event` is non-blocking — returns `None` when the queue is empty.
The standard pattern is `feed`-then-drain in a loop. Events accumulate
across `feed` calls, so a streaming source can call `feed` repeatedly
as bytes arrive.

`flush` emits any in-flight events from partial PES still sitting in
reassembly state. The classic case is a video PES with `PES_packet_length=0`
(unbounded length, normal for AUs > 65535 bytes) which only commits when
the next PES arrives — at end-of-file there is no next PES, so without
`flush` the trailing AU vanishes silently. `flush` is idempotent and
safe to call repeatedly. For live SRT receive, `pipeline::Receiver`
auto-flushes on `Closed` — you only call `flush` directly when feeding
finite inputs (file replay, test fixtures).

```rust,no_run
use srt_core::mpegts::demux::Demuxer;
use std::fs;

fn drain_file(path: &str) -> Result<usize, Box<dyn std::error::Error>> {
    let bytes = fs::read(path)?;
    let mut d = Demuxer::new();
    d.feed(&bytes)?;
    d.flush();
    let mut count = 0;
    while d.next_event().is_some() {
        count += 1;
    }
    Ok(count)
}
```

## Lenient vs. strict modes

`StrictMode` is the knob that turns `NonConformant` events into hard
errors. The default is `Off` (lenient). Strict mode categorises the
checks because real-world ISR streams routinely violate descriptor /
stream-type rules — a single "strict everything" mode would be unusable
on most live data.

| Mode | Rejects | Use case |
| --- | --- | --- |
| `Off` (default) | nothing | Triage, real-world capture analysis, live receivers. |
| `TimingOnly` | `PcrAnomaly`, `PusiMidPes`, `PsiChecksumMismatch` | Receivers paranoid about clock integrity but tolerant of encoder quirks. |
| `DescriptorsOnly` | `StreamTypeMismatch{Sync,Async}OnPid`, `MissingMetadataDescriptor` | Spec-compliance gating: did the encoder declare its streams correctly? |
| `Full` | every variant including future-added ones | CI-grade compliance test against a known-good encoder. |

In strict mode, the rejected `NonConformant` event is still pushed onto
the event queue *before* the error returns from `feed`. This means a
caller draining events alongside the error gets the full narrative —
`feed` returns `Err(DemuxError::StrictRejection(_))` *and* a subsequent
`next_event()` returns the structured `NonConformant { issue, .. }`.

```rust,no_run
use srt_core::mpegts::demux::{DemuxerBuilder, StrictMode};

let _d = DemuxerBuilder::new()
    .strict(StrictMode::DescriptorsOnly)
    .build();
```

## Override surface

`DemuxerBuilder` exposes four override knobs. Use them when the encoder
lies, when memory pressure matters, or when topology inference is
ambiguous.

| Method | What it does | When to reach for it |
| --- | --- | --- |
| `link_klv(klv_pid, video_pid)` | Force a `KlvLink` between two PIDs regardless of what the PMT declares. Surfaces as `LinkSource::Override` in the `klv_links` table. | The encoder doesn't emit `metadata_descriptor`, your topology has multiple video PIDs, and you know which KLV PID feeds which video. |
| `treat_as(pid, kind)` | Override the demuxer's PMT-derived `StreamKind` for one PID. | Encoder advertises wrong `stream_type`; you know the real shape of the bytes. |
| `pes_cap_per_pid(bytes)` | Maximum PES reassembly buffer per PID. Default 4 MiB. Exceeding this emits `Discontinuity::PesOversize { pid }` and drops the partial PES. | Memory-tight environments, or paranoia against runaway PES from a malformed encoder. |
| `pes_cap_total(bytes)` | Aggregate cap across all PIDs. Default 64 MiB. Exceeding this emits `Discontinuity::PesTotalOversize` and drops. | Same as above but at the workspace level. |

```rust,no_run
use srt_core::mpegts::demux::{DemuxerBuilder, StreamKind, VideoCodec};

let _d = DemuxerBuilder::new()
    .link_klv(0x1031, 0x1011)                                // klv -> video override
    .treat_as(0x1011, StreamKind::Video(VideoCodec::H265))   // PMT lied about codec
    .pes_cap_per_pid(1 << 20)                                // 1 MiB per-PID
    .pes_cap_total(8 << 20)                                  // 8 MiB total
    .build();
```

`DemuxerOptions` is the plain-struct form if you'd rather build a config
once and pass it around:

```rust,no_run
use srt_core::mpegts::demux::{Demuxer, DemuxerOptions, StreamKind, VideoCodec};
use std::collections::HashMap;

let mut overrides = HashMap::new();
overrides.insert(0x1011u16, StreamKind::Video(VideoCodec::H265));
let mut options = DemuxerOptions::default();
options.stream_kind_overrides = overrides;
let _d = Demuxer::with_options(options);
```

## Robustness behaviours

The demuxer was designed against real-world captures, not against the
spec. The behaviours below are what makes lenient mode useful.

**Missing `metadata_descriptor` on a sync KLV PID.** Many encoders
declare `stream_type=0x15` on the KLV PID without the
`metadata_descriptor` that links it to the video PID. The demuxer
infers the link when there is exactly one video PID in the PMT
(surfaces in `klv_links` as `LinkSource::Inferred`); if there are
zero or multiple video PIDs it cannot infer and emits
`NonConformantIssue::MissingMetadataDescriptor` instead.

**Wrong `stream_type` on the KLV PID.** A PID declared `0x06` (private
data) that actually carries an ST 1910 AU cell, or a PID declared
`0x15` that carries bare async KLV. The demuxer detects the actual
shape via the leading bytes (AU cell UL prefix vs. bare ST 0601 UL
prefix), classifies correctly, and emits
`NonConformantIssue::StreamTypeMismatch{Sync,Async}On*Pid`. To avoid
flooding the event stream when a stream emits the same mismatch on
every record (often thousands), the issue is coalesced to one event
per (PID, PMT version) — re-arms on PMT version bump.

**Continuity-counter jumps (CC discontinuities).** When the per-PID CC
skips a value, the demuxer emits `Discontinuity::ContinuityJump
{ expected, observed }` and continues. PES reassembly state on that PID
is preserved — the CC jump is a signal, not a teardown.

**PUSI mid-PES.** A new PUSI-set TS packet arrives before the previous
PES's `PES_packet_length` has been satisfied (or before a length-zero
PES has had a chance to terminate). The demuxer discards the partial
PES, emits `NonConformantIssue::PusiMidPes`, and starts fresh on the
new PUSI. This is the recovery shape for live captures where a few
packets dropped upstream split a PES across a gap.

**Oversize PES.** A PES grows past the per-PID or total cap. The demuxer
drops the partial bytes, emits `Discontinuity::PesOversize { pid }` (or
`PesTotalOversize`), and resumes on the next PUSI for that PID.

**Garbage prefix bytes (HUNT/VERIFY/LOCKED).** When a stream starts
mid-flight or recovers from severe loss, the bytes leading the next TS
packet boundary aren't TS-aligned. The demuxer's syncer state machine
scans for `0x47`, then verifies the candidate is real by checking
that bytes 188 ahead is also `0x47` (VERIFY), then transitions to
LOCKED and parses packets normally. If it can't find a sync byte
within `SYNC_SEARCH_WINDOW` (~6 KiB) it returns
`DemuxError::Unrecoverable` — that's the "this isn't TS at all"
signal.

**PSI checksum mismatch.** PAT or PMT section CRC fails. Lenient mode
falls back to the previous PSI version (so the demuxer keeps its known-
good topology) and emits `NonConformantIssue::PsiChecksumMismatch`.
Strict (`TimingOnly` or `Full`) converts to `StrictRejection`.

**PCR jumps.** A PCR-bearing packet's clock jumps more than one second
relative to the previous. The demuxer emits
`NonConformantIssue::PcrAnomaly { delta }` (delta in 27 MHz ticks)
and continues. Stream-monotonic backward PTS more than one second is
also flagged as `PcrAnomaly` (delta carries the negative diff).

## What gets parsed vs. passed through

Some payloads the demuxer fully types; others it surfaces verbatim so
consumers can apply their own decoders.

**Video (H.264 / H.265).** The PES payload is split into NAL units. The
demuxer strips Annex-B start codes (`0x000001` / `0x00000001`),
preserves emulation-prevention bytes (the consumer's H.264 / H.265
decoder removes them), and returns each NAL with codec-tagged headers
on `NalUnit::H264` / `NalUnit::H265`. The full AU is one
`SamplePayload::Video { codec, nals: Vec<NalUnit> }`. Callers re-emitting
to a downstream Annex-B sink prepend `0x00 0x00 0x00 0x01` between
NALs themselves — see [../crates/srt-core/examples/extract_video_au.rs](../crates/srt-core/examples/extract_video_au.rs).

**Sync KLV (`stream_type=0x15`).** The demuxer detects the ST 1910
AU cell shape (UL prefix `06 0E 2B 34 02 0B 01 01 0E 01 03 01 01 00 00 00`),
unwraps the AU cell, and emits `MetadataKind::KlvSyncAuCell`. The
event's `pts` is the AU cell's metadata access-unit timestamp from
the embedded `klv::st0605::PrecisionTimeStampPack`, not the PES PTS.
The `payload` is the inner KLV LS bytes — feed directly to
`klv::st0601::decode`.

**Async KLV (`stream_type=0x06` + `KLVA` registration descriptor).**
The PES payload is bare KLV LS bytes. `MetadataKind::KlvAsync`. The
`pts` is the raw PES PTS (or zero if the PES carried no PTS).

**Real-world wrinkle: AU-cell wrap-peeling.** Some production ISR
encoders emit `stream_type=0x15` whose AU cell wraps an inner UL that
is *not* itself a sync record (no second AU cell wrap on the inner
KLV). The demuxer peels the outer AU cell, sees the inner UL doesn't
look like another sync record, and surfaces the bytes as
`KlvAsync` with the AU cell's PTS preserved on the parent event. This
is why pairing recipes (cookbook §12) match BOTH `KlvSyncAuCell` AND
`KlvAsync` for sync-style consumers — many real captures present as
the latter after wrap-peeling.

**Unknown stream types.** PIDs with `stream_type` not in the
`{0x1B, 0x24, 0x06+KLVA, 0x15}` set surface as
`SamplePayload::Unknown { stream_type, raw }`. The PES payload is
preserved verbatim. Audio (`stream_type=0x0F`/AAC, `0x03`/MP1, etc.),
subtitles, and AV1 / H.266 all fall through here today; typed variants
on `AudioCodec` / `SubtitleCodec` / `VideoCodec` land additively when
a consumer asks. See [deferred-features.md](deferred-features.md).

## Pairing is a consumer concern

The demuxer surfaces every video AU and every KLV record as an
independent event with full timing. It does **not** pair sync KLV with
video AUs. This is a deliberate decision — pairing tolerance, sample-
and-hold semantics, and multi-stream routing are domain choices the
library can't make correctly for everyone.

The three canonical pairing patterns are documented as cookbook recipes
with runnable examples:

- **[Recipe 12](cookbook.md): Pair sync-KLV with video AUs by nearest PTS.**
  The "frame and metadata are the same wall-clock event" workflow.
  Match on both `MetadataKind::KlvSyncAuCell` AND `MetadataKind::KlvAsync`
  (because of the AU-cell wrap-peeling case described above). Tolerance
  window is consumer domain knowledge.
- **[Recipe 13](cookbook.md): Sample-and-hold async-KLV against video frames.**
  KLV at 1–10 Hz, video at 25–60 fps. Each frame uses the most recent
  KLV record where `klv.pts <= frame.pts`. Optional staleness drop.
- **[Recipe 14](cookbook.md): EO + IR sensor pair with shared async-KLV.**
  Two video PIDs, one metadata PID; both videos attach the same KLV
  state, no per-stream pairing logic.

A potential `pipeline::pairing` opt-in helper module is captured in
[deferred-features.md](deferred-features.md) — not part of this ship.

## Common pitfalls

**Forgetting to drain `next_event` after `feed`.** `feed` accumulates
events into an internal queue but does not block on draining. If you
only call `feed` repeatedly and never call `next_event`, the queue
grows unbounded. The pattern is `feed`-then-drain.

**Forgetting `flush()` on finite inputs.** Video PES with
`PES_packet_length=0` only commits when the next PES arrives. At
end-of-file there is no next PES, so the trailing AU sits in the
reassembler. Call `flush()` once you know no more bytes are coming
(file replay, test fixture, end of `cargo run`'s `main`).

**`flush()` not needed for live SRT receive.** `pipeline::Receiver`
auto-flushes on `TransportError::Closed`. You only call `flush()`
yourself when feeding the demuxer directly.

**Assuming `SamplePayload::Video.nals` is Annex-B framed.** It isn't.
Each `NalUnit::H264` / `NalUnit::H265` carries the RBSP bytes only —
Annex-B start codes have been stripped. Re-emit start codes between
NALs yourself if writing back to an Annex-B sink. Pattern shown in
[../crates/srt-core/examples/extract_video_au.rs](../crates/srt-core/examples/extract_video_au.rs).

**Treating `Closed` as an error.** It isn't. `pipeline::Receiver` turns
`TransportError::Closed` into iterator termination — the `for` loop
simply ends after `Demuxer::flush` runs. `Broken` is the peer-disconnect
error variant; `Closed` is clean EOF. See `srt_recv_typed.rs`'s
"stream-end contract" doc-comment for the full discussion.

**Matching only `KlvSyncAuCell` for sync pairing.** Production ISR
captures often surface sync KLV as `KlvAsync` after the demuxer's
AU-cell wrap-peeling pass — the AU cell's PTS is preserved on the
event, so the bytes are still PTS-aligned with video. Cookbook
Recipe 12 matches both. Matching only `KlvSyncAuCell` silently drops
the most common shape we see in the field.

**Reaching for strict mode by default.** Strict mode is not the
"correct" mode — it's the compliance mode. Real-world captures
violate descriptor and stream-type rules routinely. Use lenient by
default; reach for strict only for a CI gate against a known-good
encoder, or for triage of a specific non-conformance category.

## Examples

Four runnable examples cover the demuxer's surface:

- [../crates/srt-core/examples/demux_to_events.rs](../crates/srt-core/examples/demux_to_events.rs)
  — file in, full event stream out. Triage-grade diagnostic.
- [../crates/srt-core/examples/srt_recv_typed.rs](../crates/srt-core/examples/srt_recv_typed.rs)
  — bind a listener, wrap with `pipeline::Receiver`, drain typed events
  from a live SRT peer.
- [../crates/srt-core/examples/pair_sync_klv.rs](../crates/srt-core/examples/pair_sync_klv.rs)
  — nearest-PTS pairing of KLV records with video AUs (Cookbook §12).
- [../crates/srt-core/examples/tee_disk_and_demux.rs](../crates/srt-core/examples/tee_disk_and_demux.rs)
  — `add_byte_sink` fan-out: write `.ts` to disk while consuming typed
  events, all in one pass.

Two existing examples were also retrofitted to use `Demuxer` internally:

- [../crates/srt-core/examples/extract_klv.rs](../crates/srt-core/examples/extract_klv.rs)
  — extract KLV records from a `.ts` capture (now `Demuxer`-driven).
- [../crates/srt-core/examples/extract_video_au.rs](../crates/srt-core/examples/extract_video_au.rs)
  — extract video access units, re-emit Annex-B framing.

## What's deferred

Each item below maps to an entry in
[deferred-features.md](deferred-features.md).

- **Typed SPS / VPS / PPS payload parser** — SPS/VPS/PPS surface as
  ordinary `NalUnit` with raw RBSP. Consumers wanting frame
  width/height/profile use an external codec library (`h264-reader`,
  `h265-parser`).
- **`pipeline::pairing` opt-in helper** — pairing stays consumer-side
  via cookbook recipes; library-level helper is deferred.
- **Multi-program TS** — single PMT only today. `ProgramMap` carries
  `program_number` so multi-program lifts additively.
- **AV1 / H.266 codec variants** — surface as `SamplePayload::Unknown`
  today. AV1 is OBU-shaped (not NAL-shaped), so adding it requires a
  cross-codec rework of `SamplePayload::Video`.
- **Typed audio + subtitle codecs** — `AudioCodec` / `SubtitleCodec`
  enums exist as reserved variants; adding e.g. `AudioCodec::Aac` is
  additive, not breaking.

See [compatibility.md](compatibility.md)'s `mpegts::demux` block for
the full feature-by-feature status.
