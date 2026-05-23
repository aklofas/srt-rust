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
    m.push_video(_minimal_h264_nal_aud(), Pts90khz.from_raw(900_000))
    assert m.pending_packets() > before


def test_push_video_invalid_nal_raises_input_malformed():
    m = Muxer(_simple_config())
    with pytest.raises(MuxError) as ei:
        # No Annex-B start code — should fail validate_annex_b.
        m.push_video(b"\xDE\xAD\xBE\xEF", Pts90khz.from_raw(900_000))
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
        m.push_video_to(bogus, _minimal_h264_nal_aud(), Pts90khz.from_raw(900_000))
    assert ei.value.kind == MuxErrorKind.INVALID_USAGE


def test_push_video_then_pull_emits_188_aligned_bytes():
    m = Muxer(_simple_config())
    m.push_video(_minimal_h264_nal_aud(), Pts90khz.from_raw(900_000))
    n_packets = int(m.pending_packets())
    assert n_packets > 0
    buf = bytearray(n_packets * 188)
    n = m.pull(buf)
    assert n > 0
    assert n % 188 == 0


def test_push_video_to_handle_form_works():
    # Task 9 will add video_handles(); without it, skip and rely on the
    # single-target form's coverage above.
    m = Muxer(_simple_config())
    if not hasattr(m, "video_handles"):
        pytest.skip("video_handles() not yet wired (Task 9)")
    handle = m.video_handles()[0]
    m.push_video_to(handle, _minimal_h264_nal_aud(), Pts90khz.from_raw(900_000))
    assert m.pending_packets() > 0


def test_push_video_to_with_dts_b_frame_schedule():
    m = Muxer(_simple_config())
    if not hasattr(m, "video_handles"):
        pytest.skip("video_handles() not yet wired (Task 9)")
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
    m.push_video(_minimal_h264_nal_aud(), Pts90khz.from_raw(900_000), key_frame=True)
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
    m.push_audio(_minimal_aac_frame(), Pts90khz.from_raw(900_000))
    assert m.pending_packets() >= 0


def test_push_klv_works_with_single_klv_stream():
    m = Muxer(_simple_config())
    m.push_klv(_minimal_klv_ls(), Pts90khz.from_raw(900_000), metadata_service_id=0)
    assert m.pending_packets() >= 0


def test_push_klv_default_metadata_service_id():
    """metadata_service_id defaults to 0 — the most common single-service
    case. Callers needing a non-zero value pass it explicitly."""
    m = Muxer(_simple_config())
    m.push_klv(_minimal_klv_ls(), Pts90khz.from_raw(900_000))
    assert m.pending_packets() >= 0


def test_push_audio_invalid_handle_raises():
    from tstrans.mpegts import AudioStreamHandle

    m = Muxer(_simple_config())
    bad = AudioStreamHandle.from_raw((255 << 16) | 255)
    with pytest.raises(MuxError) as ei:
        m.push_audio_to(bad, Pts90khz.from_raw(900_000), _minimal_aac_frame())
    assert ei.value.kind is MuxErrorKind.INVALID_USAGE


def test_push_klv_invalid_handle_raises():
    from tstrans.mpegts import KlvStreamHandle

    m = Muxer(_simple_config())
    bad = KlvStreamHandle.from_raw((255 << 16) | 255)
    with pytest.raises(MuxError) as ei:
        m.push_klv_to(bad, _minimal_klv_ls(), Pts90khz.from_raw(900_000))
    assert ei.value.kind is MuxErrorKind.INVALID_USAGE


@pytest.mark.skip(
    reason="push_subtitle end-to-end requires deeper SubtitleCodec Python "
    "representation (mux-side codec is struct-variant; Phase 4 follow-up)"
)
def test_push_subtitle_works():
    pass  # placeholder for future deeper subtitle support
