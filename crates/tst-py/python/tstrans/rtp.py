"""tstrans.rtp — RTP + RTSP bindings.

Available when tstrans was built with the `rtp` cargo feature
(default-on in published wheels). Source-built without `--features rtp`
will fail to import this submodule with a friendly ImportError.

Submodule contents are populated by `tstrans._native.rtp`:
- `Sender`, `Receiver`, `SocketStats`, `CancelHandle`   (Wave A)
- `MuxSender`, `DemuxReceiver`                           (Wave B)
- `RtspClient`, `RtspSession`, `BasicAuth`, `DigestAuth`,
  `RtspClientConfig`                                     (Wave A)
- `RtspServer`, `MountHandle`, `RtspServerConfig`        (Wave A)
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

# Re-exports populated incrementally as Wave A / Wave B tasks land their
# PyO3 types in `tstrans._native.rtp`. Wave A Task 20 lands the four
# transport-level types below; subsequent tasks add `MuxSender`,
# `DemuxReceiver`, `RtspClient`, `RtspServer`, `MountHandle`, etc.
Sender = _rtp.Sender
Receiver = _rtp.Receiver
SocketStats = _rtp.SocketStats
CancelHandle = _rtp.CancelHandle

__all__: list[str] = [
    "Sender",
    "Receiver",
    "SocketStats",
    "CancelHandle",
]
