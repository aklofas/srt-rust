"""Tests for codec.split_units and codec.parse_audio opt-in parsers (Task 4.1)."""

import pytest
import tstrans.codec as codec
from tstrans.exceptions import CodecError
from tstrans.mpegts import AudioCodec, VideoCodec


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
