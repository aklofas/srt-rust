"""VTargetPack dataclass-shape tests. Decode integration is exercised
via test_klv_st0903.py (where targets land inside VmtiLs.targets)."""

from dataclasses import fields

import pytest

from tstrans.klv import KlvFieldError, VTargetPack


def test_vtarget_pack_has_target_id():
    p = VTargetPack(target_id=42)
    assert p.target_id == 42


def test_vtarget_pack_defaults_are_none():
    p = VTargetPack(target_id=1)
    assert p.centroid_pixel is None
    assert p.bbox_top_left_pixel is None
    assert p.priority is None
    assert p.target_color is None
    assert p.target_location is None


def test_vtarget_pack_color_is_3_byte_tuple():
    p = VTargetPack(target_id=1, target_color=(255, 128, 0))
    assert p.target_color == (255, 128, 0)


def test_vtarget_pack_field_errors_default_empty_tuple():
    p = VTargetPack(target_id=1)
    assert p.field_errors == ()
    assert isinstance(p.field_errors, tuple)


def test_vtarget_pack_unknown_default_empty_tuple():
    p = VTargetPack(target_id=1)
    assert p.unknown == ()


def test_vtarget_pack_frozen():
    p = VTargetPack(target_id=1)
    with pytest.raises((AttributeError, TypeError)):
        p.target_id = 99  # type: ignore[misc]


def test_vtarget_pack_pattern_match():
    p = VTargetPack(target_id=42, priority=5)
    match p:
        case VTargetPack(target_id=t, priority=pr):
            assert t == 42
            assert pr == 5
        case _:
            pytest.fail("did not match")


def test_vtarget_pack_field_count():
    """Sanity: ~30 fields per ST 0903.6 §10.2 Table 10."""
    f = fields(VTargetPack)
    assert len(f) >= 29
