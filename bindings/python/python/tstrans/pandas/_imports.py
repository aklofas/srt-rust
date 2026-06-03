"""Centralized numpy/pandas import gate for tstrans.pandas adapters.

The [pandas] extra is optional. Every adapter calls require_pandas() to
gate access; if pandas/numpy aren't installed, the function raises a
friendly ImportError with the install hint.

Imports are cached after the first successful call (single-digit
microsecond overhead on subsequent calls).
"""

from typing import Any, Tuple

_pd: Any = None
_np: Any = None


def require_pandas() -> Tuple[Any, Any]:
    """Returns (pandas, numpy). Raises ImportError if [pandas] extra not installed.

    Raises:
        ImportError: with canonical install hint when pandas or numpy missing.
    """
    global _pd, _np
    if _pd is not None and _np is not None:
        return _pd, _np
    try:
        import pandas as pd
        import numpy as np
    except ImportError as e:
        raise ImportError(
            "tstrans pandas adapters require the [pandas] extra. "
            "Install: pip install 'tstrans[pandas]'"
        ) from e
    _pd = pd
    _np = np
    return _pd, _np
