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


# ---------------------------------------------------------------------------
# Encode-path validation: invalid `universal_label` shapes must raise, not
# be silently dropped (audit #6).
# ---------------------------------------------------------------------------


def test_encode_universal_label_3_bytes_raises():
    """A 3-byte universal_label must raise ValueError. Audit-2 #4 moved
    validation to construction time (__post_init__) so the error now fires
    at UasDatalinkLs(...) rather than at encode_uas_datalink(...)."""
    with pytest.raises(ValueError, match="universal_label"):
        UasDatalinkLs(universal_label=b"\x06\x0e\x2b")


def test_encode_universal_label_17_bytes_raises():
    """A 17-byte universal_label must raise ValueError. Audit-2 #4 moved
    validation to construction time (__post_init__) so the error now fires
    at UasDatalinkLs(...) rather than at encode_uas_datalink(...)."""
    with pytest.raises(ValueError, match="universal_label"):
        UasDatalinkLs(universal_label=b"\x00" * 17)


def test_encode_universal_label_16_bytes_ok():
    """A correctly-sized 16-byte universal_label still passes the
    encoder."""
    rec = UasDatalinkLs(universal_label=b"\x00" * 16)
    out = encode_uas_datalink(rec)
    assert isinstance(out, bytes)


# ---------------------------------------------------------------------------
# DA-KLV-4: sentinel round-trip tests
# ---------------------------------------------------------------------------


def test_sentinel_encode_then_decode_produces_sentinel_tags_not_field_error():
    """Tag 6 (Platform Pitch) sentinel → sentinel_tags=(6,), field=None, no error.

    Construct a record with sentinel_tags=(6,) and no typed pitch field.  The
    encoder emits INT_MIN (0x8000) for tag 6 per ST 0601.19 §8.6.  Decoding the
    result must give platform_pitch_deg=None, sentinel_tags containing 6, and an
    empty field_errors — the INT_MIN value is a spec-defined signal, not an error.
    """
    rec = UasDatalinkLs(sentinel_tags=(6,))
    wire = encode_uas_datalink(rec)
    rec2 = decode_uas_datalink(wire)
    assert rec2.platform_pitch_deg is None, "INT_MIN sentinel must leave typed field None"
    assert 6 in rec2.sentinel_tags, "tag 6 must appear in sentinel_tags"
    assert rec2.field_errors == (), "sentinel must not produce a field_error"


def test_sentinel_round_trips_through_encode():
    """A decoded sentinel record re-encodes and re-decodes with the sentinel preserved."""
    rec = UasDatalinkLs(sentinel_tags=(6,))
    wire1 = encode_uas_datalink(rec)
    rec2 = decode_uas_datalink(wire1)
    wire2 = encode_uas_datalink(rec2)
    rec3 = decode_uas_datalink(wire2)
    assert rec3.platform_pitch_deg is None, "sentinel field must remain None after re-encode"
    assert 6 in rec3.sentinel_tags, "tag 6 must still be a sentinel after re-encode"
    assert rec3.field_errors == (), "no field_errors after re-encode"


def test_sentinel_value_wins_over_sentinel_tags():
    """If sentinel_tags lists a tag whose typed field is also set, the value wins.

    Build a record with platform_roll_deg=25.0 AND sentinel_tags=(7,) — tag 7
    is Platform Roll.  The encoder must emit the real value, not INT_MIN (0x8000).
    """
    rec = UasDatalinkLs(
        platform_roll_deg=25.0,
        sentinel_tags=(7,),
    )
    encoded = encode_uas_datalink(rec)
    rec2 = decode_uas_datalink(encoded)
    assert rec2.platform_roll_deg is not None, "value must survive (not replaced by sentinel)"
    assert abs(rec2.platform_roll_deg - 25.0) < 0.5, "value must be close to 25.0"
    assert 7 not in rec2.sentinel_tags, "tag 7 must NOT be a sentinel after value-wins encoding"
