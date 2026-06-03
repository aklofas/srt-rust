"""tstrans.rtp — RTP + RTSP bindings.

Available when tstrans was built with the `rtp` cargo feature
(default-on in published wheels). Source-built without `--features rtp`
will fail to import this submodule with a friendly ImportError.

Submodule contents are populated by `tstrans._native.rtp`:
- `Sender`, `Receiver`, `SocketStats`, `CancelHandle`   (Wave A T20)
- `MuxSender`, `DemuxReceiver`                           (Wave B T23)
- `RtspClient`, `RtspSession`, `BasicAuth`, `DigestAuth`,
  `RtspClientConfig`, `RtspStats`, `RtspCancelHandle`,
  `DigestAlgorithm`, `TransportPref`, `RtspVersion`     (Wave A T21)
- `RtspServer`, `MountHandle`, `RtspServerConfig`,
  `ServerStats`, `MountStats`, `RtspServerCancelHandle` (Wave A T22)
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Optional, Union

try:
    from . import _native
    _rtp = _native.rtp
except (ImportError, AttributeError) as exc:  # pragma: no cover
    raise ImportError(
        "tstrans.rtp is unavailable. Wheels published to PyPI include "
        "RTP by default; if you built tstrans from source, ensure the "
        "`rtp` cargo feature is enabled (it is on by default)."
    ) from exc

# Wave A T20 — RTP transport types.
Sender = _rtp.Sender
Receiver = _rtp.Receiver
SocketStats = _rtp.SocketStats
CancelHandle = _rtp.CancelHandle

# Wave B T23 — MuxSender + DemuxReceiver convenience wrappers.
MuxSender = _rtp.MuxSender
DemuxReceiver = _rtp.DemuxReceiver

# Wave A T21 — RtspClient, RtspSession, auth, config, stats.
# BasicAuth + DigestAuth are PyClass-backed dataclass-equivalents living
# in src/rtp/client.rs (NOT pure-Python). Both T21 client auth and T22
# server auth use these same classes.
RtspClient = _rtp.RtspClient
RtspSession = _rtp.RtspSession
RtspClientConfig = _rtp.RtspClientConfig
RtspStats = _rtp.RtspStats
RtspCancelHandle = _rtp.RtspCancelHandle
BasicAuth = _rtp.BasicAuth
DigestAuth = _rtp.DigestAuth
DigestAlgorithm = _rtp.DigestAlgorithm
TransportPref = _rtp.TransportPref
RtspVersion = _rtp.RtspVersion

# Wave A T22 — RtspServer + MountHandle + server-side stats.
RtspServer = _rtp.RtspServer
MountHandle = _rtp.MountHandle
ServerStats = _rtp.ServerStats
MountStats = _rtp.MountStats
RtspServerCancelHandle = _rtp.RtspServerCancelHandle


# ---------------------------------------------------------------------------
# T22 — RtspServerConfig dataclass. Lives Python-side (not PyClass) because
# the underlying Rust builder takes a stream of fluent setter calls rather
# than a typed config struct; the dataclass is the natural Python shape and
# `RtspServer.start(cfg)` reads its attributes in `src/rtp/server.rs`.
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class RtspServerConfig:
    """Configuration for :py:meth:`RtspServer.start`.

    All fields have sensible defaults; in the common case, callers only
    set `bind_addr` (and optionally `auth` for credentialed servers).
    """

    bind_addr: str = "0.0.0.0:8554"
    """Bind URL or `host:port` (the latter assumed `rtsp://`)."""

    auth: Optional[Union[BasicAuth, DigestAuth]] = None
    """Optional Basic / Digest auth challenge. `None` allows anonymous
    SETUP."""

    max_sessions: int = 100
    """Cap on concurrent client connections."""

    session_timeout_secs: int = 60
    """Advertised session timeout (seconds). Clients are expected to
    keepalive at timeout/2."""

    fanout_capacity: int = 256
    """Per-mount broadcast channel capacity (frames). Slow peers drop
    oldest beyond this; the muxer is never back-pressured."""

    graceful_shutdown_drain_ms: int = 2000
    """Drain window (ms) after `stop()` to let in-flight RTP finish."""

    tls_cert_pem: Optional[bytes] = None
    """PEM-encoded server certificate chain (for `rtsps://` binds).
    Currently raises `RtspError(TLS)` at `start()` time because the
    tst-rtp `tls` feature is not wired through tst-py yet. Field is
    reserved for forward compat."""

    tls_key_pem: Optional[bytes] = None
    """PEM-encoded server private key (for `rtsps://` binds). Same
    reservation as `tls_cert_pem`."""

    def __post_init__(self) -> None:
        if self.max_sessions <= 0:
            raise ValueError(
                f"RtspServerConfig.max_sessions must be > 0; got {self.max_sessions}"
            )
        if self.session_timeout_secs <= 0:
            raise ValueError(
                f"RtspServerConfig.session_timeout_secs must be > 0; "
                f"got {self.session_timeout_secs}"
            )
        if self.fanout_capacity <= 0:
            raise ValueError(
                f"RtspServerConfig.fanout_capacity must be > 0; "
                f"got {self.fanout_capacity}"
            )
        if self.graceful_shutdown_drain_ms < 0:
            raise ValueError(
                f"RtspServerConfig.graceful_shutdown_drain_ms must be >= 0; "
                f"got {self.graceful_shutdown_drain_ms}"
            )
        cert_set = self.tls_cert_pem is not None
        key_set = self.tls_key_pem is not None
        if cert_set != key_set:
            raise ValueError(
                "RtspServerConfig.tls_cert_pem and tls_key_pem must be set together "
                "(both or neither)"
            )


__all__: list[str] = [
    # T20 transport
    "Sender",
    "Receiver",
    "SocketStats",
    "CancelHandle",
    # T23 mux/demux convenience wrappers
    "MuxSender",
    "DemuxReceiver",
    # T21 RTSP client
    "RtspClient",
    "RtspSession",
    "RtspClientConfig",
    "RtspStats",
    "RtspCancelHandle",
    "BasicAuth",
    "DigestAuth",
    "DigestAlgorithm",
    "TransportPref",
    "RtspVersion",
    # T22 RTSP server
    "RtspServer",
    "MountHandle",
    "ServerStats",
    "MountStats",
    "RtspServerCancelHandle",
    "RtspServerConfig",
]
