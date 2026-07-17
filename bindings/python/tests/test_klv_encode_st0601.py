"""ST 0601 encode round-trip tests."""

import pytest

from tstrans.exceptions import KlvEncodeError, KlvEncodeErrorKind
from tstrans.klv import (
    ST_0601_UL,
    IcingDetected,
    OperationalMode,
    OutOfRangePolicy,
    SensorFovName,
    UasDatalinkLs,
    decode_uas_datalink,
    encode_uas_datalink,
    encode_uas_datalink_strict_compliance,
    st0601_sentinel_meaning,
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


def test_sentinel_tags_out_of_u32_range_raises_overflow():
    """A sentinel_tags entry above u32::MAX must raise OverflowError, not truncate.

    Regression test for a silent `as u32` truncation: 2**33 would have wrapped
    to 0 and encoded the wrong (or no) sentinel tag.
    """
    rec = UasDatalinkLs(sentinel_tags=(2**33,))
    with pytest.raises(OverflowError):
        encode_uas_datalink(rec)


# ---------------------------------------------------------------------------
# OutOfRangePolicy: encode indicator for eligible tags
# ---------------------------------------------------------------------------


def test_encode_out_of_range_indicator_policy():
    """INDICATOR policy emits the INT_MIN sentinel for Tag 6 (Platform Pitch)
    when the value (25.0°) is outside the narrow [-20, 20]° range.

    With the default ERROR policy, encoding raises. With INDICATOR, encode
    succeeds, and decoding the result shows platform_pitch_deg=None and
    tag 6 in sentinel_tags (the spec Out-of-Range signal).
    """
    # platform_pitch_deg Tag 6 has range [-20, 20]; 25.0 is out of range.
    rec = UasDatalinkLs(platform_pitch_deg=25.0)
    # Default policy (ERROR) must still raise.
    with pytest.raises(KlvEncodeError):
        encode_uas_datalink(rec)
    # INDICATOR policy succeeds and emits the INT_MIN sentinel.
    raw = encode_uas_datalink(rec, out_of_range_policy=OutOfRangePolicy.INDICATOR)
    assert isinstance(raw, bytes)
    back = decode_uas_datalink(raw)
    assert back.platform_pitch_deg is None, "sentinel field must be None after decode"
    assert 6 in back.sentinel_tags, "tag 6 must appear in sentinel_tags"


def test_encode_indicator_policy_ineligible_tag_still_raises():
    """INDICATOR policy only applies to tags whose INT_MIN sentinel means
    Out of Range. Tag 13 (sensor_lat_deg, range [-90, 90]) has a Reserved
    sentinel meaning, so a value outside range must still raise KlvEncodeError.

    95.0° is beyond [-90, 90] and Tag 13 is not in the eligible set.
    """
    rec = UasDatalinkLs(sensor_lat_deg=95.0)
    with pytest.raises(KlvEncodeError):
        encode_uas_datalink(rec, out_of_range_policy=OutOfRangePolicy.INDICATOR)


# ---------------------------------------------------------------------------
# WP-A: Table A1 ranged f64 fields — round-trip within quantization tolerance
# ---------------------------------------------------------------------------

# (field, example value, tolerance). Tolerance is ~1.5x the fixed-point
# quantization step for the field's range/byte-width (Appendix Table A1) —
# tight enough to catch a wrong-field/wrong-scale marshalling bug, loose
# enough not to flake on the encoder's actual rounding. Byte-level wire
# correctness is already covered by the tst-core unit tests (Tasks A1-A5);
# this is a Python <-> Rust marshalling check.
_F64_FIELDS = [
    ("wind_direction_deg", 235.924010, 0.01),
    ("wind_speed", 69.8039216, 0.6),
    ("static_pressure_mbar", 3725.18502, 0.12),
    ("density_altitude_m", 14818.6770, 0.46),
    ("target_location_lat_deg", -79.163850051892850, 1e-6),
    ("target_location_lon_deg", 166.40081296041646, 1e-6),
    ("target_location_elev_m", 18389.0471, 0.46),
    ("target_track_gate_width_px", 6.0, 3.0),
    ("target_track_gate_height_px", 30.0, 3.0),
    ("target_error_ce90_m", 425.215152, 0.1),
    ("target_error_le90_m", 608.9231, 0.1),
    ("differential_pressure_mbar", 1191.95850, 0.12),
    ("platform_vertical_speed", -61.8878750, 0.01),
    ("platform_sideslip_deg", -5.08255257, 0.001),
    ("airfield_barometric_pressure_mbar", 2088.96010, 0.12),
    ("airfield_elevation_m", 8306.80552, 0.46),
    ("relative_humidity_pct", 50.5882353, 0.6),
    ("platform_ground_speed", 140.0, 1.5),
    ("ground_range_m", 3506979.0316063400, 0.01),
    ("platform_fuel_remaining_kg", 6420.53864, 0.23),
    ("platform_magnetic_heading_deg", 311.868162, 0.01),
    ("alternate_platform_lat_deg", -86.041207348947040, 1e-6),
    ("alternate_platform_lon_deg", 0.15552755452484243, 1e-6),
    ("alternate_platform_alt_m", 9.44533455, 0.46),
    ("alternate_platform_heading_deg", 32.6024262, 0.01),
    ("alternate_platform_ellipsoid_height_m", 9.44533455, 0.46),
    ("sensor_north_velocity", 25.4977569, 0.02),
    ("sensor_east_velocity", 12.1, 0.02),
    ("platform_angle_of_attack_full_deg", -8.6701769841230370, 1e-6),
    ("platform_sideslip_full_deg", -47.683, 1e-6),
]


@pytest.mark.parametrize("field,value,tol", _F64_FIELDS)
def test_wpa_f64_field_round_trip(field, value, tol):
    """Table A1: every ranged f64 field survives encode -> decode within
    its fixed-point quantization step."""
    rec = UasDatalinkLs(**{field: value})
    back = decode_uas_datalink(encode_uas_datalink(rec))
    got = getattr(back, field)
    assert got is not None, f"{field} must round-trip non-None"
    assert abs(got - value) < tol, f"{field}: got {got}, want {value} (tol {tol})"
    assert back.field_errors == ()


# ---------------------------------------------------------------------------
# WP-A: Table A2 raw/simple scalar + string fields — exact round-trip
# ---------------------------------------------------------------------------

_RAW_FIELDS = [
    ("outside_air_temp_c", 84),
    ("weapon_load", 45016),
    ("weapon_fired", 186),
    ("laser_prf_code", 1743),
    ("alternate_platform_name", "APACHE"),
    ("event_start_time_us", 798039894000000),
    ("stream_designator", "BLUE"),
    ("operational_base", "BASE01"),
    ("broadcast_source", "HOME"),
    ("target_id", "A123"),
    ("communications_method", "Frequency Modulation"),
]


@pytest.mark.parametrize("field,value", _RAW_FIELDS)
def test_wpa_raw_field_round_trip_exact(field, value):
    """Table A2: raw int/string fields are not fixed-point quantized —
    they round-trip byte-exact."""
    rec = UasDatalinkLs(**{field: value})
    back = decode_uas_datalink(encode_uas_datalink(rec))
    assert getattr(back, field) == value
    assert back.field_errors == ()


# ---------------------------------------------------------------------------
# WP-A: Table A4 named nested-set raw byte fields — exact round-trip
# ---------------------------------------------------------------------------

_BYTES_FIELDS = [
    "rvt",
    "sar_mi_local_set",
    "range_image_local_set",
    "geo_registration_local_set",
    "composite_imaging_local_set",
    "segment_local_set",
    "amend_local_set",
]


@pytest.mark.parametrize("field", _BYTES_FIELDS)
def test_wpa_bytes_field_round_trip_exact(field):
    value = bytes([0xDE, 0xAD, 0xBE, 0xEF, len(field) % 256])
    rec = UasDatalinkLs(**{field: value})
    back = decode_uas_datalink(encode_uas_datalink(rec))
    assert getattr(back, field) == value
    assert isinstance(getattr(back, field), bytes)


# ---------------------------------------------------------------------------
# WP-A: Table A3 coded enums — known codepoint + wire-unknown int round-trip
# ---------------------------------------------------------------------------


def test_icing_detected_known_round_trip():
    rec = UasDatalinkLs(icing_detected=IcingDetected.ICING_DETECTED)
    back = decode_uas_datalink(encode_uas_datalink(rec))
    assert back.icing_detected == IcingDetected.ICING_DETECTED


def test_icing_detected_unknown_int_round_trip():
    """A wire-unknown codepoint (not 0/1/2) surfaces as a raw int, not an
    enum instance — mirrors the SecurityClassification.Unknown(b) asymmetry."""
    rec = UasDatalinkLs(icing_detected=200)
    back = decode_uas_datalink(encode_uas_datalink(rec))
    assert back.icing_detected == 200
    assert isinstance(back.icing_detected, int)
    assert not isinstance(back.icing_detected, IcingDetected)


def test_sensor_fov_name_known_round_trip_incl_continuous_zoom():
    # ContinuousZoom (8) is the spec-discrepancy Table-4 codepoint beyond
    # the item's own [0, 7] definition-table cap.
    rec = UasDatalinkLs(sensor_fov_name=SensorFovName.CONTINUOUS_ZOOM)
    back = decode_uas_datalink(encode_uas_datalink(rec))
    assert back.sensor_fov_name == SensorFovName.CONTINUOUS_ZOOM


def test_sensor_fov_name_unknown_int_round_trip():
    rec = UasDatalinkLs(sensor_fov_name=250)
    back = decode_uas_datalink(encode_uas_datalink(rec))
    assert back.sensor_fov_name == 250
    assert isinstance(back.sensor_fov_name, int)


def test_operational_mode_other_mode_round_trip():
    """Spec code 0 ('Other' in Table 5) must round-trip as the named
    OTHER_MODE enum member, NOT as a raw int — it's a known codepoint,
    distinct from the wire-unknown Other(u8) catch-all."""
    rec = UasDatalinkLs(operational_mode=OperationalMode.OTHER_MODE)
    back = decode_uas_datalink(encode_uas_datalink(rec))
    assert back.operational_mode == OperationalMode.OTHER_MODE
    assert isinstance(back.operational_mode, OperationalMode)


def test_operational_mode_known_round_trip():
    rec = UasDatalinkLs(operational_mode=OperationalMode.TRAINING)
    back = decode_uas_datalink(encode_uas_datalink(rec))
    assert back.operational_mode == OperationalMode.TRAINING


def test_operational_mode_unknown_int_round_trip():
    rec = UasDatalinkLs(operational_mode=99)
    back = decode_uas_datalink(encode_uas_datalink(rec))
    assert back.operational_mode == 99
    assert isinstance(back.operational_mode, int)


# ---------------------------------------------------------------------------
# WP-A: representative combined round-trip
# ---------------------------------------------------------------------------


def test_wpa_new_fields_round_trip():
    ls = UasDatalinkLs(
        wind_direction_deg=235.924010,
        target_location_lat_deg=-79.16385005189285,
        outside_air_temp_c=84,
        weapon_load=45016,
        alternate_platform_name="APACHE",
        icing_detected=IcingDetected.ICING_DETECTED,
        rvt=b"\xde\xad",
    )
    back = decode_uas_datalink(encode_uas_datalink(ls))
    assert abs(back.wind_direction_deg - 235.924010) < 0.006
    assert back.weapon_load == 45016
    assert back.icing_detected == IcingDetected.ICING_DETECTED
    assert back.rvt == b"\xde\xad"


# ---------------------------------------------------------------------------
# WP-A: ST 0601.19 INT_MIN sentinel meaning lookup
# ---------------------------------------------------------------------------


def test_sentinel_meaning_lookup():
    assert st0601_sentinel_meaning(6) == "out_of_range"
    assert st0601_sentinel_meaning(13) == "reserved"
    assert st0601_sentinel_meaning(26) == "not_available"
    assert st0601_sentinel_meaning(5) is None


def test_sentinel_meaning_lookup_covers_every_out_of_range_tag():
    """Cross-check against the full INDICATOR-eligible tag set documented
    on the Rust OutOfRangePolicy::Indicator variant."""
    for tag in (6, 7, 50, 51, 52, 79, 80, 90, 91, 92, 93):
        assert st0601_sentinel_meaning(tag) == "out_of_range", tag


def test_sentinel_meaning_lookup_reserved_and_not_available_tags():
    for tag in (13, 14, 19, 67, 68):
        assert st0601_sentinel_meaning(tag) == "reserved", tag
    for tag in (23, 24, 40, 41):
        assert st0601_sentinel_meaning(tag) == "not_available", tag
    # Corner offset (26-33) and full-corner (82-89) ranges are all
    # "not_available" too.
    for tag in list(range(26, 34)) + list(range(82, 90)):
        assert st0601_sentinel_meaning(tag) == "not_available", tag
