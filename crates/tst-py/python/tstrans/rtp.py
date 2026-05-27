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

# Re-exports populated by Wave A / Wave B tasks. Kept empty in the
# scaffold so `from tstrans.rtp import *` is a documented no-op until
# real types land.
__all__: list[str] = []
