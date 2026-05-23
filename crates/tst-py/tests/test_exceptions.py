"""Exception hierarchy contract: every tstrans error is a TstError;
each domain error has a typed .kind attribute (a Python Enum)."""

import pytest

from tstrans.exceptions import (
    TstError,
    MuxError,
    DemuxError,
    KlvError,
    CodecError,
    MuxErrorKind,
    DemuxErrorKind,
    KlvErrorKind,
    CodecErrorKind,
)


def test_tst_error_is_exception():
    assert issubclass(TstError, Exception)


@pytest.mark.parametrize("cls", [MuxError, DemuxError, KlvError, CodecError])
def test_domain_errors_inherit_tst_error(cls):
    assert issubclass(cls, TstError)


def test_mux_error_carries_kind_and_message():
    err = MuxError(kind=MuxErrorKind.INVALID_CONFIG, message="missing pcr_pid")
    assert err.kind is MuxErrorKind.INVALID_CONFIG
    assert err.message == "missing pcr_pid"
    assert "missing pcr_pid" in str(err)


def test_codec_error_carries_codec_label():
    err = CodecError(
        kind=CodecErrorKind.UNSUPPORTED_PROFILE,
        message="profile 244 not supported",
        codec="h264",
    )
    assert err.codec == "h264"
    assert err.kind is CodecErrorKind.UNSUPPORTED_PROFILE


def test_can_catch_specific_then_general():
    try:
        raise MuxError(kind=MuxErrorKind.INVALID_CONFIG, message="x")
    except TstError as e:
        assert isinstance(e, MuxError)
