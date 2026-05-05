# Codec Parameter Set Guide

## What this module is

`srt_core::codec` provides typed payload parsers for codec parameter sets —
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

srt_core::codec::h264
  parse_sps(rbsp)         → Result<H264Sps, ParseError>
  parse_pps(rbsp)         → Result<H264Pps, ParseError>
  parse_parameter_sets(nals) → Result<H264ParameterSets, ParseError>

srt_core::codec::h265
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
use srt_core::codec::h264;
use srt_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload};

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
use srt_core::codec::h265;
use srt_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload};

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
use srt_core::codec::h264;
use srt_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload};

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
[`crates/srt-core/examples/parse_video_parameters.rs`](../crates/srt-core/examples/parse_video_parameters.rs).

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

`srt_core::codec` re-exports several types used by both the H.264 and H.265
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

`srt_core::codec` is an umbrella for typed payload parsing across codec and
stream types. The H.264 and H.265 parameter-set parsers are the first slice.
Future slices in the same umbrella (each landing additively when a consumer
asks):

- **AV1 sequence header** (`codec::av1`) — requires `VideoCodec::Av1` in the
  demuxer event surface. AV1 is OBU-shaped rather than NAL-shaped, so the
  demuxer event shape will differ from `NalUnit`.
- **H.266 VPS / SPS / PPS / APS** (`codec::h266`) — NAL-shaped per H.266;
  fits the existing `codec::*` pattern.
- **SEI parsing** for H.264 and H.265 — HDR mastering display info, content
  light level, picture timing, etc.
- **Audio framing parsers** (`codec::aac`, `codec::ac3`) — frame-header
  extraction from `SamplePayload::Audio` (the audio payload variant is
  `__Reserved` in the demuxer today; it lands when audio carriage is added).
- **Heuristic payload-kind detection** (`codec::detect`) — looks-like-ADTS /
  looks-like-UL+BER / looks-like-H.264 helpers for `Unknown` / private streams.

See `docs/deferred-features.md` for the trigger conditions on each.
