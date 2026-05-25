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


# ---------------------------------------------------------------------------
# Task 7 — push_video / push_video_to / push_video_to_with_dts
# ---------------------------------------------------------------------------


def _minimal_h264_nal_aud() -> bytes:
    """Annex B Access Unit Delimiter NAL — smallest valid H.264 NAL.

    `00 00 00 01` Annex-B 4-byte start code, then `09` (nal_unit_type=9
    AUD) and `F0` (primary_pic_type=7, trailing rbsp_trailing_bits).
    Two body bytes satisfy `validate_annex_b`'s "non-empty NAL body"
    rule and is small enough to fit in a single TS packet.
    """
    return b"\x00\x00\x00\x01\x09\xF0"


def test_push_video_single_target_form_increments_pending():
    m = Muxer(_simple_config())
    before = m.pending_packets()
    m.push_video(_minimal_h264_nal_aud(), pts=Pts90khz.from_raw(900_000))
    assert m.pending_packets() > before


def test_push_video_invalid_nal_raises_input_malformed():
    m = Muxer(_simple_config())
    with pytest.raises(MuxError) as ei:
        # No Annex-B start code — should fail validate_annex_b.
        m.push_video(b"\xDE\xAD\xBE\xEF", pts=Pts90khz.from_raw(900_000))
    assert ei.value.kind == MuxErrorKind.INPUT_MALFORMED


def test_push_video_to_with_invalid_handle_raises_invalid_usage():
    """An out-of-range handle surfaces as InvalidStreamHandle →
    INVALID_USAGE per the MuxSenderErrorKind classifier in
    tst-core/src/error.rs. Uses a within-canonical-layout-but-not-configured
    handle (program=15, within=15 = 0xFF) so the closeout audit's
    `from_raw` validation passes and the push-time range check fires.
    Forged handles with bits outside the canonical layout are covered
    by `tests/test_handle_forge.py`."""
    from tstrans.mpegts import VideoStreamHandle

    m = Muxer(_simple_config())
    # 0xFF = (program=15, within=15) — canonical layout, but no such
    # stream is configured. Push-time range check rejects.
    bogus = VideoStreamHandle.from_raw(0xFF)
    with pytest.raises(MuxError) as ei:
        m.push_video_to(bogus, _minimal_h264_nal_aud(), pts=Pts90khz.from_raw(900_000))
    assert ei.value.kind == MuxErrorKind.INVALID_USAGE


def test_push_video_then_pull_emits_188_aligned_bytes():
    m = Muxer(_simple_config())
    m.push_video(_minimal_h264_nal_aud(), pts=Pts90khz.from_raw(900_000))
    n_packets = int(m.pending_packets())
    assert n_packets > 0
    buf = bytearray(n_packets * 188)
    n = m.pull(buf)
    assert n > 0
    assert n % 188 == 0


def test_push_video_to_handle_form_works():
    m = Muxer(_simple_config())
    handle = m.video_handles()[0]
    m.push_video_to(handle, _minimal_h264_nal_aud(), pts=Pts90khz.from_raw(900_000))
    assert m.pending_packets() > 0


def test_push_video_to_with_dts_b_frame_schedule():
    m = Muxer(_simple_config())
    handle = m.video_handles()[0]
    m.push_video_to_with_dts(
        handle,
        _minimal_h264_nal_aud(),
        pts=Pts90khz.from_raw(990_000),
        dts=Pts90khz.from_raw(900_000),
    )
    assert m.pending_packets() > 0


def test_push_video_key_frame_arg_accepted():
    """key_frame=True must be a valid kwarg (random_access_indicator
    semantics tested in Rust-side tests; here we just verify the Python
    surface accepts it)."""
    m = Muxer(_simple_config())
    m.push_video(
        _minimal_h264_nal_aud(), pts=Pts90khz.from_raw(900_000), key_frame=True
    )
    assert m.pending_packets() > 0


# ---------------------------------------------------------------------------
# Task 8 — push_audio + push_klv + push_subtitle (single-stream + handle forms)
# ---------------------------------------------------------------------------


def _minimal_aac_frame() -> bytes:
    """ADTS header — syncword + LC profile + 44.1kHz + mono + length=7.

    Smallest valid AAC-ADTS frame: 7-byte header with no payload. The
    mux-side audio path just frames bytes into PES; the parser is
    receiver-side, so any well-shaped ADTS header is enough to exercise
    the push path.
    """
    return b"\xFF\xF1\x4C\x40\x00\x1F\xFC"


def _minimal_klv_ls() -> bytes:
    """16-byte SMPTE UL + 1-byte BER + 0-byte body = minimal LS bytes.

    The muxer auto-prepends the 5-byte `Metadata_AU_cell` header per
    ITU-T H.222.0 §2.12.4.2 for SynchronousMetadata streams (CLAUDE.md
    "KLV AU cell auto-wrap"), so callers pass raw KLV LS bytes only.
    """
    return b"\x06\x0E\x2B\x34\x02\x0B\x01\x01\x0E\x01\x03\x01\x01\x00\x00\x00\x00"


def test_push_audio_works_with_single_audio_stream():
    m = Muxer(_simple_config())
    m.push_audio(_minimal_aac_frame(), pts=Pts90khz.from_raw(900_000))
    assert m.pending_packets() >= 0


def test_push_klv_works_with_single_klv_stream():
    m = Muxer(_simple_config())
    m.push_klv(
        _minimal_klv_ls(), pts=Pts90khz.from_raw(900_000), metadata_service_id=0
    )
    assert m.pending_packets() >= 0


def test_push_klv_default_metadata_service_id():
    """metadata_service_id defaults to 0 — the most common single-service
    case. Callers needing a non-zero value pass it explicitly."""
    m = Muxer(_simple_config())
    m.push_klv(_minimal_klv_ls(), pts=Pts90khz.from_raw(900_000))
    assert m.pending_packets() >= 0


def test_push_audio_invalid_handle_raises():
    # See test_push_video_to_with_invalid_handle_raises_invalid_usage for
    # rationale on the 0xFF (canonical-but-unconfigured) handle choice.
    from tstrans.mpegts import AudioStreamHandle

    m = Muxer(_simple_config())
    bad = AudioStreamHandle.from_raw(0xFF)
    with pytest.raises(MuxError) as ei:
        # Audit #9 normalized arg order: (handle, frames, *, pts).
        m.push_audio_to(bad, _minimal_aac_frame(), pts=Pts90khz.from_raw(900_000))
    assert ei.value.kind is MuxErrorKind.INVALID_USAGE


def test_push_klv_invalid_handle_raises():
    # See test_push_video_to_with_invalid_handle_raises_invalid_usage for
    # rationale on the 0xFF (canonical-but-unconfigured) handle choice.
    from tstrans.mpegts import KlvStreamHandle

    m = Muxer(_simple_config())
    bad = KlvStreamHandle.from_raw(0xFF)
    with pytest.raises(MuxError) as ei:
        m.push_klv_to(bad, _minimal_klv_ls(), pts=Pts90khz.from_raw(900_000))
    assert ei.value.kind is MuxErrorKind.INVALID_USAGE


def _subtitle_config(codec_config) -> MuxerConfig:
    """Single-program muxer with one video + one subtitle stream.

    Pair the subtitle stream with a video stream because the muxer's
    PCR pid defaults to the first video — without one, builder validation
    would surface a config error.
    """
    prog = (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .add_subtitle(0x200, codec_config)
        .build()
    )
    return MuxerConfigBuilder().add_program(prog).build()


def test_push_subtitle_works_webvtt_round_trip():
    """End-to-end: build a WebVTT-in-TS subtitle stream, push a cue,
    pull TS bytes, demux, assert payload + spec match."""
    from tstrans.mpegts import (
        Demuxer,
        DemuxerConfig,
        DemuxEvent,
        StreamKindTag,
        SubtitleCodec as SubtitleCodecEnum,
        WebVttInTsConfig,
    )

    cfg = _subtitle_config(WebVttInTsConfig())
    m = Muxer(cfg)

    # Sanity: builder produced a subtitle stream the muxer recognizes.
    assert m.stats().subtitle_streams_configured == 1
    assert len(m.subtitle_handles()) == 1

    cue = b"WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nhello\n"
    m.push_subtitle(cue, pts=Pts90khz.from_raw(900_000))
    assert m.pending_packets() > 0

    # Drain all TS bytes (PAT + PMT + subtitle PES).
    buf = bytearray(m.pending_packets() * 188)
    n = m.pull(buf)
    assert n > 0
    assert n % 188 == 0
    ts_bytes = bytes(buf[:n])

    # Demux and find the subtitle event.
    d = Demuxer(DemuxerConfig())
    d.feed(ts_bytes)
    d.flush()
    events = list(d)

    # PMT should advertise the WebVtt subtitle stream.
    pmts = [e for e in events if isinstance(e, DemuxEvent.ProgramMap)]
    assert pmts, "expected at least one ProgramMap event"
    subtitle_streams = [
        s
        for s in pmts[-1].programs[0].streams
        if s.kind is StreamKindTag.SUBTITLE
    ]
    assert len(subtitle_streams) == 1, f"got {subtitle_streams}"
    assert subtitle_streams[0].codec is SubtitleCodecEnum.WEBVTT_IN_TS
    assert subtitle_streams[0].pid == 0x200

    # Subtitle event payload round-trips byte-for-byte.
    sub_events = [e for e in events if isinstance(e, DemuxEvent.Subtitle)]
    assert sub_events, "expected at least one Subtitle event"
    assert sub_events[0].payload == cue


def test_push_subtitle_works_dvb_subtitling_round_trip():
    """DVB subtitling — exercises the struct-variant config path with
    full language + page-id parameters."""
    from tstrans.mpegts import (
        Demuxer,
        DemuxerConfig,
        DemuxEvent,
        DvbSubtitlingConfig,
        StreamKindTag,
        SubtitleCodec as SubtitleCodecEnum,
    )

    cfg = _subtitle_config(
        DvbSubtitlingConfig(
            language=b"eng",
            subtitling_type=0x10,
            composition_page_id=0x1234,
            ancillary_page_id=0x5678,
        )
    )
    m = Muxer(cfg)
    assert m.stats().subtitle_streams_configured == 1

    # 3-byte DVB sub data: data_identifier=0x20, subtitle_stream_id=0x00,
    # end_of_PES_data_field_marker=0xFF. The muxer auto-wraps the PES
    # envelope per ETSI EN 300 743 §6.2, so callers pass raw segments.
    payload = b"\x42\x42\x42"
    m.push_subtitle(payload, pts=Pts90khz.from_raw(900_000))
    assert m.pending_packets() > 0

    buf = bytearray(m.pending_packets() * 188)
    n = m.pull(buf)
    ts_bytes = bytes(buf[:n])

    d = Demuxer(DemuxerConfig())
    d.feed(ts_bytes)
    d.flush()
    events = list(d)

    pmts = [e for e in events if isinstance(e, DemuxEvent.ProgramMap)]
    assert pmts
    subtitle_streams = [
        s
        for s in pmts[-1].programs[0].streams
        if s.kind is StreamKindTag.SUBTITLE
    ]
    # The demuxer collapses struct-variant subtitle codecs to the flat
    # enum tag — the per-stream descriptor bytes carry the language /
    # page IDs (not surfaced via this Python `StreamInfo` listing).
    assert len(subtitle_streams) == 1
    assert subtitle_streams[0].codec is SubtitleCodecEnum.DVB_SUBTITLING
    assert subtitle_streams[0].pid == 0x200


def test_push_subtitle_works_dvb_teletext_round_trip():
    """DVB teletext — exercises the second struct-variant config path."""
    from tstrans.mpegts import (
        Demuxer,
        DemuxerConfig,
        DemuxEvent,
        DvbTeletextConfig,
        StreamKindTag,
        SubtitleCodec as SubtitleCodecEnum,
    )

    cfg = _subtitle_config(
        DvbTeletextConfig(
            language=b"eng",
            teletext_type=0x02,  # subtitle page
            magazine_number=1,
            page_number=0x88,
        )
    )
    m = Muxer(cfg)
    assert m.stats().subtitle_streams_configured == 1

    # 1-byte data identifier (per ETSI EN 300 472 §4.1).
    payload = b"\x10"
    m.push_subtitle(payload, pts=Pts90khz.from_raw(900_000))
    assert m.pending_packets() > 0

    buf = bytearray(m.pending_packets() * 188)
    n = m.pull(buf)
    ts_bytes = bytes(buf[:n])

    d = Demuxer(DemuxerConfig())
    d.feed(ts_bytes)
    d.flush()
    events = list(d)

    pmts = [e for e in events if isinstance(e, DemuxEvent.ProgramMap)]
    assert pmts
    subtitle_streams = [
        s
        for s in pmts[-1].programs[0].streams
        if s.kind is StreamKindTag.SUBTITLE
    ]
    assert len(subtitle_streams) == 1
    assert subtitle_streams[0].codec is SubtitleCodecEnum.DVB_TELETEXT
    assert subtitle_streams[0].pid == 0x200


def test_push_subtitle_works_cea708_standalone_round_trip():
    """CEA-708 standalone — exercises the second unit-variant config."""
    from tstrans.mpegts import (
        Cea708StandaloneConfig,
        Demuxer,
        DemuxerConfig,
        DemuxEvent,
        StreamKindTag,
        SubtitleCodec as SubtitleCodecEnum,
    )

    cfg = _subtitle_config(Cea708StandaloneConfig())
    m = Muxer(cfg)
    assert m.stats().subtitle_streams_configured == 1

    payload = b"\xFF\xFF\xFF"
    m.push_subtitle(payload, pts=Pts90khz.from_raw(900_000))
    assert m.pending_packets() > 0

    buf = bytearray(m.pending_packets() * 188)
    n = m.pull(buf)
    ts_bytes = bytes(buf[:n])

    d = Demuxer(DemuxerConfig())
    d.feed(ts_bytes)
    d.flush()
    events = list(d)

    pmts = [e for e in events if isinstance(e, DemuxEvent.ProgramMap)]
    assert pmts
    subtitle_streams = [
        s
        for s in pmts[-1].programs[0].streams
        if s.kind is StreamKindTag.SUBTITLE
    ]
    assert len(subtitle_streams) == 1
    assert subtitle_streams[0].codec is SubtitleCodecEnum.CEA708_STANDALONE
    assert subtitle_streams[0].pid == 0x200


def test_push_subtitle_to_handle_form_works():
    """Handle-form push works after `add_subtitle` plumbing."""
    from tstrans.mpegts import SubtitleStreamHandle, WebVttInTsConfig

    cfg = _subtitle_config(WebVttInTsConfig())
    m = Muxer(cfg)
    handles = m.subtitle_handles()
    assert len(handles) == 1
    assert isinstance(handles[0], SubtitleStreamHandle)

    m.push_subtitle_to(
        handles[0],
        b"WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nhi\n",
        pts=Pts90khz.from_raw(900_000),
    )
    assert m.pending_packets() > 0


def test_push_subtitle_to_invalid_handle_raises():
    """Out-of-range handle surfaces as INVALID_USAGE via the
    MuxSenderErrorKind classifier."""
    from tstrans.mpegts import SubtitleStreamHandle, WebVttInTsConfig

    cfg = _subtitle_config(WebVttInTsConfig())
    m = Muxer(cfg)
    bogus = SubtitleStreamHandle.from_raw((255 << 16) | 255)
    with pytest.raises(MuxError) as ei:
        m.push_subtitle_to(
            bogus,
            b"WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nx\n",
            pts=Pts90khz.from_raw(900_000),
        )
    assert ei.value.kind is MuxErrorKind.INVALID_USAGE


# ---------------------------------------------------------------------------
# Subtitle codec config dataclass validation (__post_init__ guards)
# ---------------------------------------------------------------------------


def test_dvb_subtitling_config_rejects_out_of_range_page_ids():
    """Page IDs are u16; values >= 0x10000 must be rejected at
    construction (not later inside the muxer)."""
    from tstrans.mpegts import DvbSubtitlingConfig

    with pytest.raises(ValueError, match="composition_page_id"):
        DvbSubtitlingConfig(
            language=b"eng",
            subtitling_type=0x10,
            composition_page_id=0x10000,  # one over u16
            ancillary_page_id=0,
        )

    with pytest.raises(ValueError, match="ancillary_page_id"):
        DvbSubtitlingConfig(
            language=b"eng",
            subtitling_type=0x10,
            composition_page_id=0,
            ancillary_page_id=0x10000,
        )


def test_dvb_subtitling_config_rejects_wrong_language_length():
    from tstrans.mpegts import DvbSubtitlingConfig

    with pytest.raises(ValueError, match="language"):
        DvbSubtitlingConfig(
            language=b"en",  # 2 bytes — should be 3
            subtitling_type=0x10,
            composition_page_id=0,
            ancillary_page_id=0,
        )


def test_dvb_subtitling_config_rejects_subtitling_type_out_of_u8():
    from tstrans.mpegts import DvbSubtitlingConfig

    with pytest.raises(ValueError, match="subtitling_type"):
        DvbSubtitlingConfig(
            language=b"eng",
            subtitling_type=256,  # one over u8
            composition_page_id=0,
            ancillary_page_id=0,
        )


def test_dvb_teletext_config_rejects_magazine_over_seven():
    """magazine_number is 3 bits — 0..=7. Higher values are wire-invalid."""
    from tstrans.mpegts import DvbTeletextConfig

    with pytest.raises(ValueError, match="magazine_number"):
        DvbTeletextConfig(
            language=b"eng",
            teletext_type=0x02,
            magazine_number=8,  # out of 3-bit range
            page_number=0x88,
        )


def test_dvb_teletext_config_rejects_non_bcd_page_number():
    """page_number is BCD-encoded — each nibble must be 0..=9."""
    from tstrans.mpegts import DvbTeletextConfig

    with pytest.raises(ValueError, match="page_number"):
        # Low nibble = 0xA (10) — invalid BCD.
        DvbTeletextConfig(
            language=b"eng",
            teletext_type=0x02,
            magazine_number=1,
            page_number=0x8A,
        )

    with pytest.raises(ValueError, match="page_number"):
        # Over 0x99.
        DvbTeletextConfig(
            language=b"eng",
            teletext_type=0x02,
            magazine_number=1,
            page_number=0x100,
        )


def test_dvb_teletext_config_rejects_teletext_type_over_31():
    """teletext_type is a 5-bit field (0..=31)."""
    from tstrans.mpegts import DvbTeletextConfig

    with pytest.raises(ValueError, match="teletext_type"):
        DvbTeletextConfig(
            language=b"eng",
            teletext_type=32,  # one over 5-bit range
            magazine_number=1,
            page_number=0x88,
        )


def test_add_subtitle_rejects_non_subtitle_codec_config():
    """`add_subtitle` raises TypeError when passed an arbitrary object."""

    with pytest.raises(TypeError, match="DvbSubtitlingConfig"):
        MuxerProgramConfigBuilder(1, 0x100).add_video(
            0x101, VideoCodec.H264
        ).add_subtitle(0x200, "not_a_config")  # type: ignore[arg-type]


# ---------------------------------------------------------------------------
# Task 9 — handle getters (video/audio/klv/subtitle × list + by_program + by_index)
# ---------------------------------------------------------------------------
#
# Rust surface coverage (verified against tst-core/src/mpegts/mux/*.rs):
#   video_*    — list + by_program + by_index
#   audio_*    — list + by_program             (no by-index getter Rust-side)
#   klv_*      — list + by_program + by_index
#   subtitle_* — list + by_program             (no by-index getter Rust-side)
#
# `*_handles_for_program` returns `Result<Vec<_>, MuxError>` in Rust and
# raises `MuxError(INVALID_USAGE)` (via the MuxSenderErrorKind classifier
# mapping `ProgramNotFound`) on a non-existent program number; the Python
# wraps propagate that error rather than returning an empty list, so the
# call-site sees the same shape as the Rust API.


def test_video_handles_returns_one_per_video_stream():
    from tstrans.mpegts import VideoStreamHandle

    m = Muxer(_simple_config())
    h = m.video_handles()
    assert len(h) == 1
    assert isinstance(h[0], VideoStreamHandle)


def test_audio_handles_returns_one_per_audio_stream():
    from tstrans.mpegts import AudioStreamHandle

    m = Muxer(_simple_config())
    h = m.audio_handles()
    assert len(h) == 1
    assert isinstance(h[0], AudioStreamHandle)


def test_klv_handles_returns_one_per_klv_stream():
    from tstrans.mpegts import KlvStreamHandle

    m = Muxer(_simple_config())
    h = m.klv_handles()
    assert len(h) == 1
    assert isinstance(h[0], KlvStreamHandle)


def test_subtitle_handles_empty_when_no_subtitle_stream():
    # _simple_config configures no subtitle stream — list-form getter
    # returns an empty list (not an error), matching the Rust contract.
    m = Muxer(_simple_config())
    assert m.subtitle_handles() == []


def test_video_handles_for_program_returns_program_streams():
    m = Muxer(_simple_config())
    h_p1 = m.video_handles_for_program(1)
    assert len(h_p1) == 1


def test_video_handles_for_program_unknown_raises_invalid_usage():
    # Rust returns `Err(MuxError::ProgramNotFound)`, classified as
    # INVALID_USAGE by the MuxSenderErrorKind classifier in
    # tst-core/src/error.rs. The Python wrap propagates that error.
    m = Muxer(_simple_config())
    with pytest.raises(MuxError) as ei:
        m.video_handles_for_program(99)
    assert ei.value.kind is MuxErrorKind.INVALID_USAGE


def test_audio_handles_for_program_returns_program_streams():
    m = Muxer(_simple_config())
    h_p1 = m.audio_handles_for_program(1)
    assert len(h_p1) == 1


def test_klv_handles_for_program_returns_program_streams():
    m = Muxer(_simple_config())
    h_p1 = m.klv_handles_for_program(1)
    assert len(h_p1) == 1


def test_subtitle_handles_for_program_empty_program_returns_empty():
    # Program 1 exists but has no subtitle streams — empty list, no error.
    m = Muxer(_simple_config())
    assert m.subtitle_handles_for_program(1) == []


def test_video_stream_handle_by_index_returns_handle():
    from tstrans.mpegts import VideoStreamHandle

    m = Muxer(_simple_config())
    h = m.video_stream_handle(0)
    assert h is not None
    assert isinstance(h, VideoStreamHandle)


def test_video_stream_handle_by_index_oob_returns_none():
    m = Muxer(_simple_config())
    assert m.video_stream_handle(99) is None


def test_klv_stream_handle_by_index_returns_handle():
    from tstrans.mpegts import KlvStreamHandle

    m = Muxer(_simple_config())
    h = m.klv_stream_handle(0)
    assert h is not None
    assert isinstance(h, KlvStreamHandle)


def test_klv_stream_handle_by_index_oob_returns_none():
    m = Muxer(_simple_config())
    assert m.klv_stream_handle(99) is None


def test_video_handle_round_trips_through_push_video_to():
    # End-to-end: getter → push_video_to → muxer accepts.
    m = Muxer(_simple_config())
    handle = m.video_handles()[0]
    m.push_video_to(handle, _minimal_h264_nal_aud(), pts=Pts90khz.from_raw(900_000))
    assert m.pending_packets() > 0


# ---------------------------------------------------------------------------
# Audit #9 — pts is keyword-only on every push_* method, and push_audio_to
# now takes `frames` positionally before its kw-only `pts` (normalized to
# match push_audio's `(frames, *, pts)` shape).
#
# These tests pin the Pythonic signature shape: positional `pts` MUST raise
# `TypeError` at the PyO3 argument-extraction boundary; the kwarg form MUST
# succeed.
# ---------------------------------------------------------------------------


def test_push_video_pts_positional_raises_type_error():
    """Audit #9: pts must be passed as a kwarg on push_video."""
    m = Muxer(_simple_config())
    nal = _minimal_h264_nal_aud()
    with pytest.raises(TypeError):
        m.push_video(nal, Pts90khz.from_raw(900_000), True)  # positional pts + key_frame
    # Kw form works.
    m.push_video(nal, pts=Pts90khz.from_raw(900_000), key_frame=True)
    assert m.pending_packets() > 0


def test_push_video_to_pts_positional_raises_type_error():
    """Audit #9: pts must be passed as a kwarg on push_video_to."""
    m = Muxer(_simple_config())
    handle = m.video_handles()[0]
    nal = _minimal_h264_nal_aud()
    with pytest.raises(TypeError):
        m.push_video_to(handle, nal, Pts90khz.from_raw(900_000))  # positional pts
    # Kw form works.
    m.push_video_to(handle, nal, pts=Pts90khz.from_raw(900_000))
    assert m.pending_packets() > 0


def test_push_audio_pts_positional_raises_type_error():
    """Audit #9: pts must be passed as a kwarg on push_audio."""
    m = Muxer(_simple_config())
    frames = _minimal_aac_frame()
    with pytest.raises(TypeError):
        m.push_audio(frames, Pts90khz.from_raw(900_000))  # positional pts
    # Kw form works.
    m.push_audio(frames, pts=Pts90khz.from_raw(900_000))
    assert m.pending_packets() >= 0


def test_push_audio_to_normalized_arg_order_and_kwonly_pts():
    """Audit #9: push_audio_to is now (handle, frames, *, pts) — `frames`
    moved before `pts` to match push_audio's `(frames, *, pts)` shape; pts
    is keyword-only."""
    m = Muxer(_simple_config())
    handle = m.audio_handles()[0]
    frames = _minimal_aac_frame()
    # New normalized form works.
    m.push_audio_to(handle, frames, pts=Pts90khz.from_raw(900_000))
    assert m.pending_packets() >= 0
    # Old (handle, pts, frames) shape must now raise. A Pts90khz handed
    # as the second positional (where `frames` lives now) fails byte
    # extraction at the PyO3 boundary → TypeError.
    with pytest.raises(TypeError):
        m.push_audio_to(handle, Pts90khz.from_raw(900_000), frames)


def test_push_klv_pts_positional_raises_type_error():
    """Audit #9: pts must be passed as a kwarg on push_klv."""
    m = Muxer(_simple_config())
    klv = _minimal_klv_ls()
    with pytest.raises(TypeError):
        m.push_klv(klv, Pts90khz.from_raw(900_000))  # positional pts
    # Kw form works (including the default metadata_service_id).
    m.push_klv(klv, pts=Pts90khz.from_raw(900_000))
    assert m.pending_packets() >= 0


def test_push_klv_to_pts_positional_raises_type_error():
    """Audit #9: pts must be passed as a kwarg on push_klv_to."""
    m = Muxer(_simple_config())
    handle = m.klv_handles()[0]
    klv = _minimal_klv_ls()
    with pytest.raises(TypeError):
        m.push_klv_to(handle, klv, Pts90khz.from_raw(900_000))  # positional pts
    # Kw form works.
    m.push_klv_to(handle, klv, pts=Pts90khz.from_raw(900_000))
    assert m.pending_packets() >= 0


def test_push_video_to_with_dts_signature_unchanged():
    """Audit #9: push_video_to_with_dts was already kw-only — this test
    pins that the audit-9 sweep didn't accidentally regress its shape."""
    m = Muxer(_simple_config())
    handle = m.video_handles()[0]
    m.push_video_to_with_dts(
        handle,
        _minimal_h264_nal_aud(),
        pts=Pts90khz.from_raw(990_000),
        dts=Pts90khz.from_raw(900_000),
    )
    assert m.pending_packets() > 0
    # Positional pts/dts still rejected.
    with pytest.raises(TypeError):
        m.push_video_to_with_dts(
            handle,
            _minimal_h264_nal_aud(),
            Pts90khz.from_raw(990_000),
            Pts90khz.from_raw(900_000),
        )
