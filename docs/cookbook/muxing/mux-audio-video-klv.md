# Mux audio + video + KLV in a single program

> **When to use this:** Build a three-stream program where audio PTS-aligns with video for synchronized playback and KLV records emit on the same PCR clock.

> **Related:**
> - [guides/mpegts-mux.md](/docs/guides/mpegts-mux.md) — audio + video + metadata composition
> - [Example: `mux_audio_video_klv`](/examples/muxing/mux_audio_video_klv.rs)

Build a three-stream program where audio PTS-aligns with video for
synchronized playback, and KLV records emit on the same PCR clock.

```rust
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{
    AudioCodec, KlvStreamType, MuxerConfig, MuxerProgramConfigBuilder, Muxer,
    VideoCodec,
};

// add_audio_with_language auto-emits an iso_639_language_descriptor
// (tag 0x0A) on the PMT entry — receivers (browsers, transcoders,
// players) get a language hint without manually wiring descriptors.
// Use plain add_audio(pid, codec) when language is unknown / unset.
let cfg = {
    let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
    prog.add_video(0x100, VideoCodec::H264);
    prog.add_klv(0x200, KlvStreamType::PrivateData, /*carries_pts=*/ false);
    prog.add_audio_with_language(0x300, AudioCodec::Aac, *b"eng");
    let mut b = MuxerConfig::builder();
    b.add_program(prog.build());
    b.build()?
};

let mut muxer = Muxer::new(cfg)?;

for frame_idx in 0..30 {
    let pts = 90_000 + frame_idx * 3000;
    muxer.push_video(&video_au_bytes, Pts90khz::new(pts), /*key_frame=*/ frame_idx % 30 == 0)?;
    muxer.push_audio(&aac_frame_bytes, Pts90khz::new(pts))?;
    if frame_idx % 30 == 0 {
        muxer.push_klv(&klv_record, Pts90khz::new(pts), /*metadata_service_id=*/ 0x00)?;
    }
    // Drain to your transport.
}
```

Full example: [`examples/muxing/mux_audio_video_klv.rs`](/examples/muxing/mux_audio_video_klv.rs).
