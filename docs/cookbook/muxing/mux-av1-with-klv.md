# Mux AV1 video with KLV

> **When to use this:** The encoder produces AV1 — note OBU framing replaces Annex-B NAL framing.

> **Related:**
> - [guides/mpegts-mux.md](/docs/guides/mpegts-mux.md) — AV1 binding-conformant default and stream_type 0x06
> - [guides/codec.md](/docs/guides/codec.md) — OBU framing and `obu_has_size_field` requirements
> - [Example: `mux_av1_with_klv`](/examples/muxing/mux_av1_with_klv.rs)

AV1 uses OBU framing — fundamentally different from the NAL-shaped codecs
(H.264 / H.265 / H.266). Key differences when feeding `Muxer::push_video`:

- **No Annex-B start codes.** OBUs are self-describing and length-prefixed
  via LEB128. Concatenating OBUs with no separator produces a complete
  access unit.
- **AV1-in-MPEG-2-TS binding §3.1 requires `obu_has_size_field = 1`** on
  every OBU so the demultiplexer can walk the OBU stream without a
  separate framing layer.
- **PMT `stream_type = 0x06`** plus an auto-emitted `AV01`
  `registration_descriptor` (binding §2.1) tells receivers the bytes are
  AV1 rather than KLV-async on the same stream_type byte.

```rust,no_run
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{MuxerConfig, KlvStreamType, Muxer, StreamSpec, VideoCodec};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = MuxerConfig {
        streams: vec![
            StreamSpec::Video { pid: 0x1011, codec: VideoCodec::Av1 },
            StreamSpec::Klv {
                pid: 0x1031,
                stream_type: KlvStreamType::PrivateData,
                carries_pts: false,
            },
        ],
        ..MuxerConfig::default()
    };
    let mut mux = Muxer::new(cfg)?;
    // `au_obus` is a contiguous OBU sequence (each with obu_has_size_field=1).
    // The example builds one synthetic Sequence Header + Temporal Delimiter +
    // Frame access unit; real consumers feed the encoder's output verbatim.
    let au_obus: Vec<u8> = vec![/* concatenated OBUs */];
    mux.push_video(&au_obus, Pts90khz::new(0), /* key_frame = */ true)?;
    Ok(())
}
```

Runnable: [examples/muxing/mux_av1_with_klv.rs](/examples/muxing/mux_av1_with_klv.rs).
