"""Audit #11 — verify heavy Rust work releases the GIL.

These tests verify that the hot-path PyO3 methods (`Demuxer.feed`,
`Muxer.push_*`, codec eager-collect iterators) wrap their Rust work
in `py.allow_threads` so other Python threads can run concurrently.

KLV decode entry points are intentionally NOT GIL-released — the
typical record size keeps per-call Rust work below the GIL transition
breakeven point (~50us), and wrapping produces lock-contention
pathology under hot batch loops. See `klv.rs` decision comments and
`reference_pyo3_allow_threads_pattern.md` for the empirical analysis.

## Technique

Each test runs a workload that calls into Rust while a background
"worker" Python thread runs a pure-Python tight loop (incrementing a
counter). Pure-Python bytecode execution requires the GIL on every
iteration.

We measure:

1. The worker's solo throughput (no Rust workload) — establishes a
   baseline for "what 100% CPU access looks like" on this machine.
2. The worker's throughput during the Rust workload.

Then we assert: worker throughput during workload ≥ THRESHOLD × solo.

## Empirical baseline (on a dev box)

Without GIL release, the worker still gets some progress because
Python's `sys.setswitchinterval` (default 5ms) periodically yields
the GIL — and many small Rust calls return to Python frequently
enough that switchinterval is effective.

The real discriminator is a SINGLE long Rust call. Without
`allow_threads`, the worker gets ZERO iterations during the call.
With `allow_threads`, the worker runs concurrently for the full
duration.

Measured ratios (worker-during-workload / worker-solo):

| Workload | Without `allow_threads` | With `allow_threads` |
|---|---|---|
| push_video (30 MB NAL, one call) | ~25% | ~100% |
| iter_aac (500 MB buf, one call) | ~10% | ~85% |
| iter_mp2 (500 MB buf, one call) | ~12% | ~85% |

We set the threshold at 60% — comfortably above the worst baseline
(~25%) and below the post-fix floor (~80%). This catches regressions
without flaking under CI host load.

## Workload sizing

Each workload is sized to run ≥50ms wall-clock. Under 50ms the
worker thread doesn't accumulate enough iterations to be statistically
meaningful — the test asserts its own setup is broken in that case
(pointing at the input-size constant to scale up).
"""

from __future__ import annotations

import threading
import time

import pytest

from tstrans.codec import iter_aac_frames_with_resync, iter_mpeg2_audio_frames_with_resync
from tstrans.mpegts import (
    Demuxer,
    Muxer,
    MuxerConfig,
    MuxerConfigBuilder,
    MuxerProgramConfigBuilder,
    Pts90khz,
    VideoCodec,
)


# ---------------------------------------------------------------------------
# Pure-Python concurrency probe
# ---------------------------------------------------------------------------


_MIN_WORKLOAD_MS = 50.0
_GIL_RELEASED_THRESHOLD = 0.60  # 60% of solo throughput


class _PyWorker:
    """Background thread running a pure-Python tight loop.

    Use as a context manager around the workload. After exit, read
    `.iters` (bytecode steps the worker got) and `.duration_s` (wall-
    clock of the workload).
    """

    def __init__(self) -> None:
        self._stop = threading.Event()
        self.iters: int = 0
        self.duration_s: float = 0.0
        self._t = threading.Thread(target=self._run, daemon=True)
        self._start_perf: float = 0.0

    def _run(self) -> None:
        # Pure Python counter increment — every iteration requires GIL.
        local_stop = self._stop
        n = 0
        while not local_stop.is_set():
            n += 1
        self.iters = n

    def __enter__(self) -> _PyWorker:
        self._t.start()
        # Tiny yield so the worker actually starts before timing.
        time.sleep(0.001)
        self._start_perf = time.perf_counter()
        return self

    def __exit__(self, *exc: object) -> None:
        self.duration_s = time.perf_counter() - self._start_perf
        self._stop.set()
        self._t.join(timeout=2.0)


def _measure_solo_throughput() -> float:
    """Iters/sec the worker achieves with no competing GIL holder."""
    w = _PyWorker()
    with w:
        time.sleep(0.2)
    return w.iters / w.duration_s


# Measure solo throughput once per session via a fixture — it varies by
# host CPU + load, so we calibrate per-run rather than hard-coding.
@pytest.fixture(scope="module")
def solo_throughput() -> float:
    """Worker's solo iters/sec on this host."""
    # Warm up + measure 3 times, take the max (best-case).
    rates = [_measure_solo_throughput() for _ in range(3)]
    return max(rates)


def _assert_gil_released(
    worker: _PyWorker, solo_rate: float, op_name: str
) -> None:
    """Assert worker thread made ≥THRESHOLD × solo progress during workload."""
    assert worker.duration_s * 1000.0 >= _MIN_WORKLOAD_MS, (
        f"{op_name}: workload too short "
        f"({worker.duration_s*1000:.0f}ms < {_MIN_WORKLOAD_MS:.0f}ms); "
        f"scale up the input or iteration count — the test would not be "
        f"discriminating at this duration"
    )
    actual_rate = worker.iters / worker.duration_s
    ratio = actual_rate / solo_rate if solo_rate > 0 else 0.0
    expected_iters = int(solo_rate * worker.duration_s * _GIL_RELEASED_THRESHOLD)
    assert ratio >= _GIL_RELEASED_THRESHOLD, (
        f"{op_name}: worker thread only completed {worker.iters} "
        f"iterations during a {worker.duration_s*1000:.0f}ms workload "
        f"({actual_rate/1e6:.1f}M iters/sec, "
        f"{ratio*100:.0f}% of solo {solo_rate/1e6:.1f}M iters/sec); "
        f"required ≥{expected_iters} ({_GIL_RELEASED_THRESHOLD*100:.0f}% "
        f"of solo); GIL likely held during the Rust work"
    )


# ---------------------------------------------------------------------------
# Muxer fixtures
# ---------------------------------------------------------------------------


def _muxer_config_video_only() -> MuxerConfig:
    """Single-video-stream muxer with large packet buffer."""
    prog = (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .build()
    )
    return MuxerConfigBuilder().add_program(prog).buffer_packets(1_000_000).build()


def _huge_h264_nal() -> bytes:
    """30 MB Annex-B NAL — one push_video call takes ~50ms.

    Bigger is more discriminating: 5 MB took ~13ms (below threshold);
    30 MB lands at ~50ms reliably; 50 MB at ~75ms.
    """
    return b"\x00\x00\x00\x01\x09" + b"\xA0" * (30 * 1024 * 1024)


def _huge_aac_buf() -> bytes:
    """~500 MB of pseudo-ADTS bytes — resync iterator scans for ~180ms.

    The resync scanner walks byte-by-byte; size dominates runtime.
    Below ~100 MB the workload is sub-discriminator (<50ms).
    """
    frame = bytes.fromhex("FFF150801FFC") + b"\x00" * (1024 - 6)
    return frame * 500_000


def _huge_mp2_buf() -> bytes:
    """~500 MB of pseudo MPEG-2 audio frames — ~150ms scan."""
    frame = b"\xFF\xFB\x90\x00" + b"\x00" * 1020
    return frame * 500_000




def _build_ts_stream(target_mb: int = 50) -> bytes:
    """Mux a ~target_mb MB TS stream by repeated push + drain."""
    cfg = _muxer_config_video_only()
    m = Muxer(cfg)
    nal = b"\x00\x00\x00\x01\x09" + b"\xA0" * (target_mb * 1024 * 1024)
    m.push_video(nal, pts=Pts90khz.from_raw(900_000))
    drain = bytearray(188 * 100_000)
    chunks: list[bytes] = []
    while True:
        n = m.pull(drain)
        if n == 0:
            break
        chunks.append(bytes(drain[:n]))
    return b"".join(chunks)


# ---------------------------------------------------------------------------
# Muxer.push_video — one huge NAL → single long Rust call
# ---------------------------------------------------------------------------


@pytest.mark.timeout(20)
def test_push_video_releases_gil(solo_throughput: float) -> None:
    """One push_video of a 30 MB NAL must let other Python threads run."""
    m = Muxer(_muxer_config_video_only())
    nal = _huge_h264_nal()

    with _PyWorker() as worker:
        m.push_video(nal, pts=Pts90khz.from_raw(900_000))

    _assert_gil_released(worker, solo_throughput, "push_video")


@pytest.mark.timeout(20)
def test_push_video_to_with_dts_releases_gil(solo_throughput: float) -> None:
    """push_video_to_with_dts on a 30 MB NAL must let other threads run.

    Covers the `_to_with_dts` variant which has the most complex
    signature (two PTS args + handle). If this one releases the GIL,
    the structurally simpler `push_video_to` / `push_audio_to` /
    `push_klv_to` variants do too (they use the same wrapper pattern).
    """
    m = Muxer(_muxer_config_video_only())
    # Need a video handle for the _to variant.
    handles = m.video_handles()
    assert len(handles) == 1
    handle = handles[0]
    nal = _huge_h264_nal()

    # Loop 3× so the total workload comfortably exceeds the 50ms _MIN_WORKLOAD_MS
    # sentinel on fast hardware — a single push_video_to_with_dts call clocked
    # at ~49ms on a Ryzen 9 7950X3D, just below the guard threshold.
    with _PyWorker() as worker:
        for i in range(3):
            m.push_video_to_with_dts(
                handle,
                nal,
                pts=Pts90khz.from_raw(900_000 + i * 90_000),
                dts=Pts90khz.from_raw(900_000 + i * 90_000),
            )

    _assert_gil_released(worker, solo_throughput, "push_video_to_with_dts")


# ---------------------------------------------------------------------------
# Demuxer.feed — repeated feeds of moderate-sized chunks
# ---------------------------------------------------------------------------


@pytest.mark.timeout(30)
def test_demuxer_feed_releases_gil(solo_throughput: float) -> None:
    """Repeated Demuxer.feed calls must let other threads run.

    Each feed call processes 500 KB (well under the 4 MB sync ceiling)
    so the demuxer doesn't error out. The cumulative work across the
    batch crosses the 50ms discriminator threshold.

    Note: with switchinterval-mediated yielding, the GIL is technically
    released between bytecode steps even without `allow_threads`. The
    `allow_threads` benefit here shows up as significantly higher
    worker throughput because the Rust portion of each feed call no
    longer blocks the worker for the full call duration.
    """
    ts_bytes = _build_ts_stream(target_mb=20)
    assert len(ts_bytes) > 5_000_000, (
        f"test setup: only got {len(ts_bytes)} bytes of TS data"
    )

    chunk_size = 500_000
    n_chunks = (len(ts_bytes) + chunk_size - 1) // chunk_size

    d = Demuxer()

    with _PyWorker() as worker:
        # Loop the stream until the workload is discriminating.
        for _ in range(15):
            for ci in range(n_chunks):
                start_off = ci * chunk_size
                end_off = min(start_off + chunk_size, len(ts_bytes))
                d.feed(ts_bytes[start_off:end_off])
                while d.next_event() is not None:
                    pass

    _assert_gil_released(worker, solo_throughput, "Demuxer.feed")


# ---------------------------------------------------------------------------
# Codec collect-then-iter — one huge eager collect
# ---------------------------------------------------------------------------


@pytest.mark.timeout(20)
def test_iter_aac_frames_with_resync_releases_gil(
    solo_throughput: float,
) -> None:
    """iter_aac_frames_with_resync on ~500MB must release the GIL.

    The Rust resync scanner walks the entire buffer byte-by-byte
    looking for sync patterns; on this scale it takes ~180ms (one
    single long Rust call).
    """
    buf = _huge_aac_buf()

    with _PyWorker() as worker:
        iter_aac_frames_with_resync(buf)

    _assert_gil_released(worker, solo_throughput, "iter_aac_frames_with_resync")


@pytest.mark.timeout(20)
def test_iter_mpeg2_audio_frames_with_resync_releases_gil(
    solo_throughput: float,
) -> None:
    """iter_mpeg2_audio_frames_with_resync on ~500MB must release the GIL."""
    buf = _huge_mp2_buf()

    with _PyWorker() as worker:
        iter_mpeg2_audio_frames_with_resync(buf)

    _assert_gil_released(
        worker, solo_throughput, "iter_mpeg2_audio_frames_with_resync"
    )


# ---------------------------------------------------------------------------
# KLV decode is intentionally NOT GIL-released — see klv.rs decision comments.
# Records are typically 20-200 bytes; per-call Rust work (~5us) is well below
# the GIL-transition breakeven point (~50us). Wrapping the small fast calls
# produced lock-contention pathology under tight batch loops in pre-ship
# benchmarks (30K decodes degraded from 0.5s baseline to 50+ seconds).
# ---------------------------------------------------------------------------
