# Recipe 17: Extract video resolution and profile from a demuxed stream

> **When to use this:** You need typed codec information (width, height, profile, level, frame rate, color) and are already demuxing the stream.

> **Related:**
> - [guides/codec.md](/docs/guides/codec.md) — `parse_parameter_sets` API across H.264 / H.265 / H.266
> - [guides/mpegts-demux.md](/docs/guides/mpegts-demux.md) — `Sample` events with `VideoPayload::Nals`
> - [Example: `parse_video_parameters`](/examples/codec-parsing/parse_video_parameters.rs)

Reach for this when you need typed codec information (width, height, profile,
level, frame rate, color) and are already demuxing the stream. The demuxer
surfaces raw NAL bytes; you call the matching `codec::*` parser explicitly
on each `Sample` event. `parse_parameter_sets` is safe to call on every
sample — it skips non-SPS/PPS NALs silently and returns `Ok` with empty
maps on P-frames.

```rust,no_run
use tst_core::codec::h264;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dx = Demuxer::new();
    // ... feed bytes to dx ...
    while let Some(ev) = dx.next_event() {
        if let DemuxEvent::Sample {
            payload: SamplePayload::Video { codec: VideoCodec::H264, payload: VideoPayload::Nals(ref nals) },
            ..
        } = ev
        {
            if let Ok(ps) = h264::parse_parameter_sets(nals) {
                if let Some(sps) = ps.sps_by_id.values().next() {
                    println!(
                        "{}x{} profile={} level={}",
                        sps.width, sps.height, sps.profile_idc, sps.level_idc
                    );
                }
            }
        }
    }
    Ok(())
}
```

For H.265 substitute `h265::parse_parameter_sets` and use
`sps.general_profile_idc` / `sps.general_level_idc` (level is `× 30` — level
4.0 is stored as 120). The pattern is identical; only the import and field
names differ.

Runnable: [../../../examples/codec-parsing/parse_video_parameters.rs](../../../examples/codec-parsing/parse_video_parameters.rs) — shows change-driven logging per PID across H.264 and H.265 in one pass.
