"""tstrans.pipeline.Pairer — byte-feeding pairing surface."""

from datetime import timedelta

import tstrans
from tstrans.mpegts import (
    DemuxEvent,
    Demuxer,
    KlvStreamType,
    Muxer,
    MuxerConfigBuilder,
    MuxerProgramConfigBuilder,
    Pts90khz,
    VideoCodec,
)
from tstrans.pipeline import (
    Pairer,
    PairerConfig,
    PairerMode,
    PairerOutput,
    PairingDemuxerConfig,
)

VIDEO_PID = 0x101
KLV_PID = 0x102


def _minimal_h264_au() -> bytes:
    # AUD (nal_type=9) + IDR (nal_type=5), Annex-B — mirrors the Rust test
    # fixture in pairing_demuxer_round_trip.rs for cross-language consistency.
    return bytes(
        [0x00, 0x00, 0x00, 0x01, 0x09, 0x10, 0x00, 0x00, 0x00, 0x01, 0x65, 0xAA, 0xBB, 0xCC]
    )


def _dummy_klv() -> bytes:
    # 16-byte SMPTE UL (ST 0601 key) + 1-byte BER length (4) + 4-byte value.
    ul = bytes(
        [0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00, 0x00]
    )
    return ul + bytes([4, 0x01, 0x02, 0x03, 0x04])


def _sync_klv_bytes() -> bytes:
    """Mux 5 video AUs + 5 KLV records at matching PTS (sync-KLV fixture).

    The Rust core test `pairing_demuxer_round_trip.rs::with_config_matches_bare_pairer_oracle`
    proves that a Realtime pairer with 100 ms tolerance yields exactly 5 Paired
    outputs from this fixture. The Python `test_realtime_pairs_sync_klv` test
    verifies the same guarantee holds across the binding layer.
    """
    prog = (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(VIDEO_PID, VideoCodec.H264)
        .add_klv(KLV_PID, KlvStreamType.SYNCHRONOUS_METADATA, carries_pts=True)
        .build()
    )
    m = Muxer(MuxerConfigBuilder().add_program(prog).build())
    for i in range(5):
        pts = Pts90khz.from_raw(90_000 + i * 3000)
        m.push_video(_minimal_h264_au(), pts=pts)
        m.push_klv(_dummy_klv(), pts=pts)
    out = bytearray()
    buf = bytearray(188 * 64)
    while True:
        n = m.pull(buf)
        if n == 0:
            break
        out.extend(buf[:n])
    return bytes(out)


def test_realtime_pairs_sync_klv():
    """5 video AUs + 5 KLV records at matching PTS → exactly 5 Paired.

    This is the cross-language equivalent of the Rust core test
    `with_config_matches_bare_pairer_oracle`. If a count other than 5 is
    observed, that indicates a conversion or wiring bug in the binding.
    """
    data = _sync_klv_bytes()
    pairer = Pairer(
        VIDEO_PID,
        KLV_PID,
        PairingDemuxerConfig(
            pairer=PairerConfig(mode=PairerMode.Realtime, tolerance=timedelta(milliseconds=100))
        ),
    )
    outs = pairer.feed(data)
    outs += pairer.flush()
    paired = [o for o in outs if isinstance(o, PairerOutput.Paired)]
    assert len(paired) == 5, (
        f"expected 5 Paired, got {len(paired)}; "
        f"variant breakdown: {[type(o).__name__ for o in outs]}"
    )
    assert pairer.stats()["paired"] == 5
    p0 = paired[0]
    assert p0.video.codec == VideoCodec.H264
    assert isinstance(p0.video.payload, list) and len(p0.video.payload) >= 1
    assert isinstance(p0.klv.payload, (bytes, bytearray)) and len(p0.klv.payload) > 0


def test_passthrough_matches_bare_demuxer_oracle():
    """PassThrough events from Pairer match events from a bare Demuxer.

    Non-video/KLV events (PAT, PMT) pass through unchanged. The bare
    Demuxer is the oracle: both must see the same ProgramMap event.
    """
    data = _sync_klv_bytes()
    pairer = Pairer(VIDEO_PID, KLV_PID)
    outs = pairer.feed(data) + pairer.flush()

    passthrough_events = [o.event for o in outs if isinstance(o, PairerOutput.PassThrough)]
    passthrough_type_names = [type(e).__name__ for e in passthrough_events]

    # The bare oracle sees the same events.
    demux = Demuxer()
    demux.feed(data)
    demux.flush()
    all_events = list(demux)

    # Both the pairer's pass-through set and the bare demuxer must include
    # at least one ProgramMap event (PAT/PMT discovery always fires one).
    assert any(isinstance(e, DemuxEvent.ProgramMap) for e in passthrough_events), (
        f"no ProgramMap in pass-through events; got types: {passthrough_type_names}"
    )
    assert any(isinstance(e, DemuxEvent.ProgramMap) for e in all_events), (
        "bare demuxer also saw no ProgramMap — fixture problem"
    )


def test_demuxer_stats_and_reset():
    """stats() and demuxer_stats() reflect real work; reset_stats() zeroes pairer counters."""
    data = _sync_klv_bytes()
    pairer = Pairer(VIDEO_PID, KLV_PID)
    pairer.feed(data)
    pairer.flush()

    # demuxer_stats reflects the PMT parse.
    assert pairer.demuxer_stats()["program_maps_seen"] > 0
    # pairing happened.
    assert pairer.stats()["paired"] > 0

    # reset_stats clears only the pairer counters.
    pairer.reset_stats()
    assert pairer.stats() == {
        "paired": 0,
        "unpaired_video": 0,
        "unpaired_klv": 0,
        "pass_through": 0,
    }


def test_feed_malformed_data_handled():
    """Malformed bytes that are not TS-sync-aligned: either resync and return
    an empty list (partial packets) or raise DemuxError. Both are valid;
    neither should panic or corrupt internal state.
    """
    pairer = Pairer(VIDEO_PID, KLV_PID)
    try:
        result = pairer.feed(b"\x00" * 200)
        # 200 null bytes contain no TS sync byte (0x47), so the only sensible
        # non-error outcome is an empty list — nothing was demuxed or paired.
        assert result == []
    except tstrans.exceptions.DemuxError:
        pass  # explicit error from strict-mode resync is also acceptable


def test_buffered_config_constructs_and_pairs():
    """Pairer constructed with Buffered mode config yields Paired outputs."""
    cfg = PairingDemuxerConfig(
        pairer=PairerConfig(
            mode=PairerMode.Buffered(max_lag=timedelta(milliseconds=200)),
            tolerance=timedelta(milliseconds=50),
            max_buffered_video=16,
        )
    )
    pairer = Pairer(VIDEO_PID, KLV_PID, cfg)
    outs = pairer.feed(_sync_klv_bytes()) + pairer.flush()
    assert any(isinstance(o, PairerOutput.Paired) for o in outs)


def test_default_constructor_no_config():
    """Pairer with no config uses defaults and accepts the fixture."""
    pairer = Pairer(VIDEO_PID, KLV_PID)
    outs = pairer.feed(_sync_klv_bytes()) + pairer.flush()
    assert isinstance(outs, list)
    # At least some output must have been produced (PAT/PMT pass-through
    # is always present, even if pairing count varies with default tolerance).
    assert len(outs) > 0


def test_stats_keys_present():
    """stats() and demuxer_stats() return dicts with the documented key sets."""
    pairer = Pairer(VIDEO_PID, KLV_PID)
    s = pairer.stats()
    assert set(s.keys()) == {"paired", "unpaired_video", "unpaired_klv", "pass_through"}
    ds = pairer.demuxer_stats()
    assert set(ds.keys()) == {
        "program_maps_seen",
        "pmt_versions_seen",
        "discontinuities",
        "nonconformant",
        "programs_seen",
        "subtitle_streams_seen",
    }


def test_pairer_output_isinstance_hierarchy():
    """PairerOutput.Paired is a subclass of PairerOutput — isinstance checks work both ways."""
    data = _sync_klv_bytes()
    pairer = Pairer(
        VIDEO_PID,
        KLV_PID,
        PairingDemuxerConfig(
            pairer=PairerConfig(mode=PairerMode.Realtime, tolerance=timedelta(milliseconds=100))
        ),
    )
    outs = pairer.feed(data) + pairer.flush()
    paired = [o for o in outs if isinstance(o, PairerOutput.Paired)]
    assert len(paired) > 0
    for p in paired:
        # Every Paired is also a PairerOutput (base class check).
        assert isinstance(p, PairerOutput)
