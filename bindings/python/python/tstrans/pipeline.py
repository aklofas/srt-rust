"""tstrans.pipeline — KLV ↔ video PTS pairing.

Public types:

- `Pairer` — byte-feeding pairer; feed TS bytes, get `PairerOutput`s.
- `PairerMode` — `Realtime` | `Buffered(max_lag=...)`.
- `PairerConfig`, `PairingDemuxerConfig` — tuning.
- `PairerOutput` base + subclasses: `Paired`, `UnpairedVideo`,
  `UnpairedKlv`, `PassThrough`.
- `VideoSample`, `KlvSample` — projections fed into outputs.

`Pairer` owns a demuxer + pairer internally: you feed raw TS bytes
(`feed(bytes)`) rather than `DemuxEvent`s, so events never round-trip
across the binding boundary.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import timedelta
from typing import ClassVar, Optional, Union

import tstrans.mpegts as _mpegts
import tstrans.codec as _codec

# The native byte-feeding pairer (#[pyclass] in src/pipeline.rs).
from tstrans._native import Pairer  # noqa: F401  (re-export)

# --- PairerMode (Realtime | Buffered{max_lag}) ------------------------------
# Namespace-class pattern (mirrors DemuxEvent): `PairerMode.Realtime` is a
# singleton instance; `PairerMode.Buffered(max_lag=...)` constructs a variant.

class PairerMode:
    """Pairing strategy discriminator."""

    Realtime: ClassVar["_RealtimeMode"]
    Buffered: ClassVar[type["_BufferedMode"]]


@dataclass(frozen=True, slots=True)
class _RealtimeMode(PairerMode):
    """Eager pairing; emit on each feed. No lookahead buffer."""


@dataclass(frozen=True, slots=True)
class _BufferedMode(PairerMode):
    """Buffer up to `max_lag` of arrival skew before forced emit."""
    max_lag: timedelta
    def __post_init__(self) -> None:
        if not isinstance(self.max_lag, timedelta):
            raise TypeError("PairerMode.Buffered.max_lag must be a datetime.timedelta")
        if self.max_lag < timedelta(0):
            raise ValueError("PairerMode.Buffered.max_lag must be non-negative")


PairerMode.Realtime = _RealtimeMode()
PairerMode.Buffered = _BufferedMode

# --- PairerConfig / PairingDemuxerConfig ------------------------------------


@dataclass(frozen=True, slots=True)
class PairerConfig:
    mode: Union[_RealtimeMode, _BufferedMode] = field(default_factory=_RealtimeMode)
    tolerance: timedelta = field(default_factory=lambda: timedelta(milliseconds=300))
    max_buffered_klv: int = 32
    max_buffered_video: int = 32
    link_klv_to_video: bool = True

    def __post_init__(self) -> None:
        if not isinstance(self.mode, (_RealtimeMode, _BufferedMode)):
            raise TypeError("PairerConfig.mode must be PairerMode.Realtime or PairerMode.Buffered(...)")
        if not isinstance(self.tolerance, timedelta):
            raise TypeError("PairerConfig.tolerance must be a datetime.timedelta")
        if isinstance(self.max_buffered_klv, bool) or not isinstance(self.max_buffered_klv, int):
            raise TypeError("PairerConfig.max_buffered_klv must be an int")
        if isinstance(self.max_buffered_video, bool) or not isinstance(self.max_buffered_video, int):
            raise TypeError("PairerConfig.max_buffered_video must be an int")
        if self.max_buffered_klv <= 0:
            raise ValueError("PairerConfig.max_buffered_klv must be > 0")
        if isinstance(self.mode, _BufferedMode) and self.max_buffered_video <= 0:
            raise ValueError("PairerConfig.max_buffered_video must be > 0 in Buffered mode")


@dataclass(frozen=True, slots=True)
class PairingDemuxerConfig:
    pairer: PairerConfig = field(default_factory=PairerConfig)
    demuxer: Optional[_mpegts.DemuxerConfig] = None  # None → demuxer defaults

# --- Sample projections -----------------------------------------------------


@dataclass(frozen=True, slots=True)
class VideoSample:
    stream: _mpegts.StreamId
    pts: _mpegts.Pts90khz
    dts: Optional[_mpegts.Pts90khz]
    codec: _mpegts.VideoCodec
    payload: Union[list[_codec.NalUnit], list[_codec.Obu]]


@dataclass(frozen=True, slots=True)
class KlvSample:
    stream: _mpegts.StreamId
    pts: _mpegts.Pts90khz
    kind: _mpegts.MetadataKindTag
    payload: bytes

# --- PairerOutput (Paired | UnpairedVideo | UnpairedKlv | PassThrough) -------


class PairerOutput:
    """One emission from `Pairer.feed` / `Pairer.flush`. Match on the
    nested subclasses (3.10+ `match` / `isinstance`)."""

    Paired: ClassVar[type["_Paired"]]
    UnpairedVideo: ClassVar[type["_UnpairedVideo"]]
    UnpairedKlv: ClassVar[type["_UnpairedKlv"]]
    PassThrough: ClassVar[type["_PassThrough"]]


@dataclass(frozen=True, slots=True)
class _Paired(PairerOutput):
    video: VideoSample
    klv: KlvSample


@dataclass(frozen=True, slots=True)
class _UnpairedVideo(PairerOutput):
    video: VideoSample


@dataclass(frozen=True, slots=True)
class _UnpairedKlv(PairerOutput):
    klv: KlvSample


@dataclass(frozen=True, slots=True)
class _PassThrough(PairerOutput):
    event: object  # a tstrans.mpegts.DemuxEvent.* instance


PairerOutput.Paired = _Paired
PairerOutput.UnpairedVideo = _UnpairedVideo
PairerOutput.UnpairedKlv = _UnpairedKlv
PairerOutput.PassThrough = _PassThrough
