"""tstrans.rist — tst-rist bindings.

Available when tstrans was built with the `rist` cargo feature (default-on
in published wheels). A source build without `--features rist` will fail to
import this submodule.

Contents are populated by `tstrans._native.rist` (Plan A5b). The native
submodule's classes are re-exported here as the wave lands them.
"""

from __future__ import annotations

from . import _native

_rist = _native.rist

# Wave rist re-exports the native classes here (Sender / RecvTransport /
# builders / stats / etc.). Until then the native submodule is empty.

__all__: list[str] = []
