"""DemuxEvent base + subclass hierarchy. Subclasses are accessed as
attributes on the base class: `DemuxEvent.Video(...)`, etc."""

from tstrans.mpegts import (
    DemuxEvent,
    Pts90khz,
    StreamId,
    StreamKindTag,
    VideoCodec,
    AudioCodec,
    SubtitleCodec,
    MetadataKindTag,
    DiscontinuityKindTag,
    NonConformantKind,
    ProgramMap,
)


def _v_stream():
    return StreamId(pid=256, kind=StreamKindTag.VIDEO,
                    codec=VideoCodec.H264, program_number=1)


def test_program_map_event():
    pm = ProgramMap(program_number=1, pcr_pid=256, pmt_pid=0x100, streams=(), klv_links=())
    ev = DemuxEvent.ProgramMap(programs=(pm,))
    assert isinstance(ev, DemuxEvent)
    assert isinstance(ev, DemuxEvent.ProgramMap)
    assert ev.programs[0].program_number == 1


def test_video_event():
    ev = DemuxEvent.Video(
        stream=_v_stream(),
        pts=Pts90khz.from_ms(100),
        dts=None,
        codec=VideoCodec.H264,
        raw=b"\x00\x00\x00\x01fake nals",
        random_access_indicator=True,
    )
    assert isinstance(ev, DemuxEvent)
    assert ev.codec is VideoCodec.H264
    assert ev.random_access_indicator is True


def test_audio_event():
    # Raw-first: Audio event carries `raw=` (was `payload=` before the
    # raw-first rewire). Typed frames come from `ev.parse()`.
    ev = DemuxEvent.Audio(
        stream=StreamId(pid=258, kind=StreamKindTag.AUDIO,
                        codec=AudioCodec.AAC, program_number=1),
        pts=Pts90khz.from_ms(100),
        dts=None,
        codec=AudioCodec.AAC,
        raw=b"adts frame bytes",
    )
    assert isinstance(ev, DemuxEvent)
    assert ev.codec is AudioCodec.AAC


def test_subtitle_event():
    ev = DemuxEvent.Subtitle(
        stream=StreamId(pid=259, kind=StreamKindTag.SUBTITLE,
                        codec=SubtitleCodec.DVB_SUBTITLING, program_number=1),
        pts=Pts90khz.from_ms(100),
        dts=None,
        codec=SubtitleCodec.DVB_SUBTITLING,
        payload=b"subtitle data",
    )
    assert isinstance(ev, DemuxEvent)


def test_klv_event_sync_au_cell():
    ev = DemuxEvent.Metadata(
        stream=StreamId(pid=257, kind=StreamKindTag.KLV_SYNC,
                        codec=None, program_number=1),
        pts=Pts90khz.from_ms(100),
        kind=MetadataKindTag.KLV_SYNC_AU_CELL,
        payload=b"\x06\x0e\x2b\x34... klv bytes",
    )
    assert isinstance(ev, DemuxEvent)
    assert ev.kind is MetadataKindTag.KLV_SYNC_AU_CELL


def test_discontinuity_event():
    ev = DemuxEvent.Discontinuity(
        stream=_v_stream(),
        kind=DiscontinuityKindTag.CONTINUITY_JUMP,
    )
    assert isinstance(ev, DemuxEvent)


def test_nonconformant_event():
    ev = DemuxEvent.NonConformant(
        stream=_v_stream(),
        issue="PCR anomaly: delta=12345",
        kind=NonConformantKind.PCR_ANOMALY,
    )
    assert isinstance(ev, DemuxEvent)
    assert "PCR" in ev.issue


def test_reconnect_discontinuity_singleton():
    ev = DemuxEvent.ReconnectDiscontinuity()
    assert isinstance(ev, DemuxEvent)
    assert isinstance(ev, DemuxEvent.ReconnectDiscontinuity)


def test_subclasses_are_distinct_types():
    pm = DemuxEvent.ProgramMap(programs=())
    rec = DemuxEvent.ReconnectDiscontinuity()
    assert not isinstance(pm, DemuxEvent.ReconnectDiscontinuity)
    assert not isinstance(rec, DemuxEvent.ProgramMap)


def test_video_event_match_statement_310_plus():
    ev = DemuxEvent.Video(
        stream=_v_stream(),
        pts=Pts90khz.from_ms(100),
        dts=None,
        codec=VideoCodec.H264,
        raw=b"x",
        random_access_indicator=False,
    )
    result = []
    src = (
        "match ev:\n"
        "    case DemuxEvent.Video(codec=c):\n"
        "        result.append(c)\n"
        "    case _:\n"
        "        result.append('no-match')\n"
    )
    exec(src, {"ev": ev, "DemuxEvent": DemuxEvent, "result": result})
    assert result == [VideoCodec.H264]
