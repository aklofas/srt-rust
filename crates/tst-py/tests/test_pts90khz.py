"""Pts90khz wrapper — value semantics over a 90 kHz tick count."""

import pytest

from tstrans.mpegts import Pts90khz


def test_from_raw_round_trip():
    pts = Pts90khz.from_raw(90_000)
    assert pts.raw == 90_000


def test_from_ms_round_trip():
    pts = Pts90khz.from_ms(1000)
    assert pts.raw == 90_000  # 1000 ms × 90 ticks/ms
    assert pts.ms == 1000


def test_from_seconds_round_trip():
    pts = Pts90khz.from_seconds(0.5)
    assert pts.raw == 45_000
    assert pts.seconds == pytest.approx(0.5)


def test_ms_is_int_truncating():
    # 90_001 ticks = 1.000011... s = 1000.0111... ms → int truncates to 1000
    pts = Pts90khz.from_raw(90_001)
    assert pts.ms == 1000


def test_seconds_is_float():
    pts = Pts90khz.from_raw(45_000)
    assert isinstance(pts.seconds, float)
    assert pts.seconds == pytest.approx(0.5)


def test_equality_by_raw_value():
    assert Pts90khz.from_raw(123) == Pts90khz.from_raw(123)
    assert Pts90khz.from_raw(123) != Pts90khz.from_raw(124)


def test_hashable():
    s = {Pts90khz.from_raw(1), Pts90khz.from_raw(1), Pts90khz.from_raw(2)}
    assert len(s) == 2


def test_repr_includes_raw_and_ms():
    pts = Pts90khz.from_raw(90_000)
    r = repr(pts)
    assert "90000" in r
    assert "1000" in r or "1.0" in r  # ms or seconds


def test_negative_raw_allowed():
    # Rust's i64 PTS allows negative for diff arithmetic
    pts = Pts90khz.from_raw(-100)
    assert pts.raw == -100
