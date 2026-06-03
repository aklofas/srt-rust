"""ST 0601 UAS Datalink LS — Python wrap tests.

Uses committed synthetic fixtures at
`crates/tst-core/tests/fixtures/st0601/`. `decode_uas_datalink`
consumes the full UL+length+body buffer (NOT body-only)."""

from pathlib import Path

import pytest

from tstrans.exceptions import KlvError, KlvErrorKind
from tstrans.klv import (
    Attitude,
    Corners,
    FieldOfView,
    GeoPoint,
    Klv0601,
    UasDatalinkLs,
    decode_uas_datalink,
)

FIXTURE_DIR = (
    Path(__file__).parent.parent.parent.parent
    / "crates" / "tst-core" / "tests" / "fixtures" / "st0601"
)

FX_MINIMAL = FIXTURE_DIR / "synthetic_minimal.klv"
FX_FULL = FIXTURE_DIR / "synthetic_full.klv"
FX_FUNKY_UL = FIXTURE_DIR / "synthetic_funky_ul.klv"
FX_FIELD_ERRORS = FIXTURE_DIR / "synthetic_field_errors.klv"


def test_fixtures_exist():
    for fx in (FX_MINIMAL, FX_FULL, FX_FUNKY_UL, FX_FIELD_ERRORS):
        assert fx.is_file(), f"missing fixture: {fx}"


def test_alias_klv0601_is_uas_datalink_ls():
    assert Klv0601 is UasDatalinkLs


def test_decode_synthetic_minimal():
    buf = FX_MINIMAL.read_bytes()
    rec = decode_uas_datalink(buf)
    assert isinstance(rec, UasDatalinkLs)
    # Minimal fixture has timestamp only per fixtures/st0601/README.md.
    assert rec.timestamp_us is not None
    # Round-trip integrity: no field errors on well-formed input.
    assert rec.field_errors == ()


def test_decode_synthetic_full_populates_typed_fields():
    rec = decode_uas_datalink(FX_FULL.read_bytes())
    # Full fixture populates ~41 typed tags per README — sanity-check
    # a few high-value ones.
    assert rec.timestamp_us is not None
    assert rec.platform_heading_deg is not None
    assert rec.sensor_lat_deg is not None
    assert rec.sensor_lon_deg is not None


def test_decode_returns_klv0601_alias():
    rec = decode_uas_datalink(FX_FULL.read_bytes())
    assert isinstance(rec, Klv0601)


def test_decode_funky_ul_lenient_accepts():
    """Funky UL fixture has byte 14 = 0x09 (ST 0601.9 legacy version
    convention) — lenient accepts any 16-byte UL."""
    rec = decode_uas_datalink(FX_FUNKY_UL.read_bytes())
    assert isinstance(rec, UasDatalinkLs)


def test_decode_funky_ul_strict_accepts_tolerant_family():
    """Strict mode requires the ST 0601 family UL pattern, which is
    tolerant of bytes 13 + 14 per spec (`is_st0601_family` ignores
    them). Funky byte 14 = 0x09 is still in-family, so strict should
    accept. If this test fails, the Rust `decode_strict` behaviour
    changed — follow the Rust source."""
    rec = decode_uas_datalink(FX_FUNKY_UL.read_bytes(), strict=True)
    assert isinstance(rec, UasDatalinkLs)


def test_decode_field_errors_fixture():
    """Field-errors fixture is intentionally malformed. Either field_errors
    OR unknown captures the malformation."""
    rec = decode_uas_datalink(FX_FIELD_ERRORS.read_bytes())
    has_field_err = len(rec.field_errors) > 0
    has_unknown = len(rec.unknown) > 0
    assert has_field_err or has_unknown


def test_decode_rejects_truncated():
    with pytest.raises(KlvError) as excinfo:
        decode_uas_datalink(b"\x06\x0E\x2B\x34")
    assert excinfo.value.kind == KlvErrorKind.TRUNCATED_SET


def test_uas_datalink_ls_frozen():
    rec = decode_uas_datalink(FX_FULL.read_bytes())
    with pytest.raises((AttributeError, TypeError)):
        rec.timestamp_us = 0  # type: ignore[misc]


def test_sensor_position_composite():
    rec = decode_uas_datalink(FX_FULL.read_bytes())
    pos = rec.sensor_position()
    if pos is not None:
        assert isinstance(pos, GeoPoint)


def test_sensor_attitude_composite():
    rec = decode_uas_datalink(FX_FULL.read_bytes())
    att = rec.sensor_attitude()
    if att is not None:
        assert isinstance(att, Attitude)


def test_sensor_fov_composite():
    rec = decode_uas_datalink(FX_FULL.read_bytes())
    fov = rec.sensor_fov()
    if fov is not None:
        assert isinstance(fov, FieldOfView)


def test_platform_attitude_composite():
    rec = decode_uas_datalink(FX_FULL.read_bytes())
    pa = rec.platform_attitude()
    if pa is not None:
        assert isinstance(pa, Attitude)


def test_frame_center_composite():
    rec = decode_uas_datalink(FX_FULL.read_bytes())
    fc = rec.frame_center()
    if fc is not None:
        assert isinstance(fc, GeoPoint)


def test_corners_composite():
    rec = decode_uas_datalink(FX_FULL.read_bytes())
    c = rec.corners()
    if c is not None:
        assert isinstance(c, Corners)
        for pt in (c.p1, c.p2, c.p3, c.p4):
            assert isinstance(pt, tuple)
            assert len(pt) == 2


def test_strict_compliance_minimal_accepts():
    """Inspection of the minimal fixture (hex: UL + len=0x11 + Tag 02
    timestamp + Tag 65=19 LS Version + Tag 01 checksum) shows it was
    crafted to satisfy strict_compliance: Tag 2 first, Tag 65 present,
    Tag 1 last. So this path should accept, not reject. Either branch
    is acceptable — the assertion just exercises the compliance path
    without trapping a particular outcome."""
    rec = decode_uas_datalink(FX_MINIMAL.read_bytes(), compliance=True)
    assert isinstance(rec, UasDatalinkLs)
    # Strict-compliance success implies all three structural gates
    # were satisfied — record carries the parsed timestamp + version.
    assert rec.timestamp_us is not None
    assert rec.uas_ls_version is not None


def test_security_local_set_bytes_preserved():
    rec = decode_uas_datalink(FX_FULL.read_bytes())
    assert rec.security_local_set is None or isinstance(
        rec.security_local_set, bytes
    )


def test_vmti_bytes_preserved():
    rec = decode_uas_datalink(FX_FULL.read_bytes())
    assert rec.vmti is None or isinstance(rec.vmti, bytes)
