"""tstrans.io — convenience helpers for reading + writing `.ts` files.

Phase 2 ships read-side helpers:

- `parse_file(path, config=None)` → `Iterator[DemuxEvent]`
- `probe(path)` → `ProbeResult` summary
- `extract_klv(path, with_pts=False)` → iterator of KLV payloads

Phase 4 adds the write-side `Muxer.write_file(path)` context manager.
"""

from dataclasses import dataclass
from pathlib import Path
from typing import Iterator, Optional, Union

from tstrans.mpegts import (
    AudioCodec,
    DemuxEvent,
    Demuxer,
    DemuxerConfig,
    Pts90khz,
    ProgramMap,
    StreamKindTag,
    SubtitleCodec,
    VideoCodec,
)


# Probe scans this many bytes from the start of the file. Enough for
# PSI + a handful of samples; tunable per call if a future caller needs
# more.
_PROBE_BYTES: int = 5 * 1024 * 1024

# Chunk size for feeding the demuxer. 64 KB is small enough to keep
# the iterator's memory footprint low and large enough to amortize the
# feed() call overhead.
_FEED_CHUNK: int = 64 * 1024


@dataclass(frozen=True, slots=True)
class ProbeResult:
    """First-N-MB scan summary. Cheaper than a full parse — enough for
    "what's in this file?" introspection."""

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
    config: Optional[DemuxerConfig] = None,
) -> Iterator[DemuxEvent]:
    """Open `path`, feed it to a `Demuxer` in chunks, and yield each
    `DemuxEvent` as it becomes available. Caller is responsible for
    iterating to completion (or stopping early — chunks are read on
    demand).

    Raises `tstrans.exceptions.DemuxError` in strict modes when the
    demuxer rejects a non-conformance.
    """

    p = Path(path)
    d = Demuxer(config) if config is not None else Demuxer()
    with p.open("rb") as f:
        while True:
            chunk = f.read(_FEED_CHUNK)
            if not chunk:
                break
            d.feed(chunk)
            # Drain available events between feeds. This keeps the
            # demuxer's queue from growing unboundedly while we read.
            while True:
                ev = d.next_event()
                if ev is None:
                    break
                yield ev
    d.flush()
    while True:
        ev = d.next_event()
        if ev is None:
            break
        yield ev


def probe(path: Union[str, Path]) -> ProbeResult:
    """Scan the first `_PROBE_BYTES` of a file and summarize. Returns
    a `ProbeResult` with program list, PID list, codec sets, and KLV
    presence. Does NOT compute duration (would require a full scan
    plus PCR analysis).
    """

    p = Path(path)
    size = p.stat().st_size
    d = Demuxer()
    read_total = 0
    with p.open("rb") as f:
        while read_total < _PROBE_BYTES:
            chunk = f.read(_FEED_CHUNK)
            if not chunk:
                break
            d.feed(chunk)
            read_total += len(chunk)
    d.flush()

    programs: list[ProgramMap] = []
    pids: set[int] = set()
    video_codecs: set[VideoCodec] = set()
    audio_codecs: set[AudioCodec] = set()
    subtitle_codecs: set[SubtitleCodec] = set()
    has_klv = False

    while True:
        ev = d.next_event()
        if ev is None:
            break
        if isinstance(ev, DemuxEvent.ProgramMap):
            for pm in ev.programs:
                programs.append(pm)
                for s in pm.streams:
                    pids.add(s.pid)
                    if s.kind is StreamKindTag.VIDEO and s.codec is not None:
                        video_codecs.add(s.codec)
                    elif s.kind is StreamKindTag.AUDIO and s.codec is not None:
                        audio_codecs.add(s.codec)
                    elif s.kind is StreamKindTag.SUBTITLE and s.codec is not None:
                        subtitle_codecs.add(s.codec)
                    elif s.kind in (StreamKindTag.KLV_SYNC, StreamKindTag.KLV_ASYNC):
                        has_klv = True

    stats = d.stats()
    # The actual stats field name for total packets fed; pick the
    # closest match. Fall back to summing whatever's present.
    packet_count = (
        stats.get("ts_packets_in", 0)
        or stats.get("packets_processed", 0)
        or sum(int(v) for v in stats.values() if isinstance(v, int))
    )

    return ProbeResult(
        size_bytes=size,
        programs=tuple(programs),
        pids=tuple(sorted(pids)),
        video_codecs=tuple(sorted(video_codecs, key=lambda c: c.name)),
        audio_codecs=tuple(sorted(audio_codecs, key=lambda c: c.name)),
        subtitle_codecs=tuple(sorted(subtitle_codecs, key=lambda c: c.name)),
        has_klv=has_klv,
        packet_count=packet_count,
    )


def extract_klv(
    path: Union[str, Path],
    with_pts: bool = False,
) -> Iterator:
    """Iterate over KLV payloads in a file. Yields `bytes` by default,
    or `(Pts90khz, bytes)` tuples when `with_pts=True`. KLV records
    are emitted as raw payload bytes — Phase 3's `tstrans.klv` adds
    the typed `Klv0601` decode."""

    for ev in parse_file(path):
        if isinstance(ev, DemuxEvent.Klv):
            if with_pts:
                yield (ev.pts, ev.payload)
            else:
                yield ev.payload


__all__: list[str] = [
    "parse_file",
    "probe",
    "extract_klv",
    "ProbeResult",
]
