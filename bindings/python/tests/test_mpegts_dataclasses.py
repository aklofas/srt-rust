"""Support dataclasses used by DemuxEvent — StreamId, StreamInfo,
KlvLink, ProgramMap."""

from tstrans.mpegts import (
    RawDescriptor,
    StreamId,
    StreamInfo,
    KlvLink,
    ProgramMap,
    StreamKindTag,
    VideoCodec,
    AudioCodec,
    LinkSource,
)


def test_stream_id_construction():
    sid = StreamId(pid=256, kind=StreamKindTag.VIDEO, codec=VideoCodec.H264, program_number=1)
    assert sid.pid == 256
    assert sid.kind is StreamKindTag.VIDEO
    assert sid.codec is VideoCodec.H264
    assert sid.program_number == 1


def test_stream_id_codec_none_for_klv():
    sid = StreamId(pid=257, kind=StreamKindTag.KLV_SYNC, codec=None, program_number=1)
    assert sid.codec is None


def test_stream_info_construction():
    info = StreamInfo(
        pid=256,
        stream_type=0x1B,  # H.264
        kind=StreamKindTag.VIDEO,
        codec=VideoCodec.H264,
        program_number=1,
    )
    assert info.stream_type == 0x1B


def test_klv_link_construction():
    link = KlvLink(klv_pid=257, video_pid=256, source=LinkSource.DECLARED)
    assert link.source is LinkSource.DECLARED


def test_program_map_construction_and_streams_list():
    pm = ProgramMap(
        program_number=1,
        pcr_pid=256,
        pmt_pid=0x100,
        streams=(
            StreamInfo(pid=256, stream_type=0x1B, kind=StreamKindTag.VIDEO,
                       codec=VideoCodec.H264, program_number=1),
            StreamInfo(pid=257, stream_type=0x06, kind=StreamKindTag.KLV_SYNC,
                       codec=None, program_number=1),
            StreamInfo(pid=258, stream_type=0x0F, kind=StreamKindTag.AUDIO,
                       codec=AudioCodec.AAC, program_number=1),
        ),
        klv_links=(KlvLink(klv_pid=257, video_pid=256, source=LinkSource.INFERRED),),
    )
    assert pm.program_number == 1
    assert pm.pmt_pid == 0x100
    assert len(pm.streams) == 3
    assert len(pm.klv_links) == 1


def test_dataclasses_equal_by_value():
    a = KlvLink(klv_pid=1, video_pid=2, source=LinkSource.DECLARED)
    b = KlvLink(klv_pid=1, video_pid=2, source=LinkSource.DECLARED)
    assert a == b


def test_dataclasses_hashable_when_frozen():
    # StreamId is hashable (frozen + hashable fields)
    sid = StreamId(pid=1, kind=StreamKindTag.VIDEO, codec=VideoCodec.H264, program_number=1)
    assert hash(sid) == hash(sid)
    # RawDescriptor and a descriptor-carrying StreamInfo are hashable too
    # (tuple-of-frozen + bytes fields).
    d = RawDescriptor(tag=0x05, data=b"KLVA")
    assert hash(d) == hash(d)
    info = StreamInfo(pid=2, stream_type=0x15, kind=StreamKindTag.KLV_SYNC,
                      codec=None, program_number=1, raw_descriptors=(d,))
    assert hash(info) == hash(info)


def test_program_map_repr_includes_program_number():
    pm = ProgramMap(program_number=42, pcr_pid=256, pmt_pid=0x100, streams=(), klv_links=())
    assert "42" in repr(pm)


def test_raw_descriptor_construction():
    d = RawDescriptor(tag=0x05, data=b"\x01\x02\x03")
    assert d.tag == 0x05
    assert d.data == b"\x01\x02\x03"


def test_stream_info_raw_descriptors_default_empty():
    info = StreamInfo(
        pid=256,
        stream_type=0x1B,
        kind=StreamKindTag.VIDEO,
        codec=VideoCodec.H264,
        program_number=1,
    )
    assert info.raw_descriptors == ()


def test_stream_info_raw_descriptors_explicit():
    d = RawDescriptor(tag=0x0A, data=b"eng")
    info = StreamInfo(
        pid=257,
        stream_type=0x0F,
        kind=StreamKindTag.AUDIO,
        codec=AudioCodec.AAC,
        program_number=1,
        raw_descriptors=(d,),
    )
    assert len(info.raw_descriptors) == 1
    assert info.raw_descriptors[0].tag == 0x0A
    assert info.raw_descriptors[0].data == b"eng"
