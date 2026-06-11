"""with_() copy-update helper on the four frozen typed KLV sets (v0.2.0 #4)."""

import pytest

from tstrans.klv import (
    PrecisionTimeStampPack,
    SecurityLs,
    TimeStatus,
    UasDatalinkLs,
    VmtiLs,
)


def test_with_replaces_named_fields_only():
    ls = UasDatalinkLs(mission_id="M1", sensor_lat_deg=1.0, sensor_lon_deg=2.0)
    out = ls.with_(sensor_lat_deg=33.5)
    assert out.sensor_lat_deg == 33.5
    assert out.mission_id == "M1"
    assert out.sensor_lon_deg == 2.0


def test_with_leaves_original_untouched():
    ls = UasDatalinkLs(sensor_lat_deg=1.0)
    ls.with_(sensor_lat_deg=2.0)
    assert ls.sensor_lat_deg == 1.0


def test_with_unknown_field_raises_typeerror():
    with pytest.raises(TypeError):
        UasDatalinkLs().with_(not_a_field=1)


def test_with_reruns_construction_validation():
    # __post_init__ re-runs on the copy: the 16-byte UL rule is enforced.
    with pytest.raises(ValueError):
        UasDatalinkLs().with_(universal_label=b"short")


def test_with_on_security_ls():
    s = SecurityLs(version=12)
    out = s.with_(classifying_country="//US")
    assert out.classifying_country == "//US"
    assert out.version == 12


def test_with_on_vmti_ls():
    assert VmtiLs(version_number=6).with_(frame_width=640).frame_width == 640


def test_with_on_precision_timestamp_pack():
    p = PrecisionTimeStampPack(time_status=TimeStatus(0xFF), timestamp_us=1)
    out = p.with_(timestamp_us=2)
    assert out.timestamp_us == 2
    assert out.time_status == TimeStatus(0xFF)


def test_with_no_changes_returns_equal_copy_of_same_type():
    for obj in (
        UasDatalinkLs(mission_id="x"),
        SecurityLs(version=12),
        VmtiLs(version_number=6),
        PrecisionTimeStampPack(time_status=TimeStatus(0xFF), timestamp_us=0),
    ):
        copy = obj.with_()
        assert type(copy) is type(obj) and copy == obj
