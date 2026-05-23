"""tstrans.pandas — pandas DataFrame adapters for tstrans types.

Requires the [pandas] extra:
    pip install 'tstrans[pandas]'

Importing this submodule does NOT trigger pandas/numpy import. Only the
adapter function calls trigger the gated import. Calling an adapter
without the extra raises ImportError with the install hint.
"""

# Submodules are imported lazily inside functions to avoid pulling
# pandas/numpy at module-import time. Users access adapters by their
# top-level names re-exported below.

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
