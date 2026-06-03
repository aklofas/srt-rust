"""ST 0605 §7 Precision Time Stamp Pack — Python wrap tests.

Generates the spec fixture inline: 16-byte UL + 1-byte BER length
(0x09) + 1-byte TimeStatus + 8-byte BE microsecond timestamp."""

import pytest

from tstrans.exceptions import KlvError, KlvErrorKind
from tstrans.klv import (
    Klv0605,
    PRECISION_TIMESTAMP_PACK_UL,
    PrecisionTimeStampPack,
    TimeStatus,
    decode_precision_timestamp,
)


def _build_pack(status_byte: int, timestamp_us: int) -> bytes:
    """Build a wire-format ST 0605 pack: 16-byte UL + BER 0x09 +
    1-byte status + 8-byte BE timestamp = 26 bytes total."""
    return (
        PRECISION_TIMESTAMP_PACK_UL
        + bytes([0x09, status_byte & 0xFF])
        + timestamp_us.to_bytes(8, "big")
    )


def test_alias_klv0605_is_precision_time_stamp_pack():
    assert Klv0605 is PrecisionTimeStampPack


def test_time_status_locked_normal():
    s = TimeStatus(0x1F)  # 0b0001_1111 — locked, normal increment
    assert s.is_locked
    assert not s.has_discontinuity
    assert not s.is_reverse_jump
    assert s.reserved_bits_valid


def test_time_status_lock_unknown_normal():
    s = TimeStatus(0x9F)  # 0b1001_1111 — lock unknown
    assert not s.is_locked
    assert not s.has_discontinuity
    assert s.reserved_bits_valid


def test_time_status_discontinuity_reverse():
    s = TimeStatus(0xFF)
    assert not s.is_locked
    assert s.has_discontinuity
    assert s.is_reverse_jump


def test_time_status_invalid_reserved_bits():
    s = TimeStatus(0x10)  # bits 3-0 are zero — should fail reserved check
    assert not s.reserved_bits_valid


def test_decode_locked_pack():
    buf = _build_pack(0x1F, 1_753_983_356_565_441)
    pack = decode_precision_timestamp(buf)
    assert pack.timestamp_us == 1_753_983_356_565_441
    assert pack.time_status.is_locked
    assert pack.time_status.reserved_bits_valid


def test_decode_returns_klv0605_alias():
    buf = _build_pack(0x1F, 1_700_000_000_000_000)
    pack = decode_precision_timestamp(buf)
    assert isinstance(pack, Klv0605)


def test_decode_rejects_wrong_ul():
    from tstrans.klv import ST_0601_UL

    buf = ST_0601_UL + bytes([0x09, 0x1F]) + (0).to_bytes(8, "big")
    with pytest.raises(KlvError) as excinfo:
        decode_precision_timestamp(buf)
    assert excinfo.value.kind is KlvErrorKind.BAD_UNIVERSAL_LABEL


def test_decode_rejects_truncated():
    with pytest.raises(KlvError) as excinfo:
        decode_precision_timestamp(b"\x00" * 8)
    assert excinfo.value.kind is KlvErrorKind.TRUNCATED_SET


def test_decode_rejects_wrong_body_length():
    buf = PRECISION_TIMESTAMP_PACK_UL + bytes([0x05]) + (0).to_bytes(5, "big")
    with pytest.raises(KlvError) as excinfo:
        decode_precision_timestamp(buf)
    assert excinfo.value.kind is KlvErrorKind.MALFORMED_BYTES


def test_pack_is_frozen_dataclass():
    buf = _build_pack(0x1F, 100)
    pack = decode_precision_timestamp(buf)
    with pytest.raises((AttributeError, TypeError)):
        pack.timestamp_us = 999  # type: ignore[misc]


def test_pack_equality():
    a = decode_precision_timestamp(_build_pack(0x1F, 100))
    b = decode_precision_timestamp(_build_pack(0x1F, 100))
    c = decode_precision_timestamp(_build_pack(0x1F, 101))
    assert a == b
    assert a != c
