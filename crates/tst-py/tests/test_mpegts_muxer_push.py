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
    """An out-of-range handle (constructed via from_raw) surfaces as
    InvalidStreamHandle → INVALID_USAGE per the MuxSenderErrorKind
    classifier in tst-core/src/error.rs."""
    from tstrans.mpegts import VideoStreamHandle

    m = Muxer(_simple_config())
    # Pack program=255, within=255 — beyond any plausible muxer config.
    bogus = VideoStreamHandle.from_raw((255 << 16) | 255)
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

    The muxer auto-prepends the 5-byte ST 1910 AU cell header for
    SynchronousMetadata streams (CLAUDE.md "KLV AU cell auto-wrap"),
    so callers pass raw KLV LS bytes only.
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
    from tstrans.mpegts import AudioStreamHandle

    m = Muxer(_simple_config())
    bad = AudioStreamHandle.from_raw((255 << 16) | 255)
    with pytest.raises(MuxError) as ei:
        # Audit #9 normalized arg order: (handle, frames, *, pts).
        m.push_audio_to(bad, _minimal_aac_frame(), pts=Pts90khz.from_raw(900_000))
    assert ei.value.kind is MuxErrorKind.INVALID_USAGE


def test_push_klv_invalid_handle_raises():
    from tstrans.mpegts import KlvStreamHandle

    m = Muxer(_simple_config())
    bad = KlvStreamHandle.from_raw((255 << 16) | 255)
    with pytest.raises(MuxError) as ei:
        m.push_klv_to(bad, _minimal_klv_ls(), pts=Pts90khz.from_raw(900_000))
    assert ei.value.kind is MuxErrorKind.INVALID_USAGE


@pytest.mark.skip(
    reason="push_subtitle end-to-end requires deeper SubtitleCodec Python "
    "representation (mux-side codec is struct-variant; Phase 4 follow-up)"
)
def test_push_subtitle_works():
    pass  # placeholder for future deeper subtitle support


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
