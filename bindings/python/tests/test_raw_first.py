"""Tests for codec.split_units and codec.parse_audio opt-in parsers (Task 4.1)
plus the DemuxEvent.Video/.Audio raw-first surface (Task 4.2)."""

import tempfile
from pathlib import Path

import pytest

import tstrans.codec as codec
from tstrans import io as tio
from tstrans.exceptions import CodecError
from tstrans.mpegts import (
    AudioCodec,
    DemuxEvent,
    Muxer,
    MuxerConfigBuilder,
    MuxerProgramConfigBuilder,
    Pts90khz,
    VideoCodec,
)


def test_split_units_h264_returns_nal_list():
    # Two H.264 NALs: SPS (nal_ref_idc=3, type=7) then IDR (type=5), 4-byte start codes.
    # NAL header byte 0x67 = (ref_idc=3 << 5) | (type=7); payload bytes: 0xAA, 0xBB
    # NAL header byte 0x65 = (ref_idc=3 << 5) | (type=5); payload bytes: 0xCC
    au = b"\x00\x00\x00\x01\x67\xAA\xBB\x00\x00\x00\x01\x65\xCC"
    units, issues = codec.split_units(au, VideoCodec.H264)
    assert len(units) == 2
    assert issues == []
    assert bytes(units[0].payload) == b"\xAA\xBB"


def test_split_units_strict_raises_on_bad_header():
    # forbidden_zero_bit set (0x80) → a NAL-header issue.
    au = b"\x00\x00\x00\x01\x80\x00"
    with pytest.raises(ValueError):
        codec.split_units(au, VideoCodec.H264, strict=True)


def test_split_units_lenient_does_not_raise_on_bad_header():
    # Lenient mode: split_units returns (units, issues) — the 0x80 forbidden-bit
    # input provably yields a conformance issue rather than raising.
    au = b"\x00\x00\x00\x01\x80\x00"
    units, issues = codec.split_units(au, VideoCodec.H264, strict=False)
    assert isinstance(units, list)
    assert isinstance(issues, list)
    assert len(issues) > 0


def test_parse_audio_aac_empty_returns_empty():
    frames = codec.parse_audio(b"", AudioCodec.AAC)
    assert frames == []


def test_parse_audio_mp2_empty_returns_empty():
    frames = codec.parse_audio(b"", AudioCodec.MP2)
    assert frames == []


def test_parse_audio_unknown_codec_returns_empty():
    # AAC_LATM has no typed parser — returns empty list.
    frames = codec.parse_audio(b"\xff\xff\xff", AudioCodec.AAC_LATM)
    assert frames == []


def test_parse_audio_aac_strict_raises_on_malformed():
    # An ADTS syncword (0xFFF1) followed by a truncated header raises under
    # strict mode (CodecError, the codec-domain exception — not ValueError).
    with pytest.raises(CodecError):
        codec.parse_audio(b"\xff\xf1\xff", AudioCodec.AAC, strict=True)


# ---------------------------------------------------------------------------
# Task 4.2 — DemuxEvent.Video/.Audio raw-first surface
# ---------------------------------------------------------------------------

# Real audio fixtures live under the tst-core fixtures tree (same accessor
# pattern as tests/test_sample_payload_typed.py).
_FIXTURE_BASE = (
    Path(__file__).parent.parent.parent.parent
    / "crates" / "tst-core" / "tests" / "fixtures"
)
_MP2_FIXTURE = _FIXTURE_BASE / "audio" / "mp2.ts"


def _sample_ts_path(tmp: Path) -> Path:
    """Build a small H.264 TS via the Muxer (mirrors the synthetic-TS
    accessor used by tests/test_sample_payload_typed.py::_make_h264_ts)."""
    prog = (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .build()
    )
    cfg = MuxerConfigBuilder().add_program(prog).build()
    m = Muxer(cfg)
    path = tmp / "h264.ts"
    nal_aud = b"\x00\x00\x00\x01\x09\xF0"  # Annex-B AUD NAL
    pts0 = 900_000
    with m.write_file(path) as proxy:
        for i in range(4):
            proxy.push_video(nal_aud, pts=Pts90khz.from_raw(pts0 + i * 3000))
    return path


def test_video_event_exposes_raw_and_opt_in_parse():
    saw = False
    with tempfile.TemporaryDirectory() as tmp:
        for ev in tio.parse_file(_sample_ts_path(Path(tmp))):
            if isinstance(ev, DemuxEvent.Video):
                assert isinstance(ev.raw, (bytes, bytearray))
                assert ev.raw[:4] == b"\x00\x00\x00\x01"  # Annex-B start code
                units = ev.parse()                          # opt-in
                assert len(units) >= 1
                assert not hasattr(ev, "payload")           # removed
                saw = True
                break
    assert saw


def test_audio_event_exposes_raw():
    for ev in tio.parse_file(_MP2_FIXTURE):
        if isinstance(ev, DemuxEvent.Audio):
            assert isinstance(ev.raw, (bytes, bytearray))
            assert isinstance(ev.parse(), list)
            break


# ---------------------------------------------------------------------------
# Task 4.3 — push_video_* accept dts=None (PTS-only PES, == push_video_to)
# ---------------------------------------------------------------------------

def _drain(mux) -> bytes:
    """Drain all queued TS packets from a Muxer into a single bytes blob."""
    out = bytearray()
    while True:
        buf = bytearray(1316)
        n = mux.pull(buf)
        if n == 0:
            break
        out += bytes(buf[:n])
    return bytes(out)


def _single_video_mux():
    prog = (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .pcr_pid(0x101)
        .build()
    )
    cfg = MuxerConfigBuilder().add_program(prog).build()
    return Muxer(cfg)


def test_push_video_accepts_dts_none():
    """Passing dts=None to push_video_to_with_dts produces a PTS-only PES.
    The muxer must accept it without error and produce TS output."""
    mux = _single_video_mux()
    vh = mux.video_stream_handle(0)
    # Valid H.264 Annex-B IDR NAL (IDR slice, nal_unit_type=5).
    au = b"\x00\x00\x00\x01\x65\x88\x84\x00\x10"
    pts = Pts90khz.from_raw(9000)
    mux.push_video_to_with_dts(vh, au, pts=pts, dts=None, key_frame=True)
    buf = bytearray(1316)
    assert mux.pull(buf) > 0


def test_push_video_dts_none_equals_push_video_to():
    """dts=None routes to the PTS-only path: byte-identical to push_video_to
    for the same AU + pts (pins the 5-byte PtsOnly PES, not a 10-byte PtsAndDts
    header with dts==pts)."""
    au = b"\x00\x00\x00\x01\x65\x88\x84\x00\x10"
    pts = Pts90khz.from_raw(9000)

    mux_a = _single_video_mux()
    mux_a.push_video_to(mux_a.video_stream_handle(0), au, pts=pts, key_frame=True)
    ref = _drain(mux_a)

    mux_b = _single_video_mux()
    mux_b.push_video_to_with_dts(
        mux_b.video_stream_handle(0), au, pts=pts, dts=None, key_frame=True
    )
    got = _drain(mux_b)

    assert len(ref) > 0
    assert got == ref
