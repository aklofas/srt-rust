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

__version__: str = _native.__version__

__all__ = [
    "__version__",
    "TstError",
    "codec",
    "exceptions",
    "io",
    "klv",
    "mpegts",
]
