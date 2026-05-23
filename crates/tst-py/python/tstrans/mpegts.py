"""tstrans.mpegts — MPEG-TS packet, PES, PSI, muxer, demuxer.

Phase 2 of the tst-py v1 plan added the demuxer surface:

- `Pts90khz` — 90 kHz timestamp wrapper
- `VideoCodec`, `AudioCodec`, `SubtitleCodec`, `StrictMode` enums
- `StreamId`, `StreamInfo`, `KlvLink`, `ProgramMap` dataclasses
- `DemuxEvent` base + subclasses (`ProgramMap`, `Video`, `Audio`,
  `Subtitle`, `Klv`, `Discontinuity`, `NonConformant`,
  `ReconnectDiscontinuity`)
- `DemuxerConfig`, `Demuxer` — feed bytes, get events

Phase 4 adds `Muxer` + `MuxerConfig` here.
"""

# Population happens task-by-task. __all__ accumulates as types land.
__all__: list[str] = []
