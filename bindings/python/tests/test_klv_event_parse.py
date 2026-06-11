"""DemuxEvent.Klv.parse() decode-on-event sugar + parse_klv_universal strict knob."""

import pytest

from tstrans.exceptions import KlvError
from tstrans.klv import (
    SECURITY_LS_UL,
    SecurityLs,
    UasDatalinkLs,
    VmtiLs,
    encode_security,
    encode_uas_datalink,
    parse_klv_universal,
)
from tstrans.mpegts import (
    DemuxEvent,
    MetadataKindTag,
    Pts90khz,
    StreamId,
    StreamKindTag,
)


def _klv_event(payload: bytes) -> "DemuxEvent.Klv":
    return DemuxEvent.Klv(
        stream=StreamId(
            pid=0x102,
            kind=StreamKindTag.KLV_SYNC,
            codec=None,
            program_number=1,
        ),
        pts=Pts90khz.from_raw(900_000),
        kind=MetadataKindTag.KLV_SYNC_AU_CELL,
        payload=payload,
    )


def _minimal_security_record() -> bytes:
    # Missing required tags 1/2/3/12/13 -> lenient-OK, strict-raises.
    body = encode_security(SecurityLs(version=12))
    assert len(body) < 0x80
    return SECURITY_LS_UL + bytes([len(body)]) + body


def test_parse_st0601_returns_uas_datalink():
    raw = encode_uas_datalink(UasDatalinkLs(mission_id="M1"))
    rec = _klv_event(raw).parse()
    assert isinstance(rec, UasDatalinkLs)
    assert rec.mission_id == "M1"


def test_parse_st0102_returns_security_ls():
    from _builders.synthetic_klv_universal import synthetic_security_ls

    assert isinstance(_klv_event(synthetic_security_ls()).parse(), SecurityLs)


def test_parse_st0903_returns_vmti_ls():
    from _builders.synthetic_klv_universal import synthetic_vmti_ls

    assert isinstance(_klv_event(synthetic_vmti_ls()).parse(), VmtiLs)


def test_parse_unknown_ul_returns_none():
    unknown = bytes(range(16)) + bytes([0x01, 0x00])
    assert _klv_event(unknown).parse() is None


def test_parse_malformed_raises_klv_error():
    truncated = encode_uas_datalink(UasDatalinkLs(mission_id="M1"))[:-4]
    with pytest.raises(KlvError):
        _klv_event(truncated).parse()


def test_parse_strict_threads_to_decoder():
    ev = _klv_event(_minimal_security_record())
    assert isinstance(ev.parse(), SecurityLs)  # lenient tolerates
    with pytest.raises(KlvError):
        ev.parse(strict=True)


def test_parse_strict_accepts_valid_record():
    from _builders.synthetic_klv_universal import synthetic_security_ls

    rec = _klv_event(synthetic_security_ls()).parse(strict=True)
    assert isinstance(rec, SecurityLs)


def test_parse_klv_universal_strict_kwarg():
    raw = _minimal_security_record()
    assert isinstance(parse_klv_universal(raw), SecurityLs)
    with pytest.raises(KlvError):
        parse_klv_universal(raw, strict=True)
