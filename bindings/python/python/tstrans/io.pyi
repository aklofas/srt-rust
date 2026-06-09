"""Type stubs for tstrans.io — see io.py for the runtime + full docstrings."""
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterator, Optional, Union

from tstrans.mpegts import (
    AudioCodec,
    DemuxEvent,
    DemuxerConfig,
    ProgramMap,
    SubtitleCodec,
    VideoCodec,
)

@dataclass(frozen=True, slots=True)
class ProbeResult:
    size_bytes: int
    programs: tuple[ProgramMap, ...]
    pids: tuple[int, ...]
    video_codecs: tuple[VideoCodec, ...]
    audio_codecs: tuple[AudioCodec, ...]
    subtitle_codecs: tuple[SubtitleCodec, ...]
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

__all__ = ["parse_file", "probe", "extract_klv", "ProbeResult"]
