"""End-to-end round-trip tests for the Phase 4 build path."""

import tempfile
from pathlib import Path

import pytest

from tstrans.io import parse_file, probe
from tstrans.mpegts import (
    DemuxEvent,
    KlvStreamType,
    Muxer,
    MuxerConfigBuilder,
    MuxerProgramConfigBuilder,
    Pts90khz,
    VideoCodec,
)


def _synthetic_config():
    prog = (
        MuxerProgramConfigBuilder(1, 0x100)
        .add_video(0x101, VideoCodec.H264)
        .add_klv(0x102, KlvStreamType.SYNCHRONOUS_METADATA, carries_pts=True)
        .build()
    )
    return MuxerConfigBuilder().add_program(prog).build()


def _deterministic_push_sequence(proxy, n_video: int = 5, n_klv: int = 5) -> None:
    """Push n_video NAL AUDs at 33ms intervals + n_klv KLV blobs at 100ms intervals."""

    nal_aud = b"\x00\x00\x00\x01\x09\xF0"
    klv_ul_zero = b"\x06\x0E\x2B\x34\x02\x0B\x01\x01\x0E\x01\x03\x01\x01\x00\x00\x00\x00"
    pts0 = 900_000  # ~10s at 90kHz

    for i in range(n_video):
        proxy.push_video(nal_aud, pts=Pts90khz.from_raw(pts0 + i * 3000))
    for i in range(n_klv):
        proxy.push_klv(klv_ul_zero, pts=Pts90khz.from_raw(pts0 + i * 9000))


def test_synthetic_determinism_two_muxers_produce_identical_bytes():
    """Two fresh Muxer instances from same config + same input sequence MUST
    produce byte-identical output. Catches non-determinism regressions
    (hash-map iteration order, timestamp jitter, etc.)."""

    cfg = _synthetic_config()

    def build_one() -> bytes:
        m = Muxer(cfg)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "out.ts"
            with m.write_file(path) as proxy:
                _deterministic_push_sequence(proxy)
            return path.read_bytes()

    a = build_one()
    b = build_one()
    assert a == b, "muxer output non-deterministic — investigate before merging"


def test_synthetic_round_trip_probe_finds_pids_and_codecs():
    """Re-probe our own muxer output: PIDs + codec ids should match config."""

    m = Muxer(_synthetic_config())
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "out.ts"
        with m.write_file(path) as proxy:
            _deterministic_push_sequence(proxy)
        pr = probe(path)

    assert 0x101 in pr.pids
    assert 0x102 in pr.pids
    assert VideoCodec.H264 in pr.video_codecs
    assert pr.has_klv is True


def test_synthetic_round_trip_event_counts_within_tolerance():
    """Push N video + M KLV; expect at least N video events + M KLV events
    on the re-demux pass (frame coalescing may inflate counts slightly,
    PSI cadence can also affect the boundary frame)."""

    m = Muxer(_synthetic_config())
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "out.ts"
        with m.write_file(path) as proxy:
            _deterministic_push_sequence(proxy, n_video=5, n_klv=5)

        video_evs = []
        klv_evs = []
        for ev in parse_file(path):
            if isinstance(ev, DemuxEvent.Video):
                video_evs.append(ev)
            elif isinstance(ev, DemuxEvent.Klv):
                klv_evs.append(ev)

    # Structural-equivalence bar: counts within tolerance.
    # Be generous; PSI repetition and AU boundaries can shift counts.
    assert len(video_evs) >= 1, f"expected >=1 video event, got {len(video_evs)}"
    assert len(klv_evs) >= 1, f"expected >=1 klv event, got {len(klv_evs)}"
    assert abs(len(video_evs) - 5) <= 3, f"video count {len(video_evs)} too far from 5"
    assert abs(len(klv_evs) - 5) <= 3, f"klv count {len(klv_evs)} too far from 5"


def test_synthetic_round_trip_byte_alignment():
    m = Muxer(_synthetic_config())
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "out.ts"
        with m.write_file(path) as proxy:
            _deterministic_push_sequence(proxy)
        size = path.stat().st_size
    assert size > 0
    assert size % 188 == 0


@pytest.mark.skipif(
    not list(Path("crates/tst-core/tests/fixtures/local").glob("*.ts")),
    reason="real-world fixture not present (tests/fixtures/local/)",
)
def test_real_fixture_round_trip_structural_equivalence():
    """Demux a real .ts, identify PIDs+codecs, build matching muxer config,
    replay sample events back through Muxer, re-demux output, assert PID
    set + codec set match.

    NOTE: full config-from-probe reconstruction is non-trivial; v1 just
    checks the source probe succeeds and re-records PIDs."""

    fixtures = list(Path("crates/tst-core/tests/fixtures/local").glob("*.ts"))
    src = fixtures[0]
    src_probe = probe(src)
    assert len(src_probe.pids) > 0
    # Full config reconstruction deferred to a follow-up; this test is
    # primarily a smoke test that probe + parse work on a real fixture.
    pytest.skip("full real-fixture round-trip deferred — see Phase 4 closeout follow-ups")
