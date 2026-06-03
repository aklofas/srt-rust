"""Phase 5: end-to-end Sample.payload typed access.

Video events now carry `list[NalUnit]` (H.264/H.265/H.266) or `list[Obu]`
(AV1) instead of raw bytes. Audio events carry `list[AdtsFrame]` (AAC) or
`list[Mpeg2AudioFrame]` (MP2) or bytes (LATM / AC-3 / fallback-on-error).

These tests use:
- Muxer-generated synthetic TS for video (H.264 NAL AUD round-trip).
- Real audio fixtures from crates/tst-core/tests/fixtures/audio/ for AAC + MP2.
"""

import tempfile
from pathlib import Path

import pytest

from tstrans.codec import AdtsFrame, Mpeg2AudioFrame, NalUnit, Obu
from tstrans.io import parse_file
from tstrans.mpegts import (
    DemuxEvent,
    KlvStreamType,
    Muxer,
    MuxerConfigBuilder,
    MuxerProgramConfigBuilder,
    Pts90khz,
    VideoCodec,
)

# ---------------------------------------------------------------------------
# Fixtures and helpers
# ---------------------------------------------------------------------------

_FIXTURE_BASE = (
    Path(__file__).parent.parent.parent.parent / "crates" / "tst-core" / "tests" / "fixtures"
)
_AAC_ADTS_FIXTURE = _FIXTURE_BASE / "audio" / "aac-adts.ts"
_MP2_FIXTURE = _FIXTURE_BASE / "audio" / "mp2.ts"

# Phase 1 skip-closure (2026-05-25): all four audio fixtures are checked into
# crates/tst-core/tests/fixtures/audio/. A missing file is a packaging bug,
# not a runtime skip condition.
for _fx in (_AAC_ADTS_FIXTURE, _MP2_FIXTURE):
    assert _fx.is_file(), f"checked-in audio fixture missing: {_fx}"


def _make_h264_ts(tmp: Path) -> Path:
    """Write a small H.264 TS into *tmp* via the Muxer and return the path."""
    prog = (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .build()
    )
    cfg = MuxerConfigBuilder().add_program(prog).build()
    m = Muxer(cfg)
    path = tmp / "h264.ts"
    # H.264 Access Unit Delimiter NAL (AUD): Annex-B, nal_unit_type=9.
    nal_aud = b"\x00\x00\x00\x01\x09\xF0"
    pts0 = 900_000  # ~10 s at 90 kHz
    with m.write_file(path) as proxy:
        for i in range(4):
            proxy.push_video(nal_aud, pts=Pts90khz.from_raw(pts0 + i * 3000))
    return path


# ---------------------------------------------------------------------------
# Video: H.264 → list[NalUnit]
# ---------------------------------------------------------------------------

def test_h264_video_payload_is_list():
    """parse_file on a synthetic H.264 TS must yield Video events whose
    payload is a list (not bytes)."""
    with tempfile.TemporaryDirectory() as tmp:
        ts_path = _make_h264_ts(Path(tmp))
        events = list(parse_file(ts_path))
    video = [e for e in events if isinstance(e, DemuxEvent.Video)]
    assert video, "expected at least one DemuxEvent.Video from synthetic H.264 TS"
    s = video[0]
    assert isinstance(s.payload, list), (
        f"expected list, got {type(s.payload).__name__}"
    )


def test_h264_video_payload_elements_are_nal_units():
    """Every element in Video.payload must be a NalUnit instance."""
    with tempfile.TemporaryDirectory() as tmp:
        ts_path = _make_h264_ts(Path(tmp))
        events = list(parse_file(ts_path))
    video = [e for e in events if isinstance(e, DemuxEvent.Video)]
    assert video
    s = video[0]
    assert all(isinstance(n, NalUnit) for n in s.payload), (
        "expected all elements to be NalUnit"
    )


def test_h264_nal_unit_kind_is_h264():
    """NalUnit.kind for H.264 stream must equal 'H264'."""
    with tempfile.TemporaryDirectory() as tmp:
        ts_path = _make_h264_ts(Path(tmp))
        events = list(parse_file(ts_path))
    video = [e for e in events if isinstance(e, DemuxEvent.Video)]
    assert video
    s = video[0]
    assert len(s.payload) > 0
    assert s.payload[0].kind == "H264", f"got kind={s.payload[0].kind!r}"


def test_h264_video_codec_parse_error_is_none():
    """codec_parse_error must be None for video (typed-parse can't fail here)."""
    with tempfile.TemporaryDirectory() as tmp:
        ts_path = _make_h264_ts(Path(tmp))
        events = list(parse_file(ts_path))
    video = [e for e in events if isinstance(e, DemuxEvent.Video)]
    assert video
    assert video[0].codec_parse_error is None


# ---------------------------------------------------------------------------
# Audio: AAC-ADTS → list[AdtsFrame]
# ---------------------------------------------------------------------------

def test_aac_audio_payload_is_list():
    """AAC ADTS events from a real fixture must carry list[AdtsFrame]."""
    aac_events = [
        e for e in parse_file(_AAC_ADTS_FIXTURE)
        if isinstance(e, DemuxEvent.Audio)
        and e.codec.value == "aac"
    ]
    assert aac_events, f"no AAC audio events found in {_AAC_ADTS_FIXTURE}"
    s = aac_events[0]
    assert isinstance(s.payload, list), (
        f"expected list, got {type(s.payload).__name__}"
    )


def test_aac_audio_payload_elements_are_adts_frames():
    """Every element in an AAC Audio.payload must be AdtsFrame."""
    aac_events = [
        e for e in parse_file(_AAC_ADTS_FIXTURE)
        if isinstance(e, DemuxEvent.Audio)
        and e.codec.value == "aac"
    ]
    assert aac_events
    s = aac_events[0]
    # payload may be empty list if AAC is parsed as zero frames, but
    # elements must be typed correctly.
    assert all(isinstance(f, AdtsFrame) for f in s.payload), (
        "expected all elements to be AdtsFrame"
    )


def test_aac_codec_parse_error_is_none_on_clean_stream():
    """codec_parse_error must be None for a clean ADTS stream."""
    aac_events = [
        e for e in parse_file(_AAC_ADTS_FIXTURE)
        if isinstance(e, DemuxEvent.Audio)
        and e.codec.value == "aac"
    ]
    assert aac_events
    # At least the first event on a well-formed fixture must parse cleanly.
    assert aac_events[0].codec_parse_error is None


# ---------------------------------------------------------------------------
# Audio: MP2 → list[Mpeg2AudioFrame]
# ---------------------------------------------------------------------------

def test_mp2_audio_payload_is_list():
    """MP2 audio events from a real fixture must carry list[Mpeg2AudioFrame]."""
    mp2_events = [
        e for e in parse_file(_MP2_FIXTURE)
        if isinstance(e, DemuxEvent.Audio)
        and e.codec.value == "mp2"
    ]
    assert mp2_events, f"no MP2 audio events found in {_MP2_FIXTURE}"
    s = mp2_events[0]
    assert isinstance(s.payload, list), (
        f"expected list, got {type(s.payload).__name__}"
    )


def test_mp2_audio_payload_elements_are_mpeg2_audio_frames():
    """Every element in an MP2 Audio.payload must be Mpeg2AudioFrame."""
    mp2_events = [
        e for e in parse_file(_MP2_FIXTURE)
        if isinstance(e, DemuxEvent.Audio)
        and e.codec.value == "mp2"
    ]
    assert mp2_events
    s = mp2_events[0]
    assert all(isinstance(f, Mpeg2AudioFrame) for f in s.payload), (
        "expected all elements to be Mpeg2AudioFrame"
    )


def test_mp2_codec_parse_error_is_none_on_clean_stream():
    """codec_parse_error must be None for a clean MP2 stream."""
    mp2_events = [
        e for e in parse_file(_MP2_FIXTURE)
        if isinstance(e, DemuxEvent.Audio)
        and e.codec.value == "mp2"
    ]
    assert mp2_events
    assert mp2_events[0].codec_parse_error is None


# ---------------------------------------------------------------------------
# Bytes fallback: LATM / AC-3 → raw bytes (typed parsing deferred)
# ---------------------------------------------------------------------------

_AAC_LATM_FIXTURE = _FIXTURE_BASE / "audio" / "aac-latm.ts"
_AC3_FIXTURE = _FIXTURE_BASE / "audio" / "ac3.ts"

for _fx in (_AAC_LATM_FIXTURE, _AC3_FIXTURE):
    assert _fx.is_file(), f"checked-in audio fixture missing: {_fx}"


def test_aac_latm_payload_is_bytes_fallback():
    """AAC-LATM typed parsing is deferred — payload must be raw bytes."""
    latm_events = [
        e for e in parse_file(_AAC_LATM_FIXTURE)
        if isinstance(e, DemuxEvent.Audio)
        and e.codec.value == "aac_latm"
    ]
    assert latm_events, f"no AAC-LATM events found in {_AAC_LATM_FIXTURE}"
    s = latm_events[0]
    assert isinstance(s.payload, bytes), (
        f"expected bytes for LATM fallback, got {type(s.payload).__name__}"
    )
    assert s.codec_parse_error is None, "no error expected for intentional bytes fallback"


def test_ac3_payload_is_bytes_fallback():
    """AC-3 typed parsing is not yet implemented — payload must be raw bytes."""
    ac3_events = [
        e for e in parse_file(_AC3_FIXTURE)
        if isinstance(e, DemuxEvent.Audio)
        and e.codec.value == "ac3"
    ]
    assert ac3_events, f"no AC-3 events found in {_AC3_FIXTURE}"
    s = ac3_events[0]
    assert isinstance(s.payload, bytes), (
        f"expected bytes for AC-3 fallback, got {type(s.payload).__name__}"
    )
    assert s.codec_parse_error is None


# ---------------------------------------------------------------------------
# Regression: codec_parse_error attribute exists on all Sample event types
# ---------------------------------------------------------------------------

def test_video_event_has_codec_parse_error_attribute():
    """DemuxEvent.Video must have a codec_parse_error attribute (may be None)."""
    with tempfile.TemporaryDirectory() as tmp:
        ts_path = _make_h264_ts(Path(tmp))
        events = list(parse_file(ts_path))
    video = [e for e in events if isinstance(e, DemuxEvent.Video)]
    assert video
    assert hasattr(video[0], "codec_parse_error")
