"""tio.transmux — the v0.2.0 demux→edit→remux capstone (Wave 6).

Covers: lazy muxer construction from the first ProgramMap, byte-faithful
copy semantics, the corrector acceptance workflow (patch_uas_datalink),
v1 single-program scope errors, drop= filtering, atomic mode, and
lifecycle misuse errors.
"""
from __future__ import annotations

from pathlib import Path

import pytest

import tstrans.io as tio
from tstrans.mpegts import (
    DemuxEvent,
    KlvStreamType,
    Muxer,
    MuxerConfigBuilder,
    MuxerProgramConfigBuilder,
    Pts90khz,
    StreamKindTag,
    VideoCodec,
)

# Three distinct minimal-but-valid H.264 Annex-B AUs (one IDR + two
# non-IDR) so byte-faithfulness is distinguishable per AU. Same shape as
# test_raw_first.py's acceptance fixture.
ORIG_AUS = [
    b"\x00\x00\x00\x01\x65\x88\x84\x00\x10\xaa\xbb",
    b"\x00\x00\x00\x01\x41\x9a\x00\x34\xcc",
    b"\x00\x00\x00\x01\x41\x9a\x01\x35\xdd\xee",
]
KEY_FRAMES = [True, False, False]
PTS0 = 900_000
PTS_STEP = 3_000


def _make_klv_bytes(lat: float) -> bytes:
    from tstrans.klv import ST_0601_UL, UasDatalinkLs, encode_uas_datalink

    rec = UasDatalinkLs(
        universal_label=ST_0601_UL,
        declared_version=19,
        timestamp_us=1_700_000_000_000_000,
        frame_center_lat_deg=lat,
        frame_center_lon_deg=-122.3321,
        sensor_lat_deg=47.6200,
        sensor_lon_deg=-122.3000,
        sensor_alt_m=500.0,
    )
    return encode_uas_datalink(rec)


def _write_video_klv_src(path: Path, lat: float = 47.6097) -> None:
    """Synthetic single-program H.264 + sync-KLV source."""
    cfg = (
        MuxerConfigBuilder()
        .add_program(
            MuxerProgramConfigBuilder(1, 0x100)
            .add_video(0x101, VideoCodec.H264)
            .add_klv(0x102, KlvStreamType.SYNCHRONOUS_METADATA, carries_pts=True)
            .build()
        )
        .build()
    )
    mux = Muxer(cfg)
    with mux.write_file(path) as proxy:
        for i, (au, key) in enumerate(zip(ORIG_AUS, KEY_FRAMES)):
            pts = Pts90khz.from_raw(PTS0 + i * PTS_STEP)
            proxy.push_video(au, pts=pts, key_frame=key)
            proxy.push_klv(_make_klv_bytes(lat), pts=pts)


def _collect(path: Path):
    """(video_aus, klv_payloads) from a TS file, in event order."""
    videos: list[bytes] = []
    klvs: list[bytes] = []
    for ev in tio.parse_file(path):
        if isinstance(ev, DemuxEvent.Video):
            videos.append(bytes(ev.raw))
        elif isinstance(ev, DemuxEvent.Klv):
            klvs.append(bytes(ev.payload))
    return videos, klvs


def test_transmux_pass_through_copies_everything_byte_faithfully(tmp_path):
    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    _write_video_klv_src(src)

    with tio.transmux(src, dst) as tx:
        # Single-pass iterator contract (file-object semantics).
        assert iter(tx) is iter(tx)
        for ev in tx:
            tx.write(ev)

    src_videos, src_klvs = _collect(src)
    out_videos, out_klvs = _collect(dst)
    assert out_videos == ORIG_AUS == src_videos
    assert out_klvs == src_klvs
    assert len(out_klvs) == len(ORIG_AUS)


def test_transmux_dst_not_created_for_psi_less_source(tmp_path):
    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    src.write_bytes(b"")  # no PSI → no ProgramMap → lazy sink never opens
    with tio.transmux(src, dst) as tx:
        for ev in tx:
            tx.write(ev)
    assert not dst.exists()


# ---------------------------------------------------------------------------
# Acceptance (umbrella spec Wave 6): the corrector workflow end-to-end.
# Patch frame-center tags on a synthetic fixture via klv.patch_uas_datalink
# (Wave 1), byte-compare every video AU out vs in, and verify the patched
# KLV differs ONLY at the edited TLVs + checksum.
# ---------------------------------------------------------------------------

def _walk_tlvs(ls: bytes) -> list[tuple[int, bytes]]:
    """Top-level (tag, full-TLV-bytes) pairs of a KLV local set:
    16-byte UL + BER outer length, then BER-OID tag / BER length / value
    triplets. Mirrors the wire layout patch_uas_datalink preserves."""

    def read_ber(buf: bytes, i: int) -> tuple[int, int]:
        b = buf[i]
        if b < 0x80:
            return b, i + 1
        n = b & 0x7F
        return int.from_bytes(buf[i + 1 : i + 1 + n], "big"), i + 1 + n

    def read_oid(buf: bytes, i: int) -> tuple[int, int]:
        val = 0
        while True:
            b = buf[i]
            i += 1
            val = (val << 7) | (b & 0x7F)
            if not (b & 0x80):
                return val, i

    body_len, i = read_ber(ls, 16)
    end = i + body_len
    out: list[tuple[int, bytes]] = []
    while i < end:
        start = i
        tag, i = read_oid(ls, i)
        vlen, i = read_ber(ls, i)
        i += vlen
        out.append((tag, ls[start:i]))
    return out


def test_transmux_acceptance_patch_corner_tags_video_byte_faithful(tmp_path):
    from tstrans.klv import decode_uas_datalink, patch_uas_datalink

    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    _write_video_klv_src(src, lat=47.6097)

    EDITED_LAT, EDITED_LON = 37.7749, -122.4194
    # Tag numbers per MISB ST 0601: 23 = frame center lat, 24 = frame
    # center lon, 1 = checksum.
    EDITED_TAGS = {23, 24}

    src_klvs: list[bytes] = []
    with tio.transmux(src, dst) as tx:
        for ev in tx:
            if isinstance(ev, DemuxEvent.Klv):
                src_klvs.append(bytes(ev.payload))
                patched = patch_uas_datalink(
                    bytes(ev.payload),
                    {
                        "frame_center_lat_deg": EDITED_LAT,
                        "frame_center_lon_deg": EDITED_LON,
                    },
                )
                tx.write_klv(ev, patched)
            else:
                tx.write(ev)

    out_videos, out_klvs = _collect(dst)

    # 1) Every video AU byte-identical, count preserved.
    assert out_videos == ORIG_AUS

    # 2) Patched KLV differs ONLY at the edited TLVs + checksum.
    assert len(out_klvs) == len(src_klvs) == len(ORIG_AUS)
    for src_ls, out_ls in zip(src_klvs, out_klvs):
        assert out_ls[:16] == src_ls[:16]  # UL verbatim
        src_tlvs, out_tlvs = _walk_tlvs(src_ls), _walk_tlvs(out_ls)
        assert [t for t, _ in out_tlvs] == [t for t, _ in src_tlvs]
        differing = {
            t_out
            for (t_out, tlv_out), (_, tlv_src) in zip(out_tlvs, src_tlvs)
            if tlv_out != tlv_src
        }
        # Edited TLVs MUST differ; checksum may legitimately differ;
        # nothing else may.
        assert EDITED_TAGS <= differing <= EDITED_TAGS | {1}

        rec = decode_uas_datalink(out_ls)
        assert rec.frame_center_lat_deg == pytest.approx(EDITED_LAT, abs=1e-4)
        assert rec.frame_center_lon_deg == pytest.approx(EDITED_LON, abs=1e-4)
        # Untouched field survives bit-exact (byte-faithful patcher).
        assert rec.sensor_lat_deg == pytest.approx(47.6200, abs=1e-4)


# ---------------------------------------------------------------------------
# v1 scope + error semantics.
# ---------------------------------------------------------------------------

def _write_two_program_src(path: Path) -> None:
    cfg = (
        MuxerConfigBuilder()
        .add_program(
            MuxerProgramConfigBuilder(1, 0x100)
            .add_video(0x101, VideoCodec.H264)
            .build()
        )
        .add_program(
            MuxerProgramConfigBuilder(2, 0x200)
            .add_video(0x201, VideoCodec.H264)
            .build()
        )
        .build()
    )
    mux = Muxer(cfg)
    handles = mux.video_handles()
    with mux.write_file(path) as proxy:
        for i, au in enumerate(ORIG_AUS):
            pts = Pts90khz.from_raw(PTS0 + i * PTS_STEP)
            for h in handles:
                proxy.push_video_to(h, au, pts=pts, key_frame=KEY_FRAMES[i])


def test_transmux_rejects_second_program(tmp_path):
    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    _write_two_program_src(src)
    with pytest.raises(ValueError, match="single-program"):
        with tio.transmux(src, dst) as tx:
            for ev in tx:
                tx.write(ev)


def test_transmux_tolerates_identical_pm_reemission(tmp_path):
    # A mid-file PMT version bump with unchanged content re-emits an
    # identical ProgramMap. The public Muxer emits a fixed PSI version,
    # so that sequence cannot be built from a file — drive the private
    # handler directly with a hand-built duplicate event.
    import dataclasses

    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    _write_video_klv_src(src)
    with tio.transmux(src, dst) as tx:
        for ev in tx:
            tx.write(ev)
            if isinstance(ev, DemuxEvent.ProgramMap):
                first_pm = ev.programs[0]
                # Distinct-but-equal clone: a broken identity-based
                # comparison (`is not`) would reject it, value equality
                # tolerates it — real re-emissions are always fresh
                # objects from the demuxer.
                duplicate = dataclasses.replace(first_pm)
                tx._on_program_map(DemuxEvent.ProgramMap(programs=(duplicate,)))
                changed = dataclasses.replace(first_pm, program_number=7)
                with pytest.raises(ValueError, match="different"):
                    tx._on_program_map(DemuxEvent.ProgramMap(programs=(changed,)))
    out_videos, _ = _collect(dst)
    assert out_videos == ORIG_AUS


def test_transmux_unknown_guard_raises_without_drop_and_no_partial_dst(tmp_path):
    # TEMPORARY guard contract (private-data W2): from_program_map now
    # maps unknown streams to Data specs, but transmux has no
    # UnknownSample routing yet — without the guard the output would
    # declare data PIDs that never receive payload (silent data loss).
    # W3 replaces this ValueError with byte-faithful pass-through.
    from _builders.unknown_stream import build_unknown_stream_ts

    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    src.write_bytes(
        build_unknown_stream_ts(stream_type=0x7F, payload=b"private-bytes")
    )
    with pytest.raises(ValueError, match="drop"):
        with tio.transmux(src, dst) as tx:
            for ev in tx:
                tx.write(ev)
    # The guard raised before the sink opened → no output file.
    assert not dst.exists()


def test_transmux_drop_unknown_succeeds_and_sheds_stream(tmp_path):
    from _builders.unknown_stream import build_unknown_stream_ts

    idr_au = ORIG_AUS[0]
    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    src.write_bytes(
        build_unknown_stream_ts(
            stream_type=0x7F, payload=b"private-bytes", video_au=idr_au
        )
    )
    with tio.transmux(src, dst, drop=(StreamKindTag.UNKNOWN,)) as tx:
        for ev in tx:
            tx.write(ev)  # UnknownSample events hit _SKIP_EVENTS → skipped
    out_videos, _ = _collect(dst)
    assert out_videos == [idr_au]
    # The unknown stream is shed: the output PMT declares no UNKNOWN kind.
    out_pm = tio.probe(dst).programs[0]
    assert all(s.kind is not StreamKindTag.UNKNOWN for s in out_pm.streams)


def test_transmux_drop_klv_skips_dropped_stream_events(tmp_path):
    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    _write_video_klv_src(src)
    with tio.transmux(src, dst, drop=(StreamKindTag.KLV_SYNC,)) as tx:
        for ev in tx:
            tx.write(ev)  # KLV events hit the no-handle path → skipped
            if isinstance(ev, DemuxEvent.Klv):
                # Substituted writes for dropped streams are no-ops too.
                tx.write_klv(ev, bytes(ev.payload))
    out_videos, out_klvs = _collect(dst)
    assert out_videos == ORIG_AUS
    assert out_klvs == []
    assert not tio.probe(dst).has_klv


def test_transmux_write_before_program_map_raises(tmp_path):
    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    _write_video_klv_src(src)
    src_video = next(
        ev for ev in tio.parse_file(src) if isinstance(ev, DemuxEvent.Video)
    )
    with tio.transmux(src, dst) as tx:
        with pytest.raises(RuntimeError, match="no ProgramMap"):
            tx.write(src_video)


def test_transmux_write_after_close_raises(tmp_path):
    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    _write_video_klv_src(src)
    events = []
    with tio.transmux(src, dst) as tx:
        for ev in tx:
            tx.write(ev)
            events.append(ev)
    video = next(ev for ev in events if isinstance(ev, DemuxEvent.Video))
    with pytest.raises(RuntimeError, match="closed"):
        tx.write(video)


def test_transmux_write_rejects_non_events(tmp_path):
    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    _write_video_klv_src(src)
    with tio.transmux(src, dst) as tx:
        for ev in tx:
            tx.write(ev)
        with pytest.raises(TypeError, match="unsupported event type"):
            tx.write(b"not an event")
        with pytest.raises(TypeError, match="expected a DemuxEvent.Klv"):
            tx.write_klv(b"not an event", b"payload")


# ---------------------------------------------------------------------------
# atomic= passes through the MuxerFileSink temp-file + os.replace machinery.
# ---------------------------------------------------------------------------

def test_transmux_atomic_success_promotes_dst(tmp_path):
    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    _write_video_klv_src(src)
    with tio.transmux(src, dst, atomic=True) as tx:
        for ev in tx:
            tx.write(ev)
            # While inside the block, output lives in a *.partial temp.
            assert not dst.exists()
    out_videos, _ = _collect(dst)
    assert out_videos == ORIG_AUS
    assert not list(tmp_path.glob("*.partial"))


def test_transmux_atomic_failure_leaves_nothing(tmp_path):
    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    _write_video_klv_src(src)
    with pytest.raises(RuntimeError, match="boom"):
        with tio.transmux(src, dst, atomic=True) as tx:
            for ev in tx:
                tx.write(ev)
                if isinstance(ev, DemuxEvent.Video):
                    raise RuntimeError("boom")
    assert not dst.exists()
    assert not list(tmp_path.glob("*.partial"))


def test_transmux_iter_and_reenter_after_close_raise(tmp_path):
    # A never-iterated-but-closed transmuxer must NOT start demuxing on
    # a later iter() — the first ProgramMap would open a sink that no
    # __exit__ will ever clean up (stray *.partial / clobbered dst).
    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    _write_video_klv_src(src)
    with tio.transmux(src, dst) as tx:
        pass  # never iterated
    with pytest.raises(RuntimeError, match="closed"):
        iter(tx)
    with pytest.raises(RuntimeError, match="closed"):
        with tx:
            pass
    assert not dst.exists()
    assert not list(tmp_path.glob("*.partial"))


def test_transmux_audio_copied_byte_faithfully(tmp_path):
    from tstrans.mpegts import AudioCodec

    # Smallest valid parseable MPEG-2 AAC-LC 44100 Hz stereo ADTS frame
    # (header only) — same literal as test_codec_aac.py /
    # _builders/build_audio_aac_large.py.
    adts = bytes.fromhex("fff95080021ffc000000000000000000")
    audio_pes = [adts, adts * 2, adts * 3]  # distinct per-PES frame runs

    cfg = (
        MuxerConfigBuilder()
        .add_program(
            MuxerProgramConfigBuilder(1, 0x100)
            .add_video(0x101, VideoCodec.H264)
            .add_audio(0x103, AudioCodec.AAC)
            .build()
        )
        .build()
    )
    src, dst = tmp_path / "src.ts", tmp_path / "out.ts"
    mux = Muxer(cfg)
    with mux.write_file(src) as proxy:
        for i, (au, frames) in enumerate(zip(ORIG_AUS, audio_pes)):
            pts = Pts90khz.from_raw(PTS0 + i * PTS_STEP)
            proxy.push_video(au, pts=pts, key_frame=KEY_FRAMES[i])
            proxy.push_audio(frames, pts=pts)

    with tio.transmux(src, dst) as tx:
        for ev in tx:
            tx.write(ev)

    out_videos: list[bytes] = []
    out_audio: list[bytes] = []
    for ev in tio.parse_file(dst):
        if isinstance(ev, DemuxEvent.Video):
            out_videos.append(bytes(ev.raw))
        elif isinstance(ev, DemuxEvent.Audio):
            out_audio.append(bytes(ev.raw))
    assert out_videos == ORIG_AUS
    assert out_audio == audio_pes
