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
- `RtspServer`, `MountHandle`, `RtspServerConfig`, etc. (Wave A T22)
"""

try:
    from . import _native
    _rtp = _native.rtp
except (ImportError, AttributeError) as exc:
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

# Wave A T21 — RtspClient, RtspSession, auth, config, stats.
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

__all__: list[str] = [
    # T20 transport
    "Sender",
    "Receiver",
    "SocketStats",
    "CancelHandle",
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
]
