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

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class Pts90khz:
    """A 90 kHz timestamp tick count, the MPEG-TS PTS unit.

    Wraps an `i64` to allow signed-diff arithmetic (per the Rust
    `tst_core::mpegts::common::Pts90khz`). Construct via
    `Pts90khz.from_raw(int)`, `Pts90khz.from_ms(int)`, or
    `Pts90khz.from_seconds(float)`.
    """

    raw: int

    @classmethod
    def from_raw(cls, ticks: int) -> "Pts90khz":
        return cls(raw=int(ticks))

    @classmethod
    def from_ms(cls, ms: int) -> "Pts90khz":
        return cls(raw=int(ms) * 90)

    @classmethod
    def from_seconds(cls, seconds: float) -> "Pts90khz":
        return cls(raw=int(seconds * 90_000))

    @property
    def ms(self) -> int:
        # Truncating divide — matches Rust's integer arithmetic.
        return self.raw // 90

    @property
    def seconds(self) -> float:
        return self.raw / 90_000.0

    def __repr__(self) -> str:
        return f"Pts90khz(raw={self.raw}, ms={self.ms})"


# Population happens task-by-task. __all__ accumulates as types land.
__all__: list[str] = ["Pts90khz"]
