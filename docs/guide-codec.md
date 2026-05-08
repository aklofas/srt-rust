# Codec Parameter Set Guide

## What this module is

`tst_srt::codec` provides typed payload parsers for codec parameter sets —
stateless functions that operate on raw NAL unit bytes from the demuxer.

When `mpegts::demux` surfaces a `DemuxEvent::Sample`, the NAL units within
it are raw RBSP bytes with the TS framing and PES reassembly stripped. The
demuxer does not parse the NAL content further. Consumers that need typed
fields — width, height, profile, level, color space, frame rate — call into
the `codec::h264` or `codec::h265` parsers explicitly.

This design keeps the demuxer surface minimal and dependency-free. You only
pay for codec parsing when you need it, and the codec parsers have no
coupling to the transport or container layers.

## Architecture overview

```
mpegts::demux::Demuxer
  ↓ DemuxEvent::Sample { payload: SamplePayload::Video { codec, payload: VideoPayload::Nals(nals) }, .. }
  ↓ nals: Vec<NalUnit>   — raw RBSP bytes; NAL type in the header

tst_core::codec::h264
  parse_sps(rbsp)         → Result<H264Sps, ParseError>
  parse_pps(rbsp)         → Result<H264Pps, ParseError>
  parse_parameter_sets(nals) → Result<H264ParameterSets, ParseError>

tst_core::codec::h265
  parse_vps(rbsp)         → Result<H265Vps, ParseError>
  parse_sps(rbsp)         → Result<H265Sps, ParseError>
  parse_pps(rbsp)         → Result<H265Pps, ParseError>
  parse_parameter_sets(nals) → Result<H265ParameterSets, ParseError>
```

The demuxer event surface is unchanged — `DemuxEvent` is the same regardless
of whether you intend to parse parameter sets. Consumers call the parsers
explicitly when they need typed fields.

## H.264 quick start

```rust,no_run
use tst_core::codec::h264;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload};

let mut dx = Demuxer::new();
// ... feed bytes ...

while let Some(ev) = dx.next_event() {
    if let DemuxEvent::Sample {
        payload: SamplePayload::Video { codec: VideoCodec::H264, payload: VideoPayload::Nals(ref nals) },
        ..
    } = ev
    {
        // parse_parameter_sets is partial-success-tolerant: bad NALs emit
        // tracing::warn! and are skipped. Returns Err only if every
        // parameter-set NAL failed. Non-SPS/PPS NALs are silently skipped,
        // so calling this on a P-frame returns Ok with empty maps.
        if let Ok(ps) = h264::parse_parameter_sets(nals) {
            if let Some(sps) = ps.sps_by_id.values().next() {
                println!(
                    "H.264 {}x{} profile={} level={} {}-bit {:?}",
                    sps.width,
                    sps.height,
                    sps.profile_idc,
                    sps.level_idc,   // x10 — level 4.0 is stored as 40
                    sps.bit_depth_luma,
                    sps.chroma_format,
                );
                if let Some(c) = &sps.color {
                    println!("  primaries={:?} transfer={:?}", c.primaries, c.transfer);
                }
                if let Some(fps) = sps.frame_rate {
                    println!("  fps={}/{}", fps.num, fps.den);
                }
            }
        }
    }
}
```

**`H264Sps` key fields:**

| Field | Type | Notes |
| --- | --- | --- |
| `width` / `height` | `u32` | Decoded from `pic_width_in_mbs_minus1` / `pic_height_in_map_units_minus1` + VUI crop rectangle. |
| `profile_idc` | `u8` | 66 = Baseline, 77 = Main, 100 = High, 110 = High 10, etc. |
| `level_idc` | `u8` | `level × 10` — level 4.0 is stored as 40. |
| `bit_depth_luma` / `bit_depth_chroma` | `u8` | 8 for most streams; 10 for HDR. |
| `chroma_format` | `ChromaFormat` | `C420` / `C422` / `C444` / `Monochrome`. |
| `color` | `Option<ColorInfo>` | VUI color information (`primaries`, `transfer`, `matrix`). `None` when VUI timing is absent. |
| `frame_rate` | `Option<Rational>` | Derived from VUI `timing_info` (`num_units_in_tick` + `time_scale`). `None` when absent. |
| `raw_rbsp` | `Vec<u8>` | The input bytes verbatim (emulation-prevention bytes included). |

**`H264Pps` key fields:** `pps_id`, `sps_id`, `entropy_coding_mode` (`Cavlc` or `Cabac`), `raw_rbsp`.

## H.265 quick start

```rust,no_run
use tst_core::codec::h265;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload};

let mut dx = Demuxer::new();
// ... feed bytes ...

while let Some(ev) = dx.next_event() {
    if let DemuxEvent::Sample {
        payload: SamplePayload::Video { codec: VideoCodec::H265, payload: VideoPayload::Nals(ref nals) },
        ..
    } = ev
    {
        if let Ok(ps) = h265::parse_parameter_sets(nals) {
            if let Some(sps) = ps.sps_by_id.values().next() {
                println!(
                    "H.265 {}x{} profile_idc={} level_idc={} {}-bit {:?}",
                    sps.width,
                    sps.height,
                    sps.general_profile_idc,
                    sps.general_level_idc,   // x30 — level 4.0 is stored as 120
                    sps.bit_depth_luma,
                    sps.chroma_format,
                );
            }
        }
    }
}
```

**`H265Vps` key fields:** `vps_video_parameter_set_id`, `general_profile_idc`,
`general_tier_flag`, `general_level_idc`.

**`H265Sps` key fields:** same width / height / bit_depth / chroma_format /
color / frame_rate shape as `H264Sps`, plus `general_profile_idc`,
`general_tier_flag`, `general_level_idc`, `max_sub_layers_minus1`.
`general_level_idc` is `level × 30` — level 4.0 is stored as 120.

**`H265Pps` key fields:** `pps_pic_parameter_set_id`, `pps_seq_parameter_set_id`.

## H.266 / VVC quick start

H.266 (Versatile Video Coding) parses identically in shape to H.265 — VPS, SPS,
PPS — with a different bitstream syntax. PMT `stream_type = 0x33`.

```rust,no_run
use tst_core::codec::h266;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload};

let mut dx = Demuxer::new();
// ... feed bytes ...

while let Some(ev) = dx.next_event() {
    if let DemuxEvent::Sample {
        payload: SamplePayload::Video { codec: VideoCodec::H266, payload: VideoPayload::Nals(ref nals) },
        ..
    } = ev
    {
        if let Ok(ps) = h266::parse_parameter_sets(nals) {
            if let Some(sps) = ps.spses.first() {
                println!(
                    "H.266 {}x{} profile_idc={} tier={} level_idc={} {}-bit {:?}",
                    sps.width,
                    sps.height,
                    sps.profile_tier_level.general_profile_idc,
                    sps.profile_tier_level.general_tier_flag as u8,
                    sps.profile_tier_level.general_level_idc,
                    sps.bit_depth_luma,
                    sps.chroma_format,
                );
            }
        }
    }
}
```

**`H266Vps` key fields:** `vps_id`, plus the headline `profile_tier_level`
fields (carried on the VPS for the operating point set).

**`H266Sps` key fields:** `sps_id`, `vps_id`, `profile_tier_level`
(`general_profile_idc` / `general_tier_flag` / `general_level_idc`),
`width`, `height`, `chroma_format`, `bit_depth_luma`, `bit_depth_chroma`,
`color_info: Option<ColorInfo>`, `frame_rate: Option<Rational>`, `raw_rbsp`.

**`H266Pps` key fields:** `pps_id`, `sps_id`.

### Known limitations

- VPS + SPS + PPS only; APS NALs (types 17 / 18), Picture Header NALs
  (type 19), and multi-layer streams (`nuh_layer_id != 0`) pass through
  unparsed.
- Bails `ParseError::UnsupportedProfile` on `sps_subpic_info_present_flag = 1`
  and `sps_scaling_list_data_present_flag = 1` (rare; not in reference
  encoder defaults).
- `color_info` and `frame_rate` are surfaced as `None` today — VUI walking
  is stubbed pending the deeper SPS field-walk.
- See [deferred-features.md](deferred-features.md).

## AV1 quick start

AV1 has different bitstream framing — OBU (Open Bitstream Unit)
length-prefixed via LEB128, no Annex-B start codes. PMT `stream_type = 0x06`
with auto-emitted AV01 `registration_descriptor` per the AV1-in-MPEG-2-TS
binding §2.1. The demuxer surfaces AV1 access units as
`VideoPayload::Obus(Vec<Obu>)` rather than `Nals(_)`.

```rust,no_run
use tst_core::codec::av1;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload};

let mut dx = Demuxer::new();
// ... feed bytes ...

while let Some(ev) = dx.next_event() {
    if let DemuxEvent::Sample {
        payload: SamplePayload::Video { codec: VideoCodec::Av1, payload: VideoPayload::Obus(ref obus) },
        ..
    } = ev
    {
        let stream = av1::parse_obu_stream(obus);
        if let Some(seq) = &stream.sequence_header {
            println!(
                "AV1 {}x{} profile={} level={} tier={} {}-bit {:?}",
                seq.max_frame_width,
                seq.max_frame_height,
                seq.profile,
                seq.level,
                seq.tier,
                seq.bit_depth,
                seq.chroma_format,
            );
        }
    }
}
```

**`Av1SequenceHeader` key fields:** `profile`, `level`, `tier`,
`max_frame_width`, `max_frame_height`, `bit_depth`, `monochrome`,
`chroma_format`, `still_picture`, `reduced_still_picture_header`,
`color_info: Option<ColorInfo>`, `frame_rate: Option<Rational>`, `raw`.

**`Av1FrameHeaderLight` key fields:** `frame_type` (0 = KEY,
1 = INTER, 2 = INTRA_ONLY, 3 = SWITCH), `show_frame`,
`show_existing_frame`, `frame_size: Option<(u32, u32)>`. The size field
is always `None` in this slice — full per-frame decode would require
reference-frame management beyond the parser's scope.

**`Av1ObuStream` (returned by `parse_obu_stream`)** holds an optional
`sequence_header` and a `Vec<Av1FrameHeaderLight>` collected from the
input OBUs. Use it when you want a single call against an AU's OBU list
rather than walking individual OBUs.

### Known limitations

- Sequence Header + Frame Header light scope; full Frame Header parsing
  crosses into "you want a decoder."
- Operating points beyond 0 are walked past but not surfaced.
- Tile Group / Metadata / Padding OBUs pass through as
  `Obu::Other { obu_type, payload }` without further parsing.
- See [deferred-features.md](deferred-features.md).

## Error handling

All parse functions return `Result<T, ParseError>`. The two tiers:

- **`parse_parameter_sets`** is partial-success-tolerant. If some NALs parse
  correctly and some don't, the correctly-parsed ones fill the output maps and
  bad NALs emit `tracing::warn!` and are skipped. The function only returns
  `Err` if every parameter-set NAL in the input failed.
- **`parse_sps` / `parse_pps` / `parse_vps`** are strict: they return `Err` on
  the first parsing failure.

`ParseError` carries a human-readable description. The most common variant
encountered in production is `UnsupportedProfile` (the H.265 SPS parser
does not yet walk `scaling_list_data` or more than a trivial number of
short-term reference picture sets — not exercised by x265 default configuration
or the current corpus). Other variants: `InvalidNalType`, `TruncatedRbsp`,
`InvalidValue`.

## Calling on non-IDR samples

`parse_parameter_sets` is safe to call on every `Sample` event, including
P-frames and B-frames. Non-SPS/PPS NALs are silently skipped. The return
value for a P-frame is `Ok(H264ParameterSets { sps_by_id: {}, pps_by_id: {} })`.
No warnings are emitted.

## Cross-frame tracking

Encoders typically emit SPS + PPS on every IDR. To log only when the stream
configuration changes, maintain a per-PID snapshot and compare:

```rust,no_run
use std::collections::HashMap;
use tst_core::codec::h264;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload};

let mut last_summary: HashMap<u16, String> = HashMap::new();
let mut dx = Demuxer::new();

// (feed bytes and call drain_events in your real loop)

fn drain_events(dx: &mut Demuxer, last: &mut HashMap<u16, String>) {
    while let Some(ev) = dx.next_event() {
        let DemuxEvent::Sample {
            stream,
            payload: SamplePayload::Video { codec: VideoCodec::H264, payload: VideoPayload::Nals(ref nals) },
            ..
        } = ev
        else {
            continue;
        };
        if let Ok(ps) = h264::parse_parameter_sets(nals) {
            if let Some(sps) = ps.sps_by_id.values().next() {
                let summary = format!(
                    "{}x{} profile={} level={}",
                    sps.width, sps.height, sps.profile_idc, sps.level_idc
                );
                let pid = stream.pid;
                if last.get(&pid) != Some(&summary) {
                    println!("[PID 0x{pid:04X}] {summary}");
                    last.insert(pid, summary);
                }
            }
        }
    }
}
```

See the full runnable form at
[`crates/tst-srt/examples/parse_video_parameters.rs`](../crates/tst-srt/examples/parse_video_parameters.rs).

## Decoder replay — reconstituting Annex B bytes

The `raw_rbsp` field on each parsed struct preserves the input bytes verbatim,
including emulation-prevention bytes, exactly as received from the demuxer.
To reconstitute Annex-B-framed parameter set bytes for passing to a decoder
initialization sequence:

**H.264 SPS / PPS:**

```rust,no_run
fn to_annex_b_h264(rbsp: &[u8]) -> Vec<u8> {
    // H.264 NAL units use a 4-byte start code followed by the RBSP.
    // The NAL header byte is the first byte of the RBSP as returned
    // by the demuxer (NalUnit::H264.payload includes the header byte).
    let mut out = vec![0x00, 0x00, 0x00, 0x01];
    out.extend_from_slice(rbsp);
    out
}
```

**H.265 VPS / SPS / PPS:**

```rust,no_run
fn to_annex_b_h265(rbsp: &[u8]) -> Vec<u8> {
    // H.265 NAL units have a 2-byte NAL header before the RBSP.
    // NalUnit::H265.payload already includes the 2-byte header.
    let mut out = vec![0x00, 0x00, 0x00, 0x01];
    out.extend_from_slice(rbsp);
    out
}
```

Both are the same in practice — prepend the 4-byte start code to the raw
RBSP bytes from the demuxer. The difference is that H.265 NAL units have
a 2-byte header (nal_unit_type + layer_id + temporal_id_plus1) as part of
the payload, while H.264 has a 1-byte header.

## Shared types

`tst_srt::codec` re-exports several types used by both the H.264 and H.265
modules:

| Type | Description |
| --- | --- |
| `ChromaFormat` | `Monochrome` / `C420` / `C422` / `C444` |
| `Rational` | `{ num: u32, den: u32 }` — frame rate numerator / denominator |
| `ColorInfo` | `{ primaries, transfer, matrix }` — H.273-faithful decoded enums |
| `ColourPrimaries` | BT.709, BT.2020, DCI-P3, Unspecified, … (full ITU-T H.273 table) |
| `TransferCharacteristics` | BT.709, SMPTE ST 2084 (PQ), HLG, IEC 61966-2-1 (sRGB), … |
| `MatrixCoefficients` | BT.601, BT.709, BT.2020 NCL/CL, Identity, … |
| `ParseError` | Shared error type for all codec parsers |

The color enum decoders are verified against BT.2020 + PQ HDR fixtures to
ensure the numeric code points match the ITU-T H.273 Table 2 / Table 3 /
Table 4 assignments.

## Roadmap

`tst_srt::codec` is an umbrella for typed payload parsing across codec and
stream types. H.264, H.265, H.266, and AV1 parameter-set parsers ship today.
Future slices in the same umbrella (each landing additively when a consumer
asks):

- **H.266 APS / Picture Header** — APS NALs (types 17 / 18) and Picture
  Header NALs (type 19) pass through unparsed today; typed surfaces follow
  on consumer ask.
- **AV1 full Frame Header** — current `parse_frame_header_light` extracts
  type / show flags only; per-frame size + reference-frame management is a
  decoder-scope expansion.
- **SEI parsing** for H.264 and H.265 — HDR mastering display info, content
  light level, picture timing, etc.
- **Audio framing parsers** (`codec::aac::latm`, `codec::ac3`) — AAC LATM
  and AC-3 frame parsing; MP2 and AAC ADTS ship today (see "Audio frame
  parsing" section below).
- **Heuristic payload-kind detection** (`codec::detect`) — looks-like-ADTS /
  looks-like-UL+BER / looks-like-H.264 helpers for `Unknown` / private streams.

## Audio frame parsing

The MPEG-TS demuxer surfaces `SamplePayload::Audio { codec, frames: Vec<u8> }`
events carrying raw PES payload bytes. The `codec::mpegaudio` and `codec::aac`
modules parse those bytes into typed per-frame metadata (sample rate, channel
count, layer/profile, frame size) without decoding audio content.

Both modules expose the same shape: `fn frames(bytes) -> impl Iterator<Item =
Result<Frame, ParseError>>`. The iterator advances by header-decoded
`frame_length_bytes` and ends on first error or buffer end.

### `codec::mpegaudio` — MPEG-1 / MPEG-2 / MPEG-2.5 Layer I/II/III

```rust
use tst_core::codec;
use tst_core::mpegts::demux::{AudioCodec, SamplePayload};

if let SamplePayload::Audio { codec: AudioCodec::Mp2, frames, .. } = payload {
    for frame in codec::mpegaudio::frames(&frames) {
        let f = frame?;
        println!("layer={:?} version={:?} sample_rate={} channels={} bitrate_kbps={}",
            f.layer, f.version, f.sample_rate_hz, f.channels, f.bitrate_kbps);
    }
}
```

`AudioCodec::Mp2` covers stream_type `0x03` and `0x04`, which spans Layer I,
II, and III at all version × sample_rate combinations. The frame iterator
recovers the actual layer and version from each header.

### `codec::aac` — AAC ADTS

```rust
if let SamplePayload::Audio { codec: AudioCodec::Aac, frames, .. } = payload {
    for frame in codec::aac::frames(&frames) {
        let f = frame?;
        println!("profile={:?} sample_rate={} channels={} blocks={} samples_per_frame={}",
            f.profile, f.sample_rate_hz, f.channels,
            f.num_raw_data_blocks, f.samples_per_frame);
    }
}
```

Note: the ADTS `profile` field is a legacy MPEG-2 AAC concept. Most real-world
ADTS streams encode AAC-LC (`profile = Lc`) regardless of which MPEG-4 audio
object type the encoder used. Streams using HE-AAC, HE-AACv2, or other MPEG-4
AOTs are usually carried in LATM, not ADTS.

### What the parsers don't do

- **No CRC verification.** `has_crc` is surfaced; the CRC bytes are consumed
  for offset accounting but not validated. Callers wanting verification can
  use `frame.bytes()` to access the full frame slice and run their own CRC.
- **No mid-stream sync-resync.** First parse error ends iteration. PES payload
  bytes from the demuxer are sync-aligned by construction; mid-stream
  corruption surfaces as `NonConformantIssue::*` upstream.
- **No raw_data_block enumeration for multi-block AAC frames.** The frame is
  surfaced as one `AdtsFrame` with `samples_per_frame = 1024 *
  num_raw_data_blocks`. Splitting into individual blocks is decoder territory.

### Deferred

AAC LATM (LOAS/LATM framing per ISO 14496-3 §1.7) and AC-3 frame parsers are
deferred to follow-up plans. See `docs/deferred-features.md`.

See `docs/deferred-features.md` for the trigger conditions on each.
