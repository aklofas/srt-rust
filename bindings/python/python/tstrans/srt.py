"""tstrans.srt — SRT transport bindings.

Available when tstrans was built with the `srt` cargo feature
(default-on in published wheels). Source-built without `--features srt`
will fail to import this submodule with a friendly ImportError.

Submodule contents are populated by `tstrans._native.srt`:
- `Sender`, `Receiver`, `SocketStats`, `SrtStats`, `CancelHandle` (Wave A T2)
- `Socket`, `Listener`, `Builder`                                  (Wave A T3)
- `MuxSender`, `DemuxReceiver`                                     (Wave B T5)
- `ReconnectPolicy`, `BackoffStrategy`, `OverflowPolicy`           (Wave B T6)
- `ManagedSender`, `ManagedReceiver`, `ManagedMuxSender`,
  `ManagedDemuxReceiver`                                           (Wave C T7+T8)
"""

from __future__ import annotations

try:
    from . import _native
    _srt = _native.srt
except (ImportError, AttributeError) as exc:  # pragma: no cover
    raise ImportError(
        "tstrans.srt is unavailable. Wheels published to PyPI include "
        "SRT by default; if you built tstrans from source, ensure the "
        "`srt` cargo feature is enabled (it is on by default)."
    ) from exc

# Wave A T2 — transport-layer types.
Sender = _srt.Sender
Receiver = _srt.Receiver
SocketStats = _srt.SocketStats
SrtStats = _srt.SrtStats
CancelHandle = _srt.CancelHandle

# Wave A T3 — low-level primitives.
Builder = _srt.Builder
Socket = _srt.Socket
Listener = _srt.Listener

# Wave B T5 — MuxSender + DemuxReceiver convenience wrappers.
MuxSender = _srt.MuxSender
DemuxReceiver = _srt.DemuxReceiver

# Wave B T6 — reconnect policy ergonomics.
BackoffStrategy = _srt.BackoffStrategy
OverflowPolicy = _srt.OverflowPolicy
ReconnectPolicy = _srt.ReconnectPolicy

# Wave C T7 — auto-reconnect basic-bytes wrappers.
ManagedSender = _srt.ManagedSender
ManagedReceiver = _srt.ManagedReceiver

# Wave C T8 — auto-reconnect MuxSender + DemuxReceiver convenience wrappers.
ManagedMuxSender = _srt.ManagedMuxSender
ManagedDemuxReceiver = _srt.ManagedDemuxReceiver


__all__: list[str] = [
    # T2 transport
    "Sender",
    "Receiver",
    "SocketStats",
    "SrtStats",
    "CancelHandle",
    # T3 low-level
    "Builder",
    "Socket",
    "Listener",
    # T5 mux/demux convenience wrappers
    "MuxSender",
    "DemuxReceiver",
    # T6 policy
    "BackoffStrategy",
    "OverflowPolicy",
    "ReconnectPolicy",
    # T7 managed basic
    "ManagedSender",
    "ManagedReceiver",
    # T8 managed convenience wrappers
    "ManagedMuxSender",
    "ManagedDemuxReceiver",
]
