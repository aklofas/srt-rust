"""ST 0601 encode round-trip tests."""

import pytest

from tstrans.exceptions import KlvEncodeError, KlvEncodeErrorKind
from tstrans.klv import (
    ST_0601_UL,
    UasDatalinkLs,
    decode_uas_datalink,
    encode_uas_datalink,
    encode_uas_datalink_strict_compliance,
)


def _populated_record() -> UasDatalinkLs:
    """A record with one of every field-type family populated, so the
    inverse translator exercises Optional<String>, Optional<scalar>, and
    Optional<bytes> paths."""
    return UasDatalinkLs(
        universal_label=ST_0601_UL,
        declared_version=19,
        mission_id="MISSION-1",
        timestamp_us=1_700_000_000_000_000,
        platform_heading_deg=42.5,
        uas_ls_version=19,
        sensor_lat_deg=37.7749,
        sensor_lon_deg=-122.4194,
        sensor_alt_m=1500.0,
    )


def test_encode_round_trip_basic_lenient():
    rec = _populated_record()
    encoded = encode_uas_datalink(rec)
    assert isinstance(encoded, bytes)
    assert len(encoded) > 0
    # Re-decode the encoded bytes — populated fields survive round-trip
    rec2 = decode_uas_datalink(encoded)
    assert rec2.mission_id == "MISSION-1"
    assert rec2.timestamp_us == 1_700_000_000_000_000
    assert rec2.uas_ls_version == 19
    # f64 round-trip within encoder quantization (KLV is fixed-point)
    assert rec2.platform_heading_deg is not None
    assert abs(rec2.platform_heading_deg - 42.5) < 0.01


def test_encode_returns_bytes_not_list():
    rec = _populated_record()
    out = encode_uas_datalink(rec)
    assert isinstance(out, bytes)


def test_encode_empty_record_lenient_ok():
    # Default-constructed record has all-None optional fields. Lenient
    # encode just emits the version tag (declared_version → Tag 65).
    rec = UasDatalinkLs()
    out = encode_uas_datalink(rec)
    assert isinstance(out, bytes)


def test_encode_strict_compliance_missing_mandatory_raises():
    # Default record has no mandatory tags (no Tag 2 precision timestamp)
    rec = UasDatalinkLs()
    with pytest.raises(KlvEncodeError) as ei:
        encode_uas_datalink_strict_compliance(rec)
    assert ei.value.kind is KlvEncodeErrorKind.MISSING_MANDATORY_ITEM
