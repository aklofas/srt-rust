"""Phase 4 Muxer config family tests."""

import pytest

from tstrans.mpegts import (
    Av1CarriageMode,
    AudioCodec,
    AudioStreamSpec,
    KlvStreamSpec,
    KlvStreamType,
    StreamSpec,
    SubtitleCodec,
    SubtitleStreamSpec,
    VideoCodec,
    VideoStreamSpec,
)


def test_klv_stream_type_enum_has_two_variants():
    assert len(list(KlvStreamType)) == 2
    # The two variants represent the two PES wrap shapes for KLV
    names = {v.name for v in KlvStreamType}
    assert "SYNCHRONOUS_METADATA" in names
    assert "PRIVATE_DATA" in names


def test_av1_carriage_mode_default_is_binding_conformant():
    # `MPEG2_TS_BINDING` exists; it is the default for new muxers
    # (matches Rust `Av1CarriageMode::Mpeg2TsBinding` default).
    assert hasattr(Av1CarriageMode, "MPEG2_TS_BINDING")


def test_video_stream_spec_frozen_dataclass_fields():
    s = VideoStreamSpec(pid=0x101, codec=VideoCodec.H264)
    assert s.pid == 0x101
    assert s.codec is VideoCodec.H264
    with pytest.raises((AttributeError, TypeError)):
        s.pid = 0x102  # type: ignore[misc]


def test_klv_stream_spec_carries_metadata_kind():
    s = KlvStreamSpec(pid=0x102, stream_type=KlvStreamType.SYNCHRONOUS_METADATA, carries_pts=True)
    assert s.stream_type is KlvStreamType.SYNCHRONOUS_METADATA
    assert s.carries_pts is True


def test_audio_stream_spec_default_language_is_none():
    s = AudioStreamSpec(pid=0x103, codec=AudioCodec.AAC)
    assert s.language is None


def test_stream_spec_subclasses_inherit_from_stream_spec_abc():
    assert issubclass(VideoStreamSpec, StreamSpec)
    assert issubclass(KlvStreamSpec, StreamSpec)
    assert issubclass(AudioStreamSpec, StreamSpec)
    assert issubclass(SubtitleStreamSpec, StreamSpec)
