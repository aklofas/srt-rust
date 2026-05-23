"""ST 0903 encode round-trip tests."""

import pytest

from tstrans.klv import (
    VmtiLs,
    VTargetPack,
    decode_vmti,
    encode_vmti,
    encode_vmti_standalone,
    parse_klv_universal,
)


def test_encode_vmti_ls_body_round_trip():
    body = (
        bytes([2, 8]) + (1_700_000_000_000_000).to_bytes(8, "big")  # PTS Tag 2
        + bytes([4, 1, 6])                                          # version Tag 4 = 6
        + bytes([8, 2]) + (1920).to_bytes(2, "big")                # frame_width Tag 8
    )
    vmti = decode_vmti(body)
    out = encode_vmti(vmti)
    assert isinstance(out, bytes)
    vmti2 = decode_vmti(out)
    assert vmti2.precision_time_stamp == vmti.precision_time_stamp
    assert vmti2.version_number == vmti.version_number


def test_encode_vmti_standalone_has_ul_prefix():
    vmti = decode_vmti(b"")
    out = encode_vmti_standalone(vmti)
    assert len(out) >= 16
    assert out[0] == 0x06  # SMPTE designator


def test_encode_vmti_standalone_round_trips_via_universal_dispatcher():
    body = bytes([2, 8]) + (1_700_000_000_000_000).to_bytes(8, "big") + bytes([4, 1, 6])
    vmti = decode_vmti(body)
    standalone = encode_vmti_standalone(vmti)
    parsed = parse_klv_universal(standalone)
    assert isinstance(parsed, VmtiLs)
    assert parsed.precision_time_stamp == vmti.precision_time_stamp


def test_encode_vmti_with_targets_round_trip():
    pack_body = bytes([0x01, 0x65, 0x02, 0xDE, 0xAD])  # target_id=1, vmask=0xDEAD
    series = bytes([len(pack_body)]) + pack_body
    body = (
        bytes([2, 8]) + (1_700_000_000_000_000).to_bytes(8, "big")
        + bytes([4, 1, 6])
        + bytes([101, len(series)]) + series
    )
    vmti = decode_vmti(body)
    assert len(vmti.targets) == 1
    out = encode_vmti(vmti)
    vmti2 = decode_vmti(out)
    assert len(vmti2.targets) == 1
    assert vmti2.targets[0].target_id == 1
    assert vmti2.targets[0].vmask == b"\xde\xad"
