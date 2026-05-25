"""tstrans.pandas — pandas DataFrame adapters for tstrans types.

Requires the [pandas] extra:
    pip install 'tstrans[pandas]'

Importing this submodule does NOT trigger pandas/numpy import. Only the
adapter function calls trigger the gated import. Calling an adapter
without the extra raises ImportError with the install hint.
"""

# Adapter submodules are imported eagerly here (Python's module-level
# imports), but they DO NOT import pandas/numpy themselves — those
# imports are gated inside each adapter function via require_pandas().
# Importing `tstrans.pandas` therefore does not trigger pandas or
# numpy installation/import. Only calling an adapter does.

from tstrans.pandas.klv import klv_to_dataframe
from tstrans.pandas.events import events_to_dataframe
from tstrans.pandas.codec import (
    audio_frames_to_dataframe,
    nals_to_dataframe,
    obus_to_dataframe,
)

__all__ = [
    "audio_frames_to_dataframe",
    "events_to_dataframe",
    "klv_to_dataframe",
    "nals_to_dataframe",
    "obus_to_dataframe",
]
