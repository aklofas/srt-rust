"""Hard-fail guard: the cross-binding scenario contract suite must collect the
pilot scenarios. A regression that makes parametrization empty (broken manifest
path, TOML parser missing, parse error) must fail here, not silently vanish."""
from __future__ import annotations

import sys

import pytest

from conftest import _TOML_AVAILABLE, scenarios_dir

pytestmark = pytest.mark.skipif(
    not _TOML_AVAILABLE,
    reason="TOML parser unavailable; install dev extra (tomli on 3.10)",
)

PILOT_IDS = {"h264-st0601-mp", "video-roundtrip", "strict-rejection"}


def _load_ids() -> set[str]:
    if sys.version_info >= (3, 11):
        import tomllib as toml
    else:
        import tomli as toml  # type: ignore[import-not-found]
    path = scenarios_dir() / "scenarios.toml"
    assert path.is_file(), f"scenarios.toml not found at {path}"
    with open(path, "rb") as fh:
        data = toml.load(fh)
    return {e["id"] for e in data.get("scenario", [])}


def test_manifest_loads_pilot_scenarios() -> None:
    ids = _load_ids()
    assert len(ids) >= 3, f"expected >=3 scenarios, got {sorted(ids)}"
    assert PILOT_IDS <= ids, f"missing pilot scenarios: {sorted(PILOT_IDS - ids)}"
