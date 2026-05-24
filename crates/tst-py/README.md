# tstrans

Python bindings (via PyO3) for the [ts-transformer](https://github.com/aklofas/ts-transformer) Rust workspace.

> **Status (Phase 6 shipped, 2026-05-23):** `tstrans` is feature-complete for v1: file inspection + construction (`Demuxer` / `Muxer` / `MuxerFileSink`), typed KLV decode + encode for ST 0601 / ST 0102 / ST 0605 / ST 0903 (with `VTargetPack`), codec parsers for H.264 / H.265 / H.266 / AV1 / AAC / MPEG-2 audio, and optional pandas DataFrame adapters + NumPy snapshot views via `pip install tstrans[pandas]`. ~582 pytest tests. Live SRT (v2) and RTP (v3) transports remain on the roadmap. Minimum Python 3.10.

## Install

```
pip install tstrans
```

Optional extras for DataFrame / NumPy integration:

```
pip install 'tstrans[pandas]'
```

## Quickstart

Inspect a `.ts` file:

```python
from tstrans.io import parse_file, probe
from tstrans.mpegts import DemuxEvent

# Quick summary
r = probe("capture.ts")
print(r.video_codecs, r.audio_codecs, r.has_klv)

# Full event stream
for event in parse_file("capture.ts"):
    match event:
        case DemuxEvent.Video(pts=p, codec=c, payload=b):
            print(f"Video {c.name} pts={p.ms}ms len={len(b)}")
        case DemuxEvent.Klv(pts=p, payload=b):
            print(f"KLV pts={p.ms}ms len={len(b)}")
```

Build a `.ts` file (single-program H.264):

```python
from tstrans.mpegts import (
    Muxer,
    MuxerConfigBuilder,
    MuxerProgramConfigBuilder,
    Pts90khz,
    VideoCodec,
)

prog = (
    MuxerProgramConfigBuilder(program_number=1, pmt_pid=0x100)
    .add_video(0x101, VideoCodec.H264)
    .build()
)
cfg = MuxerConfigBuilder().add_program(prog).build()
m = Muxer(cfg)

with m.write_file("out.ts") as proxy:
    proxy.push_video(nal_bytes, Pts90khz.from_raw(900_000))
```

See [docs/guide-python.md](https://github.com/aklofas/ts-transformer/blob/main/docs/guide-python.md)
for the full guide and [docs/guide-python-pandas.md](https://github.com/aklofas/ts-transformer/blob/main/docs/guide-python-pandas.md)
for the DataFrame / NumPy integration.
