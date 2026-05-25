# Recipe 9: Mux H.265 + sync KLV

> **When to use this:** The encoder produces HEVC, or the receiver requires strict ST 1402 sync metadata (PMT stream_type 0x15) instead of the default async private-data shape.

> **Related:**
> - [guides/mpegts-mux.md](/docs/guides/mpegts-mux.md) — "KLV-in-TS modes" + the AU-cell auto-wrap contract
> - [guides/klv.md](/docs/guides/klv.md) — caller-owned raw KLV LS bytes (no pre-wrapping)
> - [Example: `mux_h265_with_klv`](/examples/muxing/mux_h265_with_klv.rs)

Reach for this when the encoder produces HEVC, or when the receiver requires strict ST 1402 sync metadata (PMT stream_type 0x15) instead of the default async private-data shape. Three knobs flip on `MuxerConfig`: codec → `H265`, KLV stream type → `SynchronousMetadata`, `carries_pts` → `true`.

**Sync KLV auto-wraps in the muxer.** When you configure `KlvStreamType::SynchronousMetadata`, `Muxer::push_klv` auto-prepends a 5-byte `Metadata_AU_cell` header per ITU-T H.222.0 V9 § 2.12.4.2 (Tables 2-155+2-156) before TS-framing. Pass raw KLV LS bytes — do not pre-wrap. PTS lives in the PES header (per § 2.12.4.1). See [guides/mpegts-mux.md](/docs/guides/mpegts-mux.md) §"KLV-in-TS modes".

```rust,no_run
use tst_core::mpegts::mux::{
    KlvStreamType, MuxerConfig, MuxerProgramConfigBuilder, Muxer, VideoCodec,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build the program independently, then plug it into the top-level
    // MuxerConfig::builder. Mutators take `&mut self` and return
    // `&mut Self`, so each step is its own statement on a bound builder.
    let cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x1011, VideoCodec::H265);
        prog.add_klv(0x1031, KlvStreamType::SynchronousMetadata, /*carries_pts=*/ true);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build()?
    };
    let mut mux = Muxer::new(cfg)?;
    let inner_klv: Vec<u8> = vec![/* ST 0601 bytes */];
    // Muxer auto-prepends the 5-byte AU cell header. metadata_service_id
    // defaults to 0x00 per ST 1402.2 App. B Table 2.
    mux.push_klv(&inner_klv, /*pts_90khz=*/ 0, /*metadata_service_id=*/ 0x00)?;
    Ok(())
}
```

Runnable: [../../../examples/muxing/mux_h265_with_klv.rs](../../../examples/muxing/mux_h265_with_klv.rs).
