# Recipe 18: Reconstitute Annex B parameter sets for decoder replay

> **When to use this:** You need to hand SPS / PPS bytes to a hardware decoder, encoder re-init, or a library that expects Annex-B-framed codec configuration.

> **Related:**
> - [guides/codec.md](/docs/guides/codec.md) — `raw_rbsp` preservation and decoder replay section
> - [Example: `parse_video_parameters`](/examples/codec-parsing/parse_video_parameters.rs)

Reach for this when you need to hand **just the SPS / PPS** bytes to a hardware
decoder, encoder re-init, or a library that expects Annex-B-framed codec
configuration. (If you want the *whole* access unit back as Annex-B bytes, you
don't need this recipe at all under the raw-first model — `SamplePayload::Video.raw`
already IS the Annex-B AU, start codes intact. This recipe is for isolating the
parameter sets out of that AU.)

Split the AU into NAL units with `split_video(&raw, codec)`, parse the
parameter sets, and rebuild start codes around each one. The `raw_rbsp` field on
each parsed struct preserves the input bytes verbatim (including
emulation-prevention bytes). Prepend a 4-byte start code to get conformant
Annex B framing:

```rust,no_run
use tst_core::codec::h264;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload, split_video};

fn to_annex_b(rbsp: &[u8]) -> Vec<u8> {
    // Same for H.264 and H.265 — `split_video` includes the NAL header byte(s)
    // in each unit's payload, so raw_rbsp already contains the full NAL unit
    // minus its Annex-B start code. Just prepend the start code.
    let mut out = vec![0x00, 0x00, 0x00, 0x01];
    out.extend_from_slice(rbsp);
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dx = Demuxer::new();
    // ... feed bytes ...
    while let Some(ev) = dx.next_event() {
        if let DemuxEvent::Sample {
            payload: SamplePayload::Video { codec: codec @ VideoCodec::H264, raw, .. },
            ..
        } = ev
        {
            let (VideoPayload::Nals(nals), _issues) = split_video(&raw, codec) else {
                continue;
            };
            if let Ok(ps) = h264::parse_parameter_sets(&nals) {
                let mut decoder_config: Vec<u8> = Vec::new();
                for sps in ps.sps_by_id.values() {
                    decoder_config.extend(to_annex_b(&sps.raw_rbsp));
                }
                for pps in ps.pps_by_id.values() {
                    decoder_config.extend(to_annex_b(&pps.raw_rbsp));
                }
                // Pass decoder_config to your hardware decoder or codec library.
            }
        }
    }
    Ok(())
}
```

Runnable: [../../../examples/codec-parsing/parse_video_parameters.rs](../../../examples/codec-parsing/parse_video_parameters.rs) shows the full demux-to-parse loop; see `docs/guides/codec.md` for the decoder-replay section.
