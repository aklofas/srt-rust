# tstrans

Python bindings (via PyO3) for the [ts-transformer](https://github.com/aklofas/ts-transformer) Rust workspace.

> **Status:** `tstrans` is feature-complete for v1: file inspection + construction (`Demuxer` / `Muxer` / `MuxerFileSink`), typed KLV decode + encode for ST 0601 / ST 0102 / ST 0605 / ST 0903 (with `VTargetPack`), codec parsers for H.264 / H.265 / H.266 / AV1 / AAC / MPEG-2 audio, optional pandas DataFrame adapters + NumPy snapshot views via `pip install tstrans[pandas]`, and **RTP + RTSP transport** via the `tstrans.rtp` submodule (Sender / Receiver / MuxSender / DemuxReceiver / RtspClient / RtspServer / MountHandle). ~810+ pytest tests. Live SRT transport remains on the roadmap. Minimum Python 3.10.

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

## RTP + RTSP transport

`tstrans.rtp` ships RTP-over-UDP and a full RTSP client + server. Available
by default in published wheels — no extra needed (`pip install tstrans`
already includes it). The Rust-side `rtp` cargo feature is on by default;
source builds that don't need it can opt out with
`maturin develop --no-default-features` for a smaller binary.

Connect to a camera and iterate demuxed events:

```python
from tstrans.rtp import RtspClient, RtspClientConfig
from tstrans.mpegts import DemuxEvent

cfg = RtspClientConfig(url="rtsp://camera.local/live")
with RtspClient.connect(cfg) as session:
    demux = session.into_demux_receiver()
    for event in demux:
        if isinstance(event, DemuxEvent.Video):
            handle(event.payload)
```

Publish a stream from your own RTSP server:

```python
from tstrans.rtp import RtspServer, RtspServerConfig
from tstrans.mpegts import MuxerProgramConfigBuilder, Pts90khz, VideoCodec

program = (
    MuxerProgramConfigBuilder(1, 0x100)
    .add_video(0x101, VideoCodec.H264)
    .build()
)
with RtspServer.start(RtspServerConfig(bind_addr="0.0.0.0:8554")) as server:
    mount = server.add_unicast_mount("/live", program)
    mount.push_video(nal_bytes, pts=Pts90khz.from_raw(0), key_frame=True)
```

The full surface — `Sender` / `Receiver` for raw RTP, `MuxSender` /
`DemuxReceiver` for one-call mux/demux convenience, `RtspClient` /
`RtspSession` with Basic + Digest auth and TCP-interleaved fallback,
`RtspServer` + `MountHandle` with 16 push methods and multicast mounts —
is documented in the per-class docstrings and the `python/tstrans/rtp.pyi`
type stubs (`mypy --strict` clean).

## See also

See [docs/languages/python.md](https://github.com/aklofas/ts-transformer/blob/main/docs/languages/python.md)
for the full guide and [docs/languages/python.md](https://github.com/aklofas/ts-transformer/blob/main/docs/languages/python.md)
for the DataFrame / NumPy integration.
