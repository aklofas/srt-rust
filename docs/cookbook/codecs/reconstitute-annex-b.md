# Reconstitute Annex B parameter sets for decoder replay

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

Split the AU into NAL units with `split_video(&raw, codec, av1_carriage.unwrap_or_default())`, parse the
parameter sets, and rebuild Annex-B framing around each one. The `raw_rbsp` field on
each parsed struct preserves the NAL's **RBSP body** verbatim (emulation-prevention
bytes intact) — but the 1-byte NAL header was stripped during the split, so a
conformant Annex-B NAL needs **both** a start code **and** the NAL header byte
re-prepended (`0x67` for SPS, `0x68` for PPS):

```rust,no_run
use tst_core::codec::h264;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoCodec, VideoPayload, split_video};

fn to_annex_b(nal_header: u8, rbsp: &[u8]) -> Vec<u8> {
    // `split_video` strips the 1-byte H.264 NAL header from each unit's payload,
    // so `raw_rbsp` is the RBSP body only (emulation-prevention bytes preserved).
    // Rebuild a conformant Annex-B NAL by prepending the start code AND the NAL
    // header byte. (H.265 / H.266 use a 2-byte NAL header — prepend those two
    // bytes instead.)
    let mut out = vec![0x00, 0x00, 0x00, 0x01, nal_header];
    out.extend_from_slice(rbsp);
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut dx = Demuxer::new();
    // ... feed bytes ...
    while let Some(ev) = dx.next_event() {
        if let DemuxEvent::Sample {
            payload: SamplePayload::Video { codec: codec @ VideoCodec::H264, raw, av1_carriage, .. },
            ..
        } = ev
        {
            let (VideoPayload::Nals(nals), _issues) = split_video(&raw, codec, av1_carriage.unwrap_or_default()) else {
                continue;
            };
            if let Ok(ps) = h264::parse_parameter_sets(&nals) {
                let mut decoder_config: Vec<u8> = Vec::new();
                for sps in ps.sps_by_id.values() {
                    // SPS NAL header: nal_ref_idc=3, nal_unit_type=7 -> 0x67
                    decoder_config.extend(to_annex_b(0x67, &sps.raw_rbsp));
                }
                for pps in ps.pps_by_id.values() {
                    // PPS NAL header: nal_ref_idc=3, nal_unit_type=8 -> 0x68
                    decoder_config.extend(to_annex_b(0x68, &pps.raw_rbsp));
                }
                // Pass decoder_config to your hardware decoder or codec library.
            }
        }
    }
    Ok(())
}
```

Runnable: [../../../examples/codec-parsing/parse_video_parameters.rs](../../../examples/codec-parsing/parse_video_parameters.rs) shows the full demux-to-parse loop; see `docs/guides/codec.md` for the decoder-replay section.
