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
from tstrans import _native

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

# `VideoSample` uses the same hand-written frozen-class pattern as
# `DemuxEvent.Video` in tstrans.mpegts (see `_VideoEvent`): `.raw` is a lazy
# `_native.RawBytes` holder (no copy until first access), stored as `_raw`.
# This avoids an eager bytes copy per sample for callers that only use `.pts`
# or `.codec` without reading the payload.

class VideoSample:
    """Projection of a paired/unpaired video access unit.

    Raw-first: `.raw` carries the exact encoded access unit bytes (Annex-B for
    H.26x; on-wire PES payload for AV1), materialized lazily on first access.
    `.random_access_indicator` reflects the TS-level RA flag. Parsed units are
    opt-in via `.parse()`.
    """

    __slots__ = (
        "stream",
        "pts",
        "dts",
        "codec",
        "_raw",  # native _native.RawBytes holder; `.raw` materializes lazily
        "random_access_indicator",
        "av1_carriage",
    )

    __match_args__ = (
        "stream",
        "pts",
        "dts",
        "codec",
        "raw",
        "random_access_indicator",
        "av1_carriage",
    )

    def __init__(
        self,
        *,
        stream: _mpegts.StreamId,
        pts: _mpegts.Pts90khz,
        dts: Optional[_mpegts.Pts90khz],
        codec: _mpegts.VideoCodec,
        raw,
        random_access_indicator: bool,
        av1_carriage: Optional[_mpegts.Av1CarriageMode] = None,
    ) -> None:
        object.__setattr__(self, "stream", stream)
        object.__setattr__(self, "pts", pts)
        object.__setattr__(self, "dts", dts)
        object.__setattr__(self, "codec", codec)
        # Accept either a pre-built holder (the pairer path) or raw bytes
        # (direct construction with `raw=b"..."`).
        object.__setattr__(
            self,
            "_raw",
            raw if isinstance(raw, _native.RawBytes) else _native.RawBytes(raw),
        )
        object.__setattr__(self, "random_access_indicator", random_access_indicator)
        object.__setattr__(self, "av1_carriage", av1_carriage)

    @property
    def raw(self) -> bytes:
        """The exact encoded access unit bytes. Materialized on first access and cached."""
        return self._raw.value

    def __setattr__(self, name, value):
        raise AttributeError(f"cannot assign to field {name!r}")

    def __delattr__(self, name):
        raise AttributeError(f"cannot delete field {name!r}")

    def __eq__(self, other) -> bool:
        if other.__class__ is not self.__class__:
            return NotImplemented
        return (
            self.stream == other.stream
            and self.pts == other.pts
            and self.dts == other.dts
            and self.codec == other.codec
            and self._raw == other._raw
            and self.random_access_indicator == other.random_access_indicator
            and self.av1_carriage == other.av1_carriage
        )

    def __hash__(self) -> int:
        return hash(
            (
                self.stream,
                self.pts,
                self.dts,
                self.codec,
                hash(self._raw),
                self.random_access_indicator,
                self.av1_carriage,
            )
        )

    def __repr__(self) -> str:
        return (
            f"VideoSample(stream={self.stream!r}, pts={self.pts!r}, "
            f"dts={self.dts!r}, codec={self.codec!r}, raw={self.raw!r}, "
            f"random_access_indicator={self.random_access_indicator!r}, "
            f"av1_carriage={self.av1_carriage!r})"
        )

    def parse(self, *, strict: bool = False):
        """Opt-in: split `raw` into typed NAL/OBU units. Lenient drops the issue
        list (use `tstrans.codec.split_units` if you want the issues).

        For AV1, `av1_carriage` is forwarded automatically so the framing
        expectation matches the on-wire bytes."""
        if strict:
            return _codec.split_units(self.raw, self.codec, strict=True,
                                      carriage=self.av1_carriage)
        units, _issues = _codec.split_units(self.raw, self.codec,
                                            carriage=self.av1_carriage)
        return units


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
