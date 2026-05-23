"""Phase 4 Muxer push + drain tests."""

import pytest

from tstrans.exceptions import MuxError, MuxErrorKind
from tstrans.mpegts import (
    AudioCodec,
    KlvStreamType,
    Muxer,
    MuxerConfig,
    MuxerConfigBuilder,
    MuxerProgramConfigBuilder,
    Pts90khz,
    VideoCodec,
)


def _simple_config() -> MuxerConfig:
    prog = (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .add_audio(0x102, AudioCodec.AAC)
        .add_klv(0x103, KlvStreamType.SYNCHRONOUS_METADATA, carries_pts=True)
        .build()
    )
    return MuxerConfigBuilder().add_program(prog).build()


def test_muxer_constructs_with_valid_config():
    m = Muxer(_simple_config())
    assert isinstance(m, Muxer)


def test_muxer_initial_pending_is_non_negative():
    m = Muxer(_simple_config())
    # Initial state may pre-emit PAT/PMT; either 0 or a small number is OK
    assert m.pending_packets() >= 0


def test_pull_with_empty_bytearray_returns_zero():
    m = Muxer(_simple_config())
    buf = bytearray(0)
    n = m.pull(buf)
    assert n == 0


def test_pull_with_small_buffer_pre_push_returns_zero():
    # No data yet — pull returns 0 regardless.
    m = Muxer(_simple_config())
    buf = bytearray(50)
    n = m.pull(buf)
    assert n == 0


def test_capacity_packets_is_positive():
    m = Muxer(_simple_config())
    assert m.capacity_packets() > 0
