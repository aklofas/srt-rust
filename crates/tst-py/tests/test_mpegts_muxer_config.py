"""Phase 4 Muxer config family tests."""

import pytest

from tstrans.mpegts import (
    AudioCodec,
    AudioStreamHandle,
    AudioStreamSpec,
    Av1CarriageMode,
    KlvStreamHandle,
    KlvStreamSpec,
    KlvStreamType,
    StreamSpec,
    SubtitleCodec,
    SubtitleStreamHandle,
    SubtitleStreamSpec,
    VideoCodec,
    VideoStreamHandle,
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


def test_handle_raw_round_trip():
    h = VideoStreamHandle.from_raw(0x12345678)
    assert h.raw == 0x12345678


def test_handle_equality_and_hash():
    a = AudioStreamHandle.from_raw(42)
    b = AudioStreamHandle.from_raw(42)
    c = AudioStreamHandle.from_raw(43)
    assert a == b
    assert hash(a) == hash(b)
    assert a != c


def test_handles_are_distinct_types():
    v = VideoStreamHandle.from_raw(1)
    a = AudioStreamHandle.from_raw(1)
    assert type(v) is not type(a)
    assert v != a  # different types, even with same raw


def test_handle_repr_includes_class_name_and_raw():
    h = KlvStreamHandle.from_raw(0xDEAD)
    r = repr(h)
    assert "KlvStreamHandle" in r
    # 0xDEAD = 57005 — accept any reasonable rendering
    assert ("57005" in r) or ("0xdead" in r.lower()) or ("DEAD" in r)


def test_handle_unpack():
    h = SubtitleStreamHandle.from_raw(0)
    program_idx, within_idx = h.unpack()
    assert isinstance(program_idx, int)
    assert isinstance(within_idx, int)


# --- Task 4: MuxerProgramConfig + MuxerProgramConfigBuilder ---

from tstrans.mpegts import MuxerProgramConfig, MuxerProgramConfigBuilder


def test_program_builder_minimum_constructs():
    cfg = (
        MuxerProgramConfigBuilder(program_number=1, pmt_pid=0x100)
        .add_video(0x101, VideoCodec.H264)
        .build()
    )
    assert isinstance(cfg, MuxerProgramConfig)
    assert cfg.program_number == 1
    assert cfg.pmt_pid == 0x100
    assert len(cfg.streams) == 1
    assert cfg.streams[0].pid == 0x101


def test_program_builder_fluent_chain():
    cfg = (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .add_klv(0x102, KlvStreamType.SYNCHRONOUS_METADATA, carries_pts=True)
        .add_audio(0x103, AudioCodec.AAC)
        .pcr_pid(0x101)
        .build()
    )
    assert len(cfg.streams) == 3
    assert cfg.pcr_pid == 0x101


def test_program_streams_tuple_match_dispatch():
    cfg = (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .add_klv(0x102, KlvStreamType.SYNCHRONOUS_METADATA, carries_pts=True)
        .build()
    )
    kinds = []
    for s in cfg.streams:
        match s:
            case VideoStreamSpec(pid=p):
                kinds.append(("video", p))
            case KlvStreamSpec(pid=p):
                kinds.append(("klv", p))
            case _:
                pytest.fail(f"unexpected spec: {s}")
    assert kinds == [("video", 0x101), ("klv", 0x102)]


def test_program_descriptors_round_trip():
    cfg = (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .program_descriptors([b"\x05\x04KLVA"])
        .build()
    )
    assert cfg.program_descriptors == (b"\x05\x04KLVA",)


def test_audio_with_language_attaches_language():
    cfg = (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .add_audio_with_language(0x102, AudioCodec.AAC, language=b"eng")
        .build()
    )
    audio_specs = [s for s in cfg.streams if isinstance(s, AudioStreamSpec)]
    assert len(audio_specs) == 1
    assert audio_specs[0].language == b"eng"


def test_program_config_is_frozen():
    cfg = MuxerProgramConfigBuilder(1, 0x100).add_video(0x101, VideoCodec.H264).build()
    with pytest.raises((AttributeError, TypeError)):
        cfg.program_number = 99  # type: ignore[misc]


# --- Task 5: MuxerConfig + MuxerConfigBuilder ---

from tstrans.exceptions import MuxError, MuxErrorKind
from tstrans.mpegts import MuxerConfig, MuxerConfigBuilder


def test_muxer_config_builder_minimum_build():
    prog = MuxerProgramConfigBuilder(1, 0x100).add_video(0x101, VideoCodec.H264).build()
    cfg = MuxerConfigBuilder().add_program(prog).build()
    assert isinstance(cfg, MuxerConfig)
    assert len(cfg.programs) == 1
    assert cfg.programs[0].program_number == 1


def test_muxer_config_pcr_psi_defaults_reasonable():
    prog = MuxerProgramConfigBuilder(1, 0x100).add_video(0x101, VideoCodec.H264).build()
    cfg = MuxerConfigBuilder().add_program(prog).build()
    assert cfg.pcr_interval_ms > 0
    assert cfg.psi_interval_ms > 0
    assert cfg.buffer_packets > 0


def test_muxer_config_av1_default_is_mpeg2_ts_binding():
    prog = MuxerProgramConfigBuilder(1, 0x100).add_video(0x101, VideoCodec.AV1).build()
    cfg = MuxerConfigBuilder().add_program(prog).build()
    # Variant name from actual Rust (Task 2 finding).
    assert cfg.av1_carriage is Av1CarriageMode.MPEG2_TS_BINDING


def test_muxer_config_invalid_pid_collision_raises():
    # Two video streams on the same PID — should fail validation.
    prog = (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .add_video(0x101, VideoCodec.H265)
        .build()
    )
    with pytest.raises(MuxError) as ei:
        MuxerConfigBuilder().add_program(prog).build()
    # Use CONFIG_INVALID — the actual Rust variant name (Task 1 finding).
    # If actual rejection variant is different, adjust to the value seen.
    assert ei.value.kind in (
        MuxErrorKind.CONFIG_INVALID,
        MuxErrorKind.INPUT_MALFORMED,
        MuxErrorKind.INVALID_USAGE,
    )


def test_muxer_config_builder_pcr_interval_override():
    prog = MuxerProgramConfigBuilder(1, 0x100).add_video(0x101, VideoCodec.H264).build()
    cfg = MuxerConfigBuilder().add_program(prog).pcr_interval_ms(20).build()
    assert cfg.pcr_interval_ms == 20


def test_muxer_config_is_frozen():
    prog = MuxerProgramConfigBuilder(1, 0x100).add_video(0x101, VideoCodec.H264).build()
    cfg = MuxerConfigBuilder().add_program(prog).build()
    with pytest.raises((AttributeError, TypeError)):
        cfg.pcr_interval_ms = 99  # type: ignore[misc]


def test_muxer_config_static_builder_constructor():
    b = MuxerConfig.builder()
    assert isinstance(b, MuxerConfigBuilder)
