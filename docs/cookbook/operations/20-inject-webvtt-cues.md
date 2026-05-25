# Recipe 20: Inject WebVTT POI cues into a live MPEG-TS uplink

> **When to use this:** A sensor/orchestrator wants to mark Points of Interest in a live SRT/TS stream so the downstream HLS player can render them as captions.

> **Related:**
> - [guides/mpegts-mux.md](/docs/guides/mpegts-mux.md) — subtitle stream configuration and PMT shape
> - [Example: `mux_with_webvtt_subtitles`](/examples/muxing/mux_with_webvtt_subtitles.rs)

Use case: a sensor / orchestrator wants to mark Points of Interest
in a live SRT/TS stream so the downstream HLS player (hls.js etc.)
can render them as captions.

```rust
use tst_core::mpegts::mux::{
    MuxerConfig, MuxerProgramConfigBuilder, Muxer, SubtitleCodec, VideoCodec,
};

let cfg = {
    let mut prog = MuxerProgramConfigBuilder::new(1, 0x100);
    prog.add_video(0x101, VideoCodec::H264);
    prog.add_subtitle(0x200, SubtitleCodec::WebVttInTs);
    let mut b = MuxerConfig::builder();
    b.add_program(prog.build());
    b.build()?
};
let mut mux = Muxer::new(cfg)?;
let h = mux.subtitle_handles()[0];

// Each POI: assemble a WebVTT cue and push at the wall-clock PTS.
let cue = "WEBVTT\n\n00:00:01.000 --> 00:00:05.000\nPOI: target acquired\n";
mux.push_subtitle_to(h, 90_000, cue.as_bytes())?;
// Drain TS bytes via `mux.pull(&mut buf)` in a loop until it returns
// 0 (queue empty); see the runnable example for a `drain_all` helper.
```

Runnable: `cargo run -p tst-examples --example mux_with_webvtt_subtitles -- output.ts`.
