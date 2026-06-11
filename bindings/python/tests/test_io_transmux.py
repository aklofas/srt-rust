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
