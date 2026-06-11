"""Type stubs for tstrans.io — see io.py for the runtime + full docstrings.

mypy --strict clean."""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterator, Optional, Tuple, Union

from tstrans.klv import UasDatalinkLs
from tstrans.mpegts import (
    AudioCodec,
    DemuxEvent,
    DemuxerConfig,
    ProgramMap,
    Pts90khz,
    SubtitleCodec,
    VideoCodec,
)

@dataclass(frozen=True, slots=True)
class ProbeResult:
    size_bytes: int
    programs: Tuple[ProgramMap, ...]
    pids: Tuple[int, ...]
    video_codecs: Tuple[VideoCodec, ...]
    audio_codecs: Tuple[AudioCodec, ...]
    subtitle_codecs: Tuple[SubtitleCodec, ...]
    has_klv: bool
    packet_count: int

def parse_file(
    path: Union[str, Path],
    config: Optional[DemuxerConfig] = ...,
) -> Iterator[DemuxEvent]: ...
def probe(
    path: Union[str, Path],
    *,
    config: Optional[DemuxerConfig] = ...,
) -> ProbeResult: ...
def extract_klv(
    path: Union[str, Path],
    *,
    with_pts: bool = ...,
    parsed: bool = ...,
    skip_unknown: bool = ...,
    skip_malformed: bool = ...,
    config: Optional[DemuxerConfig] = ...,
) -> Iterator[Any]: ...
def iter_uas_datalink(
    path: Union[str, Path],
    *,
    strict: bool = ...,
    config: Optional[DemuxerConfig] = ...,
) -> Iterator[Tuple[Pts90khz, int, UasDatalinkLs]]: ...

__all__ = ["parse_file", "probe", "extract_klv", "iter_uas_datalink", "ProbeResult"]
