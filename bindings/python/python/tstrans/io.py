"""tstrans.io — convenience helpers for reading + writing `.ts` files.

Read-side helpers:

- `parse_file(path, config=None)` → `Iterator[DemuxEvent]`
- `probe(path)` → `ProbeResult` summary
- `extract_klv(path, with_pts=False)` → iterator of KLV payloads

The write-side `Muxer.write_file(path)` context manager lives on the
`Muxer` class in `tstrans.mpegts`.
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

    Raises `tstrans.exceptions.DemuxError` when the demuxer rejects a
    non-conformance (strict modes) or when the byte stream is
    unrecoverable. I/O failures during file reading propagate as the
    underlying `OSError` from the `read` call. This matches the
    contract of Rust's `tst_core::io_file::TryDemuxFromFile` (the
    fallible streaming iterator) — both surface read and demux errors
    rather than coercing them to early EOF. The full file-helper error
    policy is documented in the module-level rustdoc of
    `tst_core::io_file`.
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


def probe(
    path: Union[str, Path],
    *,
    config: Optional[DemuxerConfig] = None,
) -> ProbeResult:
    """Scan the first `_PROBE_BYTES` of a file and summarize. Returns
    a `ProbeResult` with program list, PID list, codec sets, and KLV
    presence. Does NOT compute duration (would require a full scan
    plus PCR analysis).

    Pass `config` to use a non-default demuxer configuration (e.g.
    `DemuxerConfig(cfi_tolerance=False)` for spec-strict conformance
    testing — the default is tolerance-on, which rescues KLV from
    producer-malformed AU cell CFI bits while still emitting the
    `CfiTolerated` diagnostic). See `tstrans.mpegts.DemuxerConfig`
    for the full knob list.
    """

    p = Path(path)
    size = p.stat().st_size
    d = Demuxer(config) if config is not None else Demuxer()
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
    skip_malformed: bool = False,
    config: Optional[DemuxerConfig] = None,
) -> Iterator:
    """Iterate over KLV payloads in a file. Yields one of:

    - `bytes` (default — `with_pts=False, parsed=False`)
    - `(Pts90khz, bytes)` (when `with_pts=True, parsed=False`)
    - typed `UasDatalinkLs | SecurityLs | PrecisionTimeStampPack | VmtiLs`
      (when `parsed=True, with_pts=False`)
    - `(Pts90khz, typed)` (when `parsed=True, with_pts=True`)

    With `parsed=True`, each payload is run through
    `tstrans.klv.parse_klv_universal`. Two independent error knobs
    control how the iterator reacts:

    - `skip_unknown` (default `True`) — controls payloads whose
      universal label is not recognized by any of the four supported
      sets (ST 0601 / ST 0102 / ST 0605 / ST 0903). When True, such
      payloads are silently dropped. When False, `None` (or
      `(pts, None)`) is yielded so the caller can count or log them.
    - `skip_malformed` (default `False`) — controls
      `tstrans.exceptions.KlvError` raised by the decoder on a
      recognized UL (truncated set, bad checksum, etc.). When False
      (the default), the exception propagates so data corruption is
      not silently lost. When True, the offending row is skipped.

    These two knobs are independent on purpose: prior to the audit-#3
    fix, a single `skip_unknown=True` overload swallowed both unknown
    ULs and `KlvError` from known ULs, hiding decoder bugs and
    upstream corruption. Catching `KlvError` specifically (rather than
    bare `Exception`) also lets binding-shape regressions (TypeError,
    AttributeError) surface naturally instead of being suppressed.

    Pass `config` to use a non-default demuxer configuration. CFI
    tolerance is **on by default** — the demuxer rescues complete KLV
    records from sync-metadata streams whose encoders set the AU cell
    `cell_fragment_indication` bits to `0b00` (Middle) or `0b01`
    (Last) for what are actually single complete records (a
    corpus-dominant real-world malformation pattern), while still
    emitting the `CfiTolerated` diagnostic so the malformation
    remains visible. Set `DemuxerConfig(cfi_tolerance=False)` for
    spec-strict conformance testing. See
    `tstrans.mpegts.DemuxerConfig` for the full knob list.
    """

    # Local imports dodge import-cycle with tstrans.klv at module load.
    from tstrans.exceptions import KlvError
    from tstrans.klv import parse_klv_universal

    for ev in parse_file(path, config=config):
        if not isinstance(ev, DemuxEvent.Klv):
            continue
        if parsed:
            try:
                typed = parse_klv_universal(ev.payload)
            except KlvError:
                if skip_malformed:
                    continue
                raise
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
