"""Shared fixtures and helpers for the cross-binding scenario harness.

The scenarios directory lives at:
  crates/tst-integration/tests/fixtures/scenarios/

This conftest resolves that path relative to __file__ (absolute, CWD-
independent) and exposes two helpers:

- `scenarios_dir()` — returns the Path to the scenarios directory.
- `require_scenario(id)` — returns (manifest_entry, scenario_dir) for the
  given scenario id; calls pytest.skip with a clear message if the id is not
  in the manifest or the directory is missing.

TOML parsing: uses `tomllib` (stdlib since Python 3.11).  The project CI
uses Python 3.12, and the project minimum is Python 3.10.  Python 3.10 does
not have `tomllib`; users on 3.10 must `pip install tomli` separately.  The
import below falls back to `tomli` on 3.10/3.11-pre, and skips all scenario
tests cleanly if neither is available, so the test suite never hard-fails
on a missing optional dep.
"""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

import pytest

# ── TOML parser ──────────────────────────────────────────────────────────────
# tomllib is stdlib in 3.11+; on 3.10 fall back to the third-party tomli.
# If neither is available the fixture functions below raise a pytest.skip so
# the scenario tests are skipped without failing the suite.
try:
    if sys.version_info >= (3, 11):
        import tomllib as _toml
    else:
        import tomli as _toml  # type: ignore[import-not-found]
    _TOML_AVAILABLE = True
except ImportError:
    _TOML_AVAILABLE = False


# ── Path resolution ───────────────────────────────────────────────────────────

def scenarios_dir() -> Path:
    """Absolute path to the committed scenario fixtures directory.

    Computed from __file__ so it is CWD-independent:
      conftest.py lives at  crates/tst-py/tests/conftest.py
      scenarios live at     crates/tst-integration/tests/fixtures/scenarios/
    """
    here = Path(__file__).resolve().parent          # crates/tst-py/tests/
    return (
        here
        / ".."    # crates/tst-py/
        / ".."    # crates/
        / "tst-integration"
        / "tests"
        / "fixtures"
        / "scenarios"
    ).resolve()


# ── Manifest loading ──────────────────────────────────────────────────────────

def _load_manifest() -> list[dict[str, Any]]:
    """Parse scenarios.toml and return the list of scenario entries.

    Raises pytest.skip if tomllib/tomli is unavailable or the file is missing.
    """
    if not _TOML_AVAILABLE:
        pytest.skip(
            "TOML parser unavailable: Python 3.11+ tomllib is built-in; "
            "on Python 3.10 run `pip install tomli`."
        )
    manifest_path = scenarios_dir() / "scenarios.toml"
    if not manifest_path.is_file():
        pytest.skip(f"scenarios.toml not found at {manifest_path}")
    with open(manifest_path, "rb") as fh:
        data = _toml.load(fh)
    return data.get("scenario", [])


def require_scenario(scenario_id: str) -> tuple[dict[str, Any], Path]:
    """Return ``(manifest_entry, scenario_path)`` for *scenario_id*.

    Calls ``pytest.skip`` with a clear message if the scenario is not listed
    in ``scenarios.toml`` or its directory is missing from the fixture tree.
    The scenario directory is the parent directory of the scenario's ``input``
    artifact.
    """
    entries = _load_manifest()
    matched = next((e for e in entries if e["id"] == scenario_id), None)
    if matched is None:
        pytest.skip(f"scenario '{scenario_id}' not listed in scenarios.toml")
    sdir = scenarios_dir()
    # The input path is relative to the scenarios dir; derive the scenario dir
    # as the first component of that relative path.
    input_rel = Path(matched["input"])
    scenario_path = (sdir / input_rel.parts[0]).resolve()
    if not scenario_path.is_dir():
        pytest.skip(
            f"scenario directory missing: {scenario_path} "
            f"(scenario '{scenario_id}')"
        )
    return matched, scenario_path
