# MPEG-TS Muxer Guide

## Introduction

This guide covers `srt_core::mpegts::mux` — the sender-side MPEG-TS
muxer. `Muxer` takes encoded H.264 / H.265 access units in Annex-B
framing plus KLV metadata blobs, builds PES packets, fragments them
into 188-byte TS packets, and emits PAT, PMT (carrying the KLVA
registration descriptor on the KLV PID), PCR, and PTS at configured
cadences. Each `push_video` / `push_klv` call corresponds to one PES
packet — the PES boundary is the AU boundary. Output is deterministic:
a function of inputs only, with no wall-clock dependency, so the same
input sequence produces the same output bytes regardless of how the
caller paces its calls.

This is the sender-side guide. The symmetric receiver-side guide is
[guide-mpegts-demux.md](guide-mpegts-demux.md) — `mpegts::demux` ships
a Rust-native TS demuxer covering the same wire shape. Consumers can
also feed extracted bytes to FFmpeg / Bento4 / JavaCV / platform
demuxers if they prefer.

## `Config` shape

`Config` is multi-stream-shaped from day one:

```rust
pub struct Config {
    pub streams: Vec<StreamSpec>,
    pub pcr_pid: Option<u16>,
    pub pcr_interval_ms: u32,
    pub psi_interval_ms: u32,
    pub buffer_packets: usize,
}
```

Today `Config::validate` caps the configuration at "at most one Video
stream and at most one Klv stream; at least one of either" — i.e.
single-program TS with up to one video PID and one KLV PID. The
`Vec<StreamSpec>` shape is already what a multi-stream lift needs, so
the cap can lift additively without breaking ABI for existing
callers. See "Multi-stream `mpegts::mux`" in
[deferred-features.md](deferred-features.md).

`Config::default()` returns the canonical single-program shape:
H.264 video at PID `0x1011`, KLV `PrivateData` (async, no PTS) at PID
`0x1031`, PCR pinned to the video PID, `pcr_interval_ms: 40`,
`psi_interval_ms: 100`, `buffer_packets: 10_000`. Two equivalent ways
to construct from defaults plus selected overrides:

```rust,no_run
use srt_core::mpegts::mux::{Config, KlvStreamType, StreamSpec, VideoCodec};

// Pure default — H.264 + async KLV.
let cfg_default = Config::default();

// Field-update form: change just the streams, keep cadence defaults.
let cfg_h265_sync = Config {
    streams: vec![
        StreamSpec::Video {
            pid: 0x1011,
            codec: VideoCodec::H265,
        },
        StreamSpec::Klv {
            pid: 0x1031,
            stream_type: KlvStreamType::SynchronousMetadata,
            carries_pts: true,
        },
    ],
    ..Config::default()
};
```

## `ConfigBuilder`

When constructing from scratch rather than tweaking the default,
`ConfigBuilder` lets you chain stream additions and cadence overrides.
Methods (all in `srt_core::mpegts::mux`):

- `add_video(pid: u16, codec: VideoCodec) -> Self`
- `add_klv(pid: u16, stream_type: KlvStreamType, carries_pts: bool) -> Self`
- `add_stream(spec: StreamSpec) -> Self` — escape hatch when you have
  a `StreamSpec` already.
- `pcr_pid(pid: u16) -> Self`
- `pcr_interval_ms(ms: u32) -> Self`
- `psi_interval_ms(ms: u32) -> Self`
- `buffer_packets(n: usize) -> Self`
- `build() -> Result<Config, MuxError>` — runs `Config::validate`.

```rust,no_run
use srt_core::mpegts::mux::{Config, ConfigBuilder, KlvStreamType, VideoCodec};

fn build() -> Result<Config, srt_core::error::MuxError> {
    Config::builder()
        .add_video(0x1011, VideoCodec::H264)
        .add_klv(0x1031, KlvStreamType::PrivateData, false)
        .pcr_interval_ms(40)
        .psi_interval_ms(100)
        .buffer_packets(10_000)
        .build()
}
```

`Config::builder()` is a convenience for `ConfigBuilder::default()`.

## Codec selection

`VideoCodec::H264` produces a PMT entry with `stream_type = 0x1B`;
`VideoCodec::H265` produces `stream_type = 0x24`. Both are first-class.
Mid-stream codec change is out of scope — destroy the muxer and create
a new one if you need to switch codecs in a single output file.

The diff between [../crates/srt-core/examples/mux_to_file.rs](../crates/srt-core/examples/mux_to_file.rs)
(H.264 + async KLV via `Config::default()`) and
[../crates/srt-core/examples/mux_h265_with_klv.rs](../crates/srt-core/examples/mux_h265_with_klv.rs)
(H.265 + sync KLV via the field-update form) is exactly the codec and
KLV-mode knobs:

```text
- VideoCodec::H264                         + VideoCodec::H265
- KlvStreamType::PrivateData               + KlvStreamType::SynchronousMetadata
- carries_pts: false                       + carries_pts: true
```

Everything else — PIDs, cadence intervals, push/pull contract — is
identical.

## KLV-in-TS modes

Two independent axes — `KlvStreamType` (the PMT `stream_type` byte)
and `carries_pts` (whether the PES header includes a PTS) — yield
four combinations, three of them valid:

| `stream_type` | `carries_pts` | PMT byte | What it is |
| --- | --- | --- | --- |
| `PrivateData` | `false` | `0x06` | Async KLV — no PTS, broadly recognized |
| `PrivateData` | `true`  | `0x06` | Async-shaped but with PTS (uncommon, advanced) |
| `SynchronousMetadata` | `true`  | `0x15` | Sync KLV per ST 1402 — PTS required |
| `SynchronousMetadata` | `false` | (invalid) | `Config::validate` rejects |

`PrivateData` + `false` is the default and matches what most
receivers (FFmpeg, mediamtx, hls.js v1.7+) recognize out of the box —
the registration descriptor `KLVA` on the PMT entry tells the receiver
the private data is KLV. `SynchronousMetadata` + `true` is the strict
ST 1402 form for receivers that conform to it; the muxer emits the
same `KLVA` registration descriptor on the PMT entry.

**ST 1910 AU cell wrapping is caller-side, not muxer-side.**
`Muxer::push_klv` does not call `klv::st1910::wrap_au_cell` for you —
it treats the KLV payload as opaque bytes regardless of the
`stream_type` / `carries_pts` configuration. When you configure the
muxer for `SynchronousMetadata + carries_pts: true`, the conventional
pipeline is:

```text
encode_to_vec(&ls)          → inner KLV bytes
wrap_au_cell(&inner, pts)   → AU-cell-wrapped bytes
mux.push_klv(&au_cell, pts) → emits PES with the wrapped bytes
```

See [guide-klv.md](guide-klv.md)'s "ST 1910 AU cell wrap/unwrap"
section for the wrap function.

## PCR cadence

`pcr_interval_ms` defaults to `40` (validated `1..=100`). PCR is
pinned to the first video PID by default; set `Config::pcr_pid:
Option<u16>` to override (the chosen PID must equal a configured
stream's PID — `Config::validate` enforces this). A 40 ms interval
gives 25 PCR samples per second, well inside the typical receiver
expectation of "one PCR every 100 ms or better".

The cadence is push-driven, not wall-clock. The muxer compares the
PTS handed to `push_video` (or `push_klv`) against the PTS at the
last PCR emission and emits a fresh PCR when the configured interval
has elapsed in stream time. This is what makes the muxer
deterministic — the same input PTS sequence produces the same output
byte sequence — and means real-time scheduling is the caller's
responsibility, not the muxer's. Drive `push_video` at the encoder's
output cadence and PCR cadence falls out automatically.

## PSI cadence

`psi_interval_ms` defaults to `100` (validated `>= 10`). PAT and PMT
emit together at this interval — they are coupled by construction,
not separately configurable. The 100 ms default matches measured
baselines from real-world STANAG 4609 captures, where five of six
sample files in the test corpus emit PSI at roughly that cadence.

PSI is detected as due using a 33-bit modular comparison against the
last PSI emission PTS, so PTS rollover (every ~26.5 hours at 90 kHz)
does not cause PSI to suppress incorrectly. Backward PTS — common
with B-frames in display order — does not retrigger PSI either.

## Bitrate budgeting

TS overhead is dominated by per-packet header cost. A 188-byte TS
packet has 4 bytes of header (and possibly a few more for adaptation-
field stuffing), leaving 184 bytes for PES payload. A typical 1 KB
video AU therefore needs roughly:

```text
1024 bytes payload + ~14 bytes PES header = 1038 bytes
1038 / 184 = 6 TS packets (~1128 bytes wire)
overhead: (1128 - 1024) / 1024 ≈ 10–12%
```

PSI overhead at the 100 ms default is two 188-byte packets every
100 ms — about 3.8 KB/s. PCR rides on existing video TS packets'
adaptation fields and adds a negligible amount.

A useful back-of-envelope for output bitrate:

```text
output_bps ≈ input_bps * 1.05 + 4 KB/s
```

For a 4 Mbps encoded video plus low-rate KLV that lands around
4.2 Mbps wire.

## `push_video` / `push_klv` contract

```text
push_video(nal: &[u8], pts_90khz: i64, key_frame: bool) -> Result<(), MuxError>
push_klv(klv: &[u8], pts_90khz: i64)                    -> Result<(), MuxError>
```

Required:

- `nal` for `push_video` must be in Annex-B framing — either start
  code `0x000001` or `0x00000001`. Otherwise the call returns
  `MuxError::InvalidNal` and muxer state is unchanged.
- One PES packet per call — the PES boundary equals the AU boundary
  for video and the metadata-record boundary for KLV.
- `pts_90khz` is the MPEG-TS 90 kHz clock value, signed `i64`. A
  30 fps stream ticks PTS by `90_000 / 30 = 3000` per frame.
- `key_frame: true` should correspond to IDR (H.264) or
  `IDR_W_RADL` / `IDR_N_LP` (H.265). The muxer sets
  `random_access_indicator` in the adaptation field of the first TS
  packet of the resulting PES.
- `push_klv`'s `pts_90khz` is honoured only when the KLV stream was
  configured with `carries_pts: true`; otherwise it is ignored.
- KLV blobs are bounded by `PES_packet_length` (`u16`) — `65532`
  bytes without PTS, `65527` bytes with. Real ST 0601 packs are
  typically under 2 KB so this is a sanity check, not a regular
  failure.

`MuxError` variants (full list in
[../crates/srt-core/src/error.rs](../crates/srt-core/src/error.rs)):

- `InvalidConfig(&'static str)` — `Config::validate` rejected the
  configuration; the message names the failed rule.
- `InvalidNal` — `push_video` was handed a buffer without an
  Annex-B start code.
- `BufferFull { capacity_packets: usize }` — the resulting TS
  packets would exceed `Config::buffer_packets`. Drain via `pull`
  and retry. State is unchanged when this variant fires.
- `KlvTooLarge { size: usize, max: usize }` — `push_klv` blob
  exceeds the `PES_packet_length` ceiling.

## `pull` contract

```text
pull(&mut [u8]) -> usize
```

`pull` is infallible — there are no failure modes that don't already
surface at `push_video` / `push_klv` time. It returns the number of
bytes written: `0` if the queue is empty or `out.len() < 188`, otherwise
a positive multiple of 188. The caller drives a drain loop until `pull`
returns 0.

The standard pattern is push-then-drain after every push so the
muxer's internal queue stays bounded:

```rust,no_run
use srt_core::mpegts::mux::{Config, Muxer};

fn drain_pattern() -> Result<(), Box<dyn std::error::Error>> {
    let mut mux = Muxer::new(Config::default())?;
    let mut buf = [0u8; 1316]; // 7 TS packets — typical SRT payload size
    let nal = [0x00, 0x00, 0x00, 0x01, 0x65, 0x00];
    mux.push_video(&nal, 0, true)?;
    let klv = vec![0xAB; 50];
    mux.push_klv(&klv, 0)?;
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        // ship buf[..n] downstream
    }
    Ok(())
}
```

## Multi-stream shape

The muxer ships single-program TS today: `Config::validate` enforces
"at most one Video and at most one Klv". `Muxer::new` further requires
exactly one of each (a future lift relaxes this on either side). The
`Config::streams: Vec<StreamSpec>` shape is multi-stream-from-day-one
so a future multi-stream lift can land additively without breaking
ABI for existing callers — the cap is the only thing that needs to
move.

See "Multi-stream `mpegts::mux`" in [deferred-features.md](deferred-features.md)
for the trigger to revisit.

## Examples

Three runnable examples cover the muxer's surface:

- [../crates/srt-core/examples/mux_to_file.rs](../crates/srt-core/examples/mux_to_file.rs)
  — H.264 + async KLV via `Config::default()`, writes a `.ts` file.
- [../crates/srt-core/examples/mux_h265_with_klv.rs](../crates/srt-core/examples/mux_h265_with_klv.rs)
  — H.265 + sync KLV via the field-update form, illustrating the
  diff against the H.264 default.
- [../crates/srt-core/examples/pipeline_send_to_socket.rs](../crates/srt-core/examples/pipeline_send_to_socket.rs)
  — the muxer composed inside `pipeline::Sender` and connected to an
  SRT socket. See [guide-pipeline.md](guide-pipeline.md) for the
  sender-shell layer.

## What's deferred

Each item below maps to an entry in
[deferred-features.md](deferred-features.md).

- Audio carriage in `mpegts::mux` — gimbaled-platform streams are
  video + KLV today; no shipping consumer asks for audio. See
  [deferred-features.md](deferred-features.md).
- Subtitle, caption, and auxiliary-data channels — same situation as
  audio, plus the abstraction varies enough across channel types
  that a generic shape is the wrong call. See
  [deferred-features.md](deferred-features.md).
- Multi-stream `mpegts::mux` (multiple video PIDs / multiple KLV PIDs
  per output TS) — `Config` is multi-stream-shaped; the cap lifts
  additively when a consumer needs it. See
  [deferred-features.md](deferred-features.md).
