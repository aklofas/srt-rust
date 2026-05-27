"""Wave A Task 22 — tstrans.rtp.RtspServer / MountHandle / RtspServerConfig.

Covers:
- RtspServerConfig dataclass validation (max_sessions / fanout_capacity /
  TLS-half-pair / drain_ms invariants).
- RtspServer.start lifecycle + context-manager + local_addr.
- add_unicast_mount / add_multicast_mount round-trip.
- MountHandle push family (video / klv / audio / subtitle) — single
  stream variants accept bytes / bytearray / memoryview.
- MountHandle stats() returns a frozen ServerStats / MountStats with
  expected fields.
- cancel_handle round-trip.
- Notice 5402 graceful shutdown path doesn't panic.
"""

from __future__ import annotations

import pytest

from tstrans.exceptions import RtspError, RtspErrorKind
from tstrans.mpegts import (
    AudioCodec,
    KlvStreamType,
    MuxerProgramConfigBuilder,
    Pts90khz,
    VideoCodec,
)
from tstrans.rtp import (
    BasicAuth,
    DigestAuth,
    MountHandle,
    MountStats,
    RtspServer,
    RtspServerCancelHandle,
    RtspServerConfig,
    ServerStats,
)


# ---------------------------------------------------------------------------
# RtspServerConfig dataclass validation.
# ---------------------------------------------------------------------------


def test_rtsp_server_config_defaults():
    cfg = RtspServerConfig()
    assert cfg.bind_addr == "0.0.0.0:8554"
    assert cfg.auth is None
    assert cfg.max_sessions == 100
    assert cfg.session_timeout_secs == 60
    assert cfg.fanout_capacity == 256
    assert cfg.graceful_shutdown_drain_ms == 2000
    assert cfg.tls_cert_pem is None
    assert cfg.tls_key_pem is None


def test_rtsp_server_config_rejects_zero_max_sessions():
    with pytest.raises(ValueError, match="max_sessions"):
        RtspServerConfig(max_sessions=0)


def test_rtsp_server_config_rejects_negative_max_sessions():
    with pytest.raises(ValueError, match="max_sessions"):
        RtspServerConfig(max_sessions=-5)


def test_rtsp_server_config_rejects_zero_session_timeout():
    with pytest.raises(ValueError, match="session_timeout_secs"):
        RtspServerConfig(session_timeout_secs=0)


def test_rtsp_server_config_rejects_zero_fanout_capacity():
    with pytest.raises(ValueError, match="fanout_capacity"):
        RtspServerConfig(fanout_capacity=0)


def test_rtsp_server_config_rejects_negative_drain_ms():
    with pytest.raises(ValueError, match="graceful_shutdown_drain_ms"):
        RtspServerConfig(graceful_shutdown_drain_ms=-1)


def test_rtsp_server_config_rejects_tls_cert_without_key():
    with pytest.raises(ValueError, match="tls_cert_pem and tls_key_pem"):
        RtspServerConfig(tls_cert_pem=b"---CERT---", tls_key_pem=None)


def test_rtsp_server_config_rejects_tls_key_without_cert():
    with pytest.raises(ValueError, match="tls_cert_pem and tls_key_pem"):
        RtspServerConfig(tls_cert_pem=None, tls_key_pem=b"---KEY---")


def test_rtsp_server_config_accepts_both_tls_pem_set():
    # Validation passes; start() will reject because TLS feature is off
    # — that's tested separately below.
    cfg = RtspServerConfig(tls_cert_pem=b"---CERT---", tls_key_pem=b"---KEY---")
    assert cfg.tls_cert_pem == b"---CERT---"
    assert cfg.tls_key_pem == b"---KEY---"


def test_rtsp_server_config_accepts_basic_auth():
    # BasicAuth comes from T21 (src/rtp/client.rs PyClass) — same instance
    # type used by both RtspClientConfig.auth (client-side credentials) and
    # RtspServerConfig.auth (server-side challenge). T21's BasicAuth shape
    # is (user, password); the `realm` to advertise on WWW-Authenticate is
    # a server-side concern Wave C T25 will plumb through alongside the
    # end-to-end server-with-auth tests.
    auth = BasicAuth(user="admin", password="hunter2")
    cfg = RtspServerConfig(auth=auth)
    assert cfg.auth is auth


def test_rtsp_server_config_accepts_digest_auth():
    # Same T21-canonical shape: (user, password, algorithm). `algorithm`
    # is the PyDigestAlgorithm enum (MD5 / SHA256), not a string.
    from tstrans.rtp import DigestAlgorithm
    auth = DigestAuth(user="admin", password="hunter2")
    cfg = RtspServerConfig(auth=auth)
    assert cfg.auth.algorithm == DigestAlgorithm.MD5


def test_digest_auth_accepts_sha256():
    from tstrans.rtp import DigestAlgorithm
    a = DigestAuth(user="x", password="x", algorithm=DigestAlgorithm.SHA256)
    assert a.algorithm == DigestAlgorithm.SHA256


# T22-era placeholder tests `test_digest_auth_rejects_unknown_algorithm` and
# `test_digest_auth_accepts_sha256_variants` (string-based algorithm) were
# removed at the T20+T21+T22 merge — the algorithm is now a typed enum so
# unknown strings cannot reach the constructor (Python raises TypeError on
# bad enum extract before `__new__` body runs).


# ---------------------------------------------------------------------------
# Helpers — build a minimal MuxerProgramConfig with one H.264 video stream.
# ---------------------------------------------------------------------------


def _single_video_program():
    return (
        MuxerProgramConfigBuilder(1, 0x1000)
        .add_video(0x1011, VideoCodec.H264)
        .build()
    )


def _video_klv_program():
    return (
        MuxerProgramConfigBuilder(1, 0x1000)
        .add_video(0x1011, VideoCodec.H264)
        .add_klv(0x1012, KlvStreamType.SYNCHRONOUS_METADATA, carries_pts=True)
        .build()
    )


def _video_audio_program():
    return (
        MuxerProgramConfigBuilder(1, 0x1000)
        .add_video(0x1011, VideoCodec.H264)
        .add_audio(0x1012, AudioCodec.AAC)
        .build()
    )


# ---------------------------------------------------------------------------
# Lifecycle — start / context manager / local_addr / stop.
# ---------------------------------------------------------------------------


def test_server_start_returns_server():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", max_sessions=2)
    server = RtspServer.start(cfg)
    try:
        assert isinstance(server, RtspServer)
        addr = server.local_addr()
        assert addr is not None
        assert addr.startswith("127.0.0.1:")
    finally:
        server.stop()


def test_server_context_manager_stops_on_exit():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        assert server.local_addr() is not None
    # No assertion needed — `with` exiting cleanly is the assertion.


def test_server_stats_initially_zero():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        stats = server.stats()
        assert isinstance(stats, ServerStats)
        assert stats.active_sessions == 0
        assert stats.mounts == 0
        assert stats.total_rtp_packets_sent == 0
        assert stats.total_rtp_bytes_sent == 0


def test_server_cancel_handle_round_trip():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        h1 = server.cancel_handle()
        h2 = server.cancel_handle()
        assert isinstance(h1, RtspServerCancelHandle)
        assert not h1.is_cancelled()
        h1.cancel()
        assert h2.is_cancelled()


def test_server_tls_pem_rejected_when_feature_off():
    # tst-py's tst-rtp dep is built without the `tls` feature → start()
    # must reject any TLS PEM bytes with `RtspError(TLS)`.
    cfg = RtspServerConfig(
        bind_addr="127.0.0.1:0",
        tls_cert_pem=b"---BEGIN CERT---",
        tls_key_pem=b"---BEGIN KEY---",
    )
    with pytest.raises(RtspError) as exc_info:
        RtspServer.start(cfg)
    assert exc_info.value.kind == RtspErrorKind.TLS


# ---------------------------------------------------------------------------
# add_unicast_mount + add_multicast_mount.
# ---------------------------------------------------------------------------


def test_add_unicast_mount_returns_handle():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        program = _single_video_program()
        mount = server.add_unicast_mount("/live", program)
        assert isinstance(mount, MountHandle)
        assert mount.mount_path() == "/live"
        assert mount.mount_kind() == "unicast"
        assert server.stats().mounts == 1


def test_add_unicast_mount_rejects_empty_path():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        program = _single_video_program()
        with pytest.raises(RtspError) as exc_info:
            server.add_unicast_mount("", program)
        assert exc_info.value.kind == RtspErrorKind.MOUNT


def test_add_unicast_mount_rejects_path_without_leading_slash():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        program = _single_video_program()
        with pytest.raises(RtspError) as exc_info:
            server.add_unicast_mount("live", program)
        assert exc_info.value.kind == RtspErrorKind.MOUNT


def test_add_unicast_mount_rejects_duplicate_path():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        program = _single_video_program()
        server.add_unicast_mount("/live", program)
        with pytest.raises(RtspError) as exc_info:
            server.add_unicast_mount("/live", _single_video_program())
        assert exc_info.value.kind == RtspErrorKind.MOUNT


def test_add_multicast_mount_returns_handle():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        program = _single_video_program()
        mount = server.add_multicast_mount(
            "/mc", "239.0.0.1", 5004, ttl=1, program_config=program
        )
        assert mount.mount_path() == "/mc"
        assert mount.mount_kind() == "multicast"


def test_add_multicast_mount_with_iface():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        program = _single_video_program()
        mount = server.add_multicast_mount(
            "/mc", "239.0.0.1", 5004, ttl=2, iface="127.0.0.1",
            program_config=program,
        )
        assert mount.mount_path() == "/mc"


def test_add_multicast_mount_rejects_unicast_group():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        program = _single_video_program()
        with pytest.raises(RtspError) as exc_info:
            server.add_multicast_mount(
                "/mc", "10.0.0.1", 5004, program_config=program
            )
        assert exc_info.value.kind == RtspErrorKind.MOUNT


# ---------------------------------------------------------------------------
# MountHandle push family.
# ---------------------------------------------------------------------------


# Annex-B IDR NAL: `00 00 00 01 65 BB` — type 5 (IDR) + filler payload.
# Reliably produces a drained TS chunk on first push.
_IDR_NAL = bytes([0x00, 0x00, 0x00, 0x01, 0x65, 0xBB])


def test_push_video_succeeds_with_no_peers():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        mount = server.add_unicast_mount("/live", _single_video_program())
        # Pre-PLAY: no peers; push should still succeed (muxer consumes,
        # broadcast::send returns Err which the Rust side suppresses).
        mount.push_video(_IDR_NAL, pts=Pts90khz(0), key_frame=True)


def test_push_video_accepts_bytearray():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        mount = server.add_unicast_mount("/live", _single_video_program())
        mount.push_video(bytearray(_IDR_NAL), pts=Pts90khz(0), key_frame=True)


def test_push_video_accepts_memoryview():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        mount = server.add_unicast_mount("/live", _single_video_program())
        mount.push_video(
            memoryview(_IDR_NAL), pts=Pts90khz(0), key_frame=True
        )


def test_push_video_updates_mount_stats():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        mount = server.add_unicast_mount("/live", _single_video_program())
        initial = mount.stats()
        assert initial.bytes_pushed == 0
        mount.push_video(_IDR_NAL, pts=Pts90khz(0), key_frame=True)
        after = mount.stats()
        assert after.bytes_pushed > 0
        assert after.packets_pushed >= 1


def test_push_klv_succeeds():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        mount = server.add_unicast_mount("/live", _video_klv_program())
        # Minimal KLV: 16-byte UL + 1-byte length(0).
        klv = b"\x06\x0e\x2b\x34\x02\x0b\x01\x01\x0e\x01\x03\x01\x01\x00\x00\x00\x00"
        mount.push_klv(klv, pts=Pts90khz(0))


def test_push_audio_succeeds():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        mount = server.add_unicast_mount("/live", _video_audio_program())
        # Smallest plausible ADTS header: syncword + payload. AAC parser
        # will likely reject, but the call must marshal correctly through
        # GIL release.
        adts = bytes([0xFF, 0xF1, 0x4C, 0x80, 0x01, 0x3F, 0xFC]) + bytes(20)
        try:
            mount.push_audio(adts, pts=Pts90khz(0))
        except RtspError as e:
            # Codec parse may reject; that's a MOUNT-mapped error and
            # confirms the path works end-to-end.
            assert e.kind == RtspErrorKind.MOUNT


# ---------------------------------------------------------------------------
# Handle getters.
# ---------------------------------------------------------------------------


def test_video_handle_returns_for_configured_program():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        mount = server.add_unicast_mount("/live", _single_video_program())
        h = mount.video_handle()
        assert h is not None


def test_klv_handle_returns_none_for_video_only_program():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        mount = server.add_unicast_mount("/live", _single_video_program())
        assert mount.klv_handle() is None
        assert mount.audio_handle() is None
        assert mount.subtitle_handle() is None


def test_push_video_to_with_explicit_handle():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        mount = server.add_unicast_mount("/live", _single_video_program())
        h = mount.video_handle()
        assert h is not None
        mount.push_video_to(h, _IDR_NAL, pts=Pts90khz(0), key_frame=True)


# ---------------------------------------------------------------------------
# ServerStats + MountStats dataclass shape.
# ---------------------------------------------------------------------------


def test_mount_stats_has_expected_fields():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        mount = server.add_unicast_mount("/live", _single_video_program())
        s = mount.stats()
        assert isinstance(s, MountStats)
        for attr in ("bytes_pushed", "packets_pushed", "peer_count", "frames_dropped_total"):
            assert hasattr(s, attr)
        assert s.peer_count == 0
        assert s.frames_dropped_total == 0


def test_server_stats_has_expected_fields():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        s = server.stats()
        for attr in (
            "active_sessions",
            "mounts",
            "total_rtp_packets_sent",
            "total_rtp_bytes_sent",
        ):
            assert hasattr(s, attr)


# ---------------------------------------------------------------------------
# stop() + Notice 5402 path.
# ---------------------------------------------------------------------------


def test_stop_is_idempotent():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    server = RtspServer.start(cfg)
    server.stop()
    server.stop()  # No-op; must not raise.


def test_stop_after_mount_does_not_panic():
    # Notice 5402 path runs over each active session; with zero sessions
    # the loop body is skipped but the cancel cascade still fires. The
    # graceful_shutdown_drain_ms=50 keeps the test fast.
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    server = RtspServer.start(cfg)
    server.add_unicast_mount("/live", _single_video_program())
    server.stop()


# ---------------------------------------------------------------------------
# flush + reset_stats.
# ---------------------------------------------------------------------------


def test_flush_is_noop_on_empty_mount():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        mount = server.add_unicast_mount("/live", _single_video_program())
        mount.flush()  # must not raise


def test_reset_stats_zeros_counters():
    cfg = RtspServerConfig(bind_addr="127.0.0.1:0", graceful_shutdown_drain_ms=50)
    with RtspServer.start(cfg) as server:
        mount = server.add_unicast_mount("/live", _single_video_program())
        mount.push_video(_IDR_NAL, pts=Pts90khz(0), key_frame=True)
        assert mount.stats().bytes_pushed > 0
        mount.reset_stats()
        assert mount.stats().bytes_pushed == 0
        assert mount.stats().packets_pushed == 0
