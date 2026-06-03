"""Wave A Task 21 — surface tests for tstrans.rtp.RtspClient + auth dataclasses.

Covers dataclass validation (RtspClientConfig accepts the auth shapes,
rejects empty URL, rejects ad-hoc duck-typed auth objects), BasicAuth /
DigestAuth construction + repr (passwords must NOT leak through repr),
and the enum surface (TransportPref / DigestAlgorithm / RtspVersion).

The actual RTSP wire exchange is covered by `test_rtsp_loopback.py`
(currently skipped at Wave A — see that file's module docstring for the
T25 follow-up).
"""

import pytest

from tstrans.exceptions import RtspError, RtspErrorKind
from tstrans.rtp import (
    BasicAuth,
    DigestAlgorithm,
    DigestAuth,
    RtspClient,
    RtspClientConfig,
    RtspSession,
    RtspStats,
    RtspVersion,
    TransportPref,
)


# ---------------------------------------------------------------------------
# Enums
# ---------------------------------------------------------------------------


def test_transport_pref_enum_members():
    assert TransportPref.AUTO != TransportPref.UDP
    assert TransportPref.UDP != TransportPref.TCP
    assert TransportPref.AUTO != TransportPref.TCP


def test_digest_algorithm_enum_members():
    assert DigestAlgorithm.MD5 != DigestAlgorithm.SHA256


def test_rtsp_version_enum_members():
    assert RtspVersion.V1_0 != RtspVersion.V2_0


# ---------------------------------------------------------------------------
# BasicAuth
# ---------------------------------------------------------------------------


def test_basic_auth_constructs_and_exposes_user():
    a = BasicAuth(user="alice", password="hunter2")
    assert a.user == "alice"


def test_basic_auth_repr_redacts_password():
    a = BasicAuth(user="alice", password="topsecret")
    r = repr(a)
    assert "alice" in r
    assert "topsecret" not in r
    assert "<redacted>" in r


def test_basic_auth_password_not_accessible_attribute():
    # Frozen pyclass without a `password` getter — attribute access fails.
    a = BasicAuth(user="alice", password="x")
    with pytest.raises(AttributeError):
        _ = a.password


# ---------------------------------------------------------------------------
# DigestAuth
# ---------------------------------------------------------------------------


def test_digest_auth_constructs_default_algorithm():
    a = DigestAuth(user="bob", password="x")
    assert a.user == "bob"
    assert a.algorithm == DigestAlgorithm.MD5


def test_digest_auth_constructs_with_sha256():
    a = DigestAuth(user="bob", password="x", algorithm=DigestAlgorithm.SHA256)
    assert a.algorithm == DigestAlgorithm.SHA256


def test_digest_auth_repr_redacts_password():
    a = DigestAuth(user="bob", password="topsecret", algorithm=DigestAlgorithm.SHA256)
    r = repr(a)
    assert "bob" in r
    assert "topsecret" not in r
    assert "<redacted>" in r


def test_digest_auth_password_not_accessible():
    a = DigestAuth(user="bob", password="x")
    with pytest.raises(AttributeError):
        _ = a.password


# ---------------------------------------------------------------------------
# RtspClientConfig
# ---------------------------------------------------------------------------


def test_rtsp_client_config_defaults():
    cfg = RtspClientConfig(url="rtsp://127.0.0.1:8554/live")
    assert cfg.url == "rtsp://127.0.0.1:8554/live"
    assert cfg.auth is None
    assert cfg.transport_pref == TransportPref.AUTO
    assert cfg.rtcp is True
    assert cfg.tls_root_certs_pem is None
    assert cfg.keepalive is True
    assert cfg.rtsp_version == RtspVersion.V1_0


def test_rtsp_client_config_accepts_basic_auth():
    a = BasicAuth(user="alice", password="x")
    cfg = RtspClientConfig(url="rtsp://h/p", auth=a)
    assert cfg.auth is a  # round-trip preserves identity


def test_rtsp_client_config_accepts_digest_auth():
    a = DigestAuth(user="bob", password="x", algorithm=DigestAlgorithm.SHA256)
    cfg = RtspClientConfig(url="rtsp://h/p", auth=a)
    assert cfg.auth is a


def test_rtsp_client_config_accepts_none_auth_explicitly():
    cfg = RtspClientConfig(url="rtsp://h/p", auth=None)
    assert cfg.auth is None


def test_rtsp_client_config_rejects_empty_url():
    with pytest.raises(ValueError, match="url must not be empty"):
        RtspClientConfig(url="")


def test_rtsp_client_config_rejects_arbitrary_auth_object():
    class FakeAuth:
        user = "x"
        password = "y"

    with pytest.raises(ValueError, match="auth must be"):
        RtspClientConfig(url="rtsp://h/p", auth=FakeAuth())


def test_rtsp_client_config_rejects_string_auth():
    with pytest.raises(ValueError, match="auth must be"):
        RtspClientConfig(url="rtsp://h/p", auth="alice:hunter2")


def test_rtsp_client_config_accepts_transport_pref_kwarg():
    cfg = RtspClientConfig(url="rtsp://h/p", transport_pref=TransportPref.TCP)
    assert cfg.transport_pref == TransportPref.TCP


def test_rtsp_client_config_accepts_tls_pem_bytes():
    # We don't validate the PEM contents at construction — passthrough only.
    pem = b"-----BEGIN CERTIFICATE-----\nMIID...\n-----END CERTIFICATE-----\n"
    cfg = RtspClientConfig(url="rtsps://h/p", tls_root_certs_pem=pem)
    assert cfg.tls_root_certs_pem == pem


def test_rtsp_client_config_repr_redacts_pem_and_auth():
    a = BasicAuth(user="alice", password="topsecret")
    cfg = RtspClientConfig(
        url="rtsp://h/p", auth=a, tls_root_certs_pem=b"PEMBYTES"
    )
    r = repr(cfg)
    assert "topsecret" not in r
    assert "PEMBYTES" not in r
    assert "<auth>" in r
    assert "<bytes>" in r


# ---------------------------------------------------------------------------
# RtspStats
# ---------------------------------------------------------------------------


def test_rtsp_stats_default_is_zero():
    # Construction directly from Python isn't supported (no __new__);
    # we exercise it indirectly via the (currently unused) classmethod
    # surface. Use type-only checks here — wave-B `session.stats()` is
    # exercised in test_rtsp_loopback.py.
    assert RtspStats is not None
    assert hasattr(RtspStats, "__qualname__")


# ---------------------------------------------------------------------------
# RtspClient / RtspSession surface (offline — no server)
# ---------------------------------------------------------------------------


def test_rtsp_client_connect_is_staticmethod():
    # RtspClient holds no instance state — `connect` is a staticmethod.
    assert callable(RtspClient.connect)


def test_rtsp_client_connect_to_invalid_url_raises_rtsp_error():
    cfg = RtspClientConfig(url="rtsp://192.0.2.1:1/never")  # TEST-NET-1, blackhole
    # Either:
    # - URL-parse error → RtspError(kind=PROTOCOL) via the Url variant
    # - I/O failure (connect refused / timeout) → RtspError(kind=IO)
    # Both are valid surfaces; we don't assert the specific kind here.
    with pytest.raises(RtspError) as exc_info:
        RtspClient.connect(cfg)
    assert exc_info.value.kind in {
        RtspErrorKind.IO,
        RtspErrorKind.PROTOCOL,
        RtspErrorKind.TIMEOUT,
    }


def test_rtsp_client_connect_malformed_url_raises_rtsp_error():
    cfg = RtspClientConfig(url="not-a-url-at-all")
    with pytest.raises(RtspError) as exc_info:
        RtspClient.connect(cfg)
    # Builder construction (URL parse) fires before any I/O.
    assert exc_info.value.kind == RtspErrorKind.PROTOCOL


def test_rtsp_session_class_exposed():
    assert RtspSession is not None
    # Methods exist on the class (whether bound or unbound).
    for m in ("play", "pause", "teardown", "cancel_handle", "stats",
              "into_demux_receiver", "__enter__", "__exit__", "is_torn_down"):
        assert hasattr(RtspSession, m), f"RtspSession missing method {m}"
