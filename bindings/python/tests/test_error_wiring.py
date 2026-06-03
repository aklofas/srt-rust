"""End-to-end test that PyO3 can raise the Python exception classes
defined in tstrans.exceptions. Uses a Rust-side test helper exposed
only for this purpose."""

import pytest

from tstrans import _native
from tstrans.exceptions import MuxError, TstError, MuxErrorKind


def test_native_can_raise_mux_error():
    with pytest.raises(MuxError) as exc_info:
        _native._raise_mux_error_for_test("synthetic failure")
    assert "synthetic failure" in str(exc_info.value)


def test_native_mux_error_is_catchable_as_tst_error():
    with pytest.raises(TstError):
        _native._raise_mux_error_for_test("x")


def test_native_mux_error_kind_attribute_is_set():
    with pytest.raises(MuxError) as exc_info:
        _native._raise_mux_error_for_test("x")
    assert exc_info.value.kind is MuxErrorKind.INTERNAL
