"""MuxerFileSink + drain proxy tests."""

import tempfile
from pathlib import Path

import pytest

from tstrans.mpegts import (
    KlvStreamType,
    Muxer,
    MuxerConfigBuilder,
    MuxerProgramConfigBuilder,
    Pts90khz,
    VideoCodec,
)


def _cfg():
    prog = (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .build()
    )
    return MuxerConfigBuilder().add_program(prog).build()


def _nal_aud() -> bytes:
    return b"\x00\x00\x00\x01\x09\xF0"


def test_write_file_creates_non_empty_file():
    m = Muxer(_cfg())
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "out.ts"
        with m.write_file(path) as proxy:
            proxy.push_video(_nal_aud(), Pts90khz.from_raw(900_000))
        assert path.stat().st_size > 0
        assert path.stat().st_size % 188 == 0


def test_write_file_drains_on_exit():
    m = Muxer(_cfg())
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "out.ts"
        with m.write_file(path) as proxy:
            for i in range(5):
                proxy.push_video(_nal_aud(), Pts90khz.from_raw(900_000 + i * 3000))
        # After __exit__, all pending must be drained
        assert m.pending_packets() == 0


def test_write_file_propagates_user_exception():
    m = Muxer(_cfg())
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "out.ts"
        with pytest.raises(RuntimeError, match="boom"):
            with m.write_file(path) as proxy:
                proxy.push_video(_nal_aud(), Pts90khz.from_raw(900_000))
                raise RuntimeError("boom")
        # File still flushed + closed despite the exception
        assert path.stat().st_size > 0


def test_proxy_delegates_read_only_methods():
    m = Muxer(_cfg())
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "out.ts"
        with m.write_file(path) as proxy:
            assert proxy.pending_packets() >= 0
            assert isinstance(proxy.video_handles(), list)


def test_write_file_accepts_str_and_pathlike():
    m1 = Muxer(_cfg())
    m2 = Muxer(_cfg())
    with tempfile.TemporaryDirectory() as tmp:
        p1 = Path(tmp) / "a.ts"
        with m1.write_file(str(p1)) as proxy:
            proxy.push_video(_nal_aud(), Pts90khz.from_raw(900_000))
        p2 = Path(tmp) / "b.ts"
        with m2.write_file(p2) as proxy:
            proxy.push_video(_nal_aud(), Pts90khz.from_raw(900_000))
        assert p1.stat().st_size > 0
        assert p2.stat().st_size > 0


def test_muxer_reusable_after_sink_exit():
    m = Muxer(_cfg())
    with tempfile.TemporaryDirectory() as tmp:
        path1 = Path(tmp) / "a.ts"
        with m.write_file(path1) as proxy:
            proxy.push_video(_nal_aud(), Pts90khz.from_raw(900_000))
        path2 = Path(tmp) / "b.ts"
        with m.write_file(path2) as proxy:
            proxy.push_video(_nal_aud(), Pts90khz.from_raw(1_800_000))
        assert path1.stat().st_size > 0
        assert path2.stat().st_size > 0


def test_write_file_no_pushes_writes_initial_psi():
    m = Muxer(_cfg())
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "out.ts"
        with m.write_file(path):
            pass
        assert path.exists()


# audit #13 — atomic-write opt-in via `Muxer.write_file(path, atomic=True)`.


class _MyTestError(Exception):
    """Sentinel exception used to trigger the exception-exit branch."""


def test_atomic_false_exception_leaves_partial_at_dest(tmp_path):
    # Default (atomic=False): exception inside `with` still flushes +
    # closes the destination file, leaving a partial TS at the user's
    # path. This documents the existing non-atomic contract.
    m = Muxer(_cfg())
    path = tmp_path / "out.ts"
    with pytest.raises(_MyTestError):
        with m.write_file(path) as proxy:
            proxy.push_video(_nal_aud(), Pts90khz.from_raw(900_000))
            raise _MyTestError()
    assert path.exists()
    assert path.stat().st_size > 0


def test_atomic_true_exception_no_file_at_dest(tmp_path):
    # atomic=True: exception inside `with` discards the tempfile and
    # leaves nothing at the destination path.
    m = Muxer(_cfg())
    path = tmp_path / "out.ts"
    with pytest.raises(_MyTestError):
        with m.write_file(path, atomic=True) as proxy:
            proxy.push_video(_nal_aud(), Pts90khz.from_raw(900_000))
            raise _MyTestError()
    assert not path.exists()
    # No `.partial` tempfile should remain in tmp_path either.
    assert list(tmp_path.glob("*.partial")) == []


def test_atomic_true_clean_exit_file_at_dest(tmp_path):
    # atomic=True happy path: file appears at destination on success,
    # tempfile is renamed away (no `.partial` leftover).
    m = Muxer(_cfg())
    path = tmp_path / "out.ts"
    with m.write_file(path, atomic=True) as proxy:
        proxy.push_video(_nal_aud(), Pts90khz.from_raw(900_000))
    assert path.exists()
    assert path.stat().st_size > 0
    assert path.stat().st_size % 188 == 0
    assert list(tmp_path.glob("*.partial")) == []


def test_atomic_kwarg_default_is_false(tmp_path):
    # No kwarg → default-False behavior matches existing
    # `test_write_file_propagates_user_exception` (partial file at
    # destination after exception). Regression guard against an
    # accidental default flip.
    m = Muxer(_cfg())
    path = tmp_path / "out.ts"
    with pytest.raises(_MyTestError):
        with m.write_file(path) as proxy:
            proxy.push_video(_nal_aud(), Pts90khz.from_raw(900_000))
            raise _MyTestError()
    assert path.exists()
