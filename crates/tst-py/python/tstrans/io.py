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

    # Compute packet_count from bytes actually read during the probe scan.
    # The demuxer doesn't expose a "TS packets fed" counter (see Rust
    # `DemuxerStats` — its fields are PSI/discontinuity/nonconformant
    # counts, not per-packet). Bytes-read / 188 is exact for any properly
    # 188-aligned TS file; an off-by-one in the last fragment is
    # acceptable since `read_total` is read in 64 KiB chunks (which are
    # always multiples of 188 ÷ chunk only when the underlying file is
    # 188-aligned, which valid TS files are). For files larger than the
    # 5 MiB probe budget, `packet_count` reflects packets in the scanned
    # prefix, matching the "first-N-MB scan summary" docstring.
    packet_count = read_total // 188

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
    *,
    with_pts: bool = False,
    parsed: bool = False,
    skip_unknown: bool = True,
) -> Iterator:
    """Iterate over KLV payloads in a file. Yields one of:

    - `bytes` (default — `with_pts=False, parsed=False`)
    - `(Pts90khz, bytes)` (when `with_pts=True, parsed=False`)
    - typed `UasDatalinkLs | SecurityLs | PrecisionTimeStampPack | VmtiLs`
      (when `parsed=True, with_pts=False`)
    - `(Pts90khz, typed)` (when `parsed=True, with_pts=True`)

    With `parsed=True`, each payload is run through
    `tstrans.klv.parse_klv_universal`. When the UL is unknown, the
    payload is skipped (default) or yielded as `None` /
    `(pts, None)` if `skip_unknown=False`.
    """

    # Local import dodges import-cycle with tstrans.klv at module load.
    from tstrans.klv import parse_klv_universal

    for ev in parse_file(path):
        if not isinstance(ev, DemuxEvent.Klv):
            continue
        if parsed:
            try:
                typed = parse_klv_universal(ev.payload)
            except Exception:  # noqa: BLE001 — caller decides via skip_unknown
                if skip_unknown:
                    continue
                typed = None
            if typed is None and skip_unknown:
                continue
            yield (ev.pts, typed) if with_pts else typed
        else:
            yield (ev.pts, ev.payload) if with_pts else ev.payload


__all__: list[str] = [
    "parse_file",
    "probe",
    "extract_klv",
    "ProbeResult",
]
