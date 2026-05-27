"""tstrans — Python bindings for the ts-transformer MPEG-TS + KLV + codec parsers.

This package re-exports types from the compiled `tstrans._native` extension
module, organized into topic-focused submodules:

- `tstrans.mpegts` — MPEG-TS packet / PES / PSI / mux / demux
- `tstrans.klv` — KLV typed sets (ST 0601, ST 0102, ST 0903, ...)
- `tstrans.codec` — H.264 / H.265 / H.266 / AV1 / AAC frame parsers
- `tstrans.io` — convenience helpers for reading and writing .ts files
- `tstrans.exceptions` — exception hierarchy raised by the above

See the per-submodule docstrings for usage.
"""

from tstrans import _native, codec, exceptions, io, klv, mpegts
from tstrans.exceptions import TstError

# `rtp` import is conditional: the `tstrans.rtp` submodule raises a
# friendly ImportError when the `rtp` cargo feature was off at build
# time (default-on, so published wheels always include it). We import
# it unconditionally here to surface the failure at package-import
# time rather than first-use time.
try:
    from tstrans import rtp  # noqa: F401
    _RTP_AVAILABLE = True
except ImportError:
    _RTP_AVAILABLE = False

__version__: str = _native.__version__

__all__: list[str] = [
    "__version__",
    "TstError",
    "codec",
    "exceptions",
    "io",
    "klv",
    "mpegts",
]
if _RTP_AVAILABLE:
    __all__.append("rtp")
