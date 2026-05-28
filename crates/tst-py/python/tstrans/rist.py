"""tstrans.rist — tst-rist bindings (Plan A5b Wave D).

Available when tstrans was built with the `rist` cargo feature (default-on
in published wheels). A source build without `--features rist` will fail to
import this submodule.

Classes are implemented in `tstrans._native.rist` (Rust/PyO3) and
re-exported here so callers use `from tstrans import rist` and access
`rist.Transport`, `rist.RecvTransport`, etc.
"""

from __future__ import annotations

from . import _native
from .exceptions import RistError, RistErrorKind

_rist = _native.rist

# Re-export native classes from the compiled extension module.
RistProfile = _rist.RistProfile
RistStats = _rist.RistStats
EncryptionKey = _rist.EncryptionKey
Transport = _rist.Transport
TransportBuilder = _rist.TransportBuilder
RecvTransport = _rist.RecvTransport
RecvTransportBuilder = _rist.RecvTransportBuilder

__all__ = [
    "RistProfile",
    "RistStats",
    "EncryptionKey",
    "Transport",
    "TransportBuilder",
    "RecvTransport",
    "RecvTransportBuilder",
    "RistError",
    "RistErrorKind",
]
