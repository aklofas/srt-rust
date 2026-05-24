"""Phase 6: verify friendly ImportError when [pandas] extra is missing.

This test simulates the missing-extra scenario by patching sys.modules,
then confirms every public adapter raises ImportError with the canonical
install hint.

Note: this test is NOT marked with @pytest.mark.pandas — it should run in
the DEFAULT venv (without the extra) AND in the pandas-extra venv (where
the simulation patches over the real numpy/pandas).
"""

import sys

import pytest


def _patch_missing(monkeypatch):
    """Patch sys.modules to make pandas + numpy imports raise ImportError."""
    monkeypatch.setitem(sys.modules, "pandas", None)
    monkeypatch.setitem(sys.modules, "numpy", None)
    # Also clear any cached references in tstrans.pandas._imports
    import tstrans.pandas._imports as _imp
    _imp._pd = None
    _imp._np = None


def test_klv_to_dataframe_raises_friendly_import_error(monkeypatch):
    _patch_missing(monkeypatch)
    from tstrans.pandas import klv_to_dataframe
    with pytest.raises(ImportError, match=r"tstrans\[pandas\]"):
        klv_to_dataframe([])


def test_events_to_dataframe_raises_friendly_import_error(monkeypatch):
    _patch_missing(monkeypatch)
    from tstrans.pandas import events_to_dataframe
    with pytest.raises(ImportError, match=r"tstrans\[pandas\]"):
        events_to_dataframe([])


def test_nals_to_dataframe_raises_friendly_import_error(monkeypatch):
    _patch_missing(monkeypatch)
    from tstrans.pandas import nals_to_dataframe
    with pytest.raises(ImportError, match=r"tstrans\[pandas\]"):
        nals_to_dataframe([])


def test_obus_to_dataframe_raises_friendly_import_error(monkeypatch):
    _patch_missing(monkeypatch)
    from tstrans.pandas import obus_to_dataframe
    with pytest.raises(ImportError, match=r"tstrans\[pandas\]"):
        obus_to_dataframe([])


def test_audio_frames_to_dataframe_raises_friendly_import_error(monkeypatch):
    _patch_missing(monkeypatch)
    from tstrans.pandas import audio_frames_to_dataframe
    with pytest.raises(ImportError, match=r"tstrans\[pandas\]"):
        audio_frames_to_dataframe([])


def test_payload_np_raises_friendly_import_error(monkeypatch):
    _patch_missing(monkeypatch)
    from tstrans.codec import NalUnit
    nal = NalUnit.h264(nal_type=5, ref_idc=3, payload=b"")
    with pytest.raises(ImportError, match=r"tstrans\[pandas\]"):
        _ = nal.payload_np


def test_tstrans_pandas_module_imports_without_extra():
    """Critical: importing tstrans.pandas must NOT trigger the extra check."""
    # If this test is running, the module has already been imported by
    # the other tests via `from tstrans.pandas import ...`. Just confirm
    # it's importable cleanly:
    import importlib
    import tstrans.pandas
    importlib.reload(tstrans.pandas)
