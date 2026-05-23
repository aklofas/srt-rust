"""Well-known Universal Label constants + is_st0601_family predicate.

Bytes match the canonical values from MISB ST 0601.19 §6.2 / ST 0102.12 §6.7
/ ST 0807.27 row 1061 / ST 0903.6 §10.1, as encoded in the Rust
`tst_core::klv::universal_label::UniversalLabel` module."""

import pytest

from tstrans.klv import (
    ST_0601_UL,
    SECURITY_LS_UL,
    PRECISION_TIMESTAMP_PACK_UL,
    VMTI_LS_UL,
    is_st0601_family,
)


def test_each_ul_is_16_bytes():
    for ul in (ST_0601_UL, SECURITY_LS_UL, PRECISION_TIMESTAMP_PACK_UL, VMTI_LS_UL):
        assert isinstance(ul, bytes)
        assert len(ul) == 16


def test_st0601_ul_canonical_bytes():
    expected = bytes.fromhex("060e2b34020b01010e01030101000000")
    assert ST_0601_UL == expected


def test_security_ls_ul_canonical_bytes():
    expected = bytes.fromhex("060e2b34020301010e01030302000000")
    assert SECURITY_LS_UL == expected


def test_precision_timestamp_pack_ul_canonical_bytes():
    expected = bytes.fromhex("060e2b34020501010e01010311000000")
    assert PRECISION_TIMESTAMP_PACK_UL == expected


def test_vmti_ls_ul_canonical_bytes():
    expected = bytes.fromhex("060e2b34020b01010e01030306000000")
    assert VMTI_LS_UL == expected


def test_is_st0601_family_accepts_canonical():
    assert is_st0601_family(ST_0601_UL)


def test_is_st0601_family_accepts_legacy_byte13_byte14():
    # ST 0601.14 captures may set byte 14 = 0x0E (document version
    # convention from older MISB encoders). Rust's family check is
    # tolerant of bytes 13 + 14 to allow round-trip interop.
    mutated = bytearray(ST_0601_UL)
    mutated[14] = 0x0E
    assert is_st0601_family(bytes(mutated))
    mutated[13] = 0x09
    assert is_st0601_family(bytes(mutated))


def test_is_st0601_family_rejects_byte15_nonzero():
    mutated = bytearray(ST_0601_UL)
    mutated[15] = 0x01
    assert not is_st0601_family(bytes(mutated))


def test_is_st0601_family_rejects_wrong_prefix():
    # Security LS UL belongs to the ST 0102 family, not ST 0601.
    assert not is_st0601_family(SECURITY_LS_UL)


def test_is_st0601_family_rejects_short_buffer():
    assert not is_st0601_family(b"")
    assert not is_st0601_family(b"\x06\x0E\x2B\x34")


def test_is_st0601_family_rejects_oid_mismatch():
    mutated = bytearray(ST_0601_UL)
    mutated[0] = 0x07
    assert not is_st0601_family(bytes(mutated))
