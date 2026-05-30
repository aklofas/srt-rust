"""Python adapter for the cross-binding scenario harness (WS-5).

For each scenario in ``crates/tst-integration/tests/fixtures/scenarios/
scenarios.toml`` this test:

1. Feeds the committed input artifact through ``tstrans``.
2. Normalises the result into the binding-neutral golden envelope dict.
3. Asserts ``observed == committed_golden`` (full equality, order-sensitive).

Normalisation rules mirror the Rust adapter at
``crates/tst-integration/tests/rust_scenarios.rs`` exactly:

Video
  ``payload_sha256`` = sha256 of the concatenated RBSP bytes from every
  ``NalUnit.payload`` (H.264/H.265/H.266) or ``Obu.payload`` (AV1) in
  ``DemuxEvent.Video.payload``.  This is the same operation as the Rust
  normaliser's ``video_payload_bytes()``.

stream_type
  The raw PMT stream_type byte from the first ``DemuxEvent.ProgramMap``
  that lists the PID, formatted as lowercase hex ``"0x1b"``.  Built into
  a ``pid → stream_type_str`` map before emitting media events — same
  approach as the Rust normaliser.

KLV set identity
  Detected from the first 13 bytes of ``DemuxEvent.Klv.payload`` using the
  MISB ST 0601 UAS Datalink LS UL prefix.  Returns ``"st0601"`` or
  ``"unknown"``.

Error mapping
  Any ``DemuxError`` raised by ``Demuxer.feed`` or ``Demuxer.flush`` maps
  to the umbrella public code ``"STRICT_REJECTION"`` regardless of the
  specific Python ``DemuxErrorKind``.  This is the binding-neutral umbrella
  code used by all adapters (see ``demux_error_code()`` in
  ``crates/tst-integration/src/scenarios/mod.rs``).

Roundtrip
  The Python muxer reproduces the exact same TS bytes as the Rust generator
  (verified byte-identical against the committed ``output.ts`` artifact AND
  sha256-equal to ``extensions.output_sha256``).  This proves the Python
  binding's muxer is deterministic and produces the same artefact.

Unknown golden event tags
  If the committed golden contains an ``event`` tag that this normaliser
  does not recognise, the test fails loudly — never silently skips.
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path
from typing import Any

import pytest

from tstrans.exceptions import DemuxError
from tstrans.mpegts import (
    DemuxEvent,
    Demuxer,
    DemuxerConfig,
    Muxer,
    MuxerConfigBuilder,
    MuxerProgramConfigBuilder,
    Pts90khz,
    StrictMode,
    VideoCodec,
)

from conftest import _TOML_AVAILABLE, require_scenario, scenarios_dir

# ── Helpers ───────────────────────────────────────────────────────────────────

_ST0601_UL_PREFIX: bytes = bytes([
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01,
    0x0E, 0x01, 0x03, 0x01, 0x01,
])


def _sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _klv_set_from_ul(payload: bytes) -> str:
    """Detect the MISB KLV set from the first 13 bytes of the raw payload.

    Returns ``"st0601"`` for the ST 0601 UAS Datalink LS UL, else
    ``"unknown"``.  Mirrors ``klv_set_from_ul()`` in the Rust normaliser.
    """
    if (
        len(payload) >= len(_ST0601_UL_PREFIX)
        and payload[: len(_ST0601_UL_PREFIX)] == _ST0601_UL_PREFIX
    ):
        return "st0601"
    return "unknown"


def _video_payload_bytes(ev: DemuxEvent.Video) -> bytes:  # type: ignore[name-defined]
    """Concatenate raw payload bytes from all NAL units / OBUs.

    H.264/H.265/H.266: concatenate ``NalUnit.payload`` (RBSP, Annex-B
    start codes already stripped by the demuxer — same bytes the Rust
    normaliser reads from ``NalUnit { payload, .. }``).
    AV1: concatenate ``Obu.payload``.

    Mirrors ``video_payload_bytes()`` in the Rust normaliser.
    """
    out = bytearray()
    for item in ev.payload:
        out.extend(bytes(item.payload))
    return bytes(out)


# ── Normaliser ────────────────────────────────────────────────────────────────

def _demux_to_core_events(ts_bytes: bytes) -> list[dict[str, Any]]:
    """Feed *ts_bytes* through the tstrans Demuxer and return normalised
    ``CoreEvent``-shaped dicts.

    Rules (matching the Rust normaliser):
    - ``ProgramMap`` → build the ``pid → stream_type_hex`` map; skip from output.
    - ``Video``      → ``{"event":"video", ...}``
    - ``Audio``      → ``{"event":"audio", ...}``
    - ``Klv``        → ``{"event":"klv", ...}``
    - ``UnknownSample`` → ``{"event":"unknown", "pid": ...}``
    - ``Discontinuity`` / ``NonConformant`` / ``ReconnectDiscontinuity`` → skipped.
    - ``DemuxError`` from ``feed`` → ``[{"event":"error","code":"STRICT_REJECTION"}]``.
    """
    demuxer = Demuxer()

    try:
        demuxer.feed(ts_bytes)
    except DemuxError:
        # Any DemuxError maps to the umbrella STRICT_REJECTION code — matches
        # ``demux_error_code()`` in the Rust normaliser which unconditionally
        # returns "STRICT_REJECTION" for every DemuxError variant.
        return [{"event": "error", "code": "STRICT_REJECTION"}]

    demuxer.flush()

    # Collect all raw events so we can build the stream_type map from
    # ProgramMap events before emitting media events — same two-pass
    # approach as the Rust normaliser.
    raw_events = list(demuxer)

    # pid → "0x1b"-style stream_type string, derived from ProgramMap events.
    stream_type_by_pid: dict[int, str] = {}
    for ev in raw_events:
        if isinstance(ev, DemuxEvent.ProgramMap):
            for prog in ev.programs:
                for si in prog.streams:
                    stream_type_by_pid[si.pid] = f"0x{si.stream_type:02x}"

    def stream_type_hex(pid: int, fallback_byte: int) -> str:
        return stream_type_by_pid.get(pid, f"0x{fallback_byte:02x}")

    events: list[dict[str, Any]] = []
    for ev in raw_events:
        if isinstance(ev, DemuxEvent.ProgramMap):
            pass  # topology — skip from output

        elif isinstance(ev, DemuxEvent.Video):
            raw = _video_payload_bytes(ev)
            events.append({
                "event": "video",
                "program": ev.stream.program_number,
                "pid": ev.stream.pid,
                "stream_type": stream_type_hex(ev.stream.pid, 0x1B),
                "pts": ev.pts.raw,
                "key": ev.random_access_indicator,
                "payload_sha256": _sha256_hex(raw),
            })

        elif isinstance(ev, DemuxEvent.Audio):
            # Audio payload is list[AdtsFrame] (AAC), list[Mpeg2AudioFrame] (MP2),
            # or bytes (LATM/AC-3/fallback).  The Rust normaliser hashes the
            # raw ``frames`` bytes from SamplePayload::Audio.  For typed frames,
            # concatenate their raw bytes via the buffer protocol; for bytes,
            # hash directly.
            if isinstance(ev.payload, (bytes, bytearray)):
                raw = bytes(ev.payload)
            else:
                # list[AdtsFrame] or list[Mpeg2AudioFrame] — each has a
                # `.payload` attribute returning the full frame bytes.
                raw = b"".join(bytes(f.payload) for f in ev.payload)
            events.append({
                "event": "audio",
                "program": ev.stream.program_number,
                "pid": ev.stream.pid,
                "stream_type": stream_type_hex(ev.stream.pid, 0x03),
                "pts": ev.pts.raw,
                "payload_sha256": _sha256_hex(raw),
            })

        elif isinstance(ev, DemuxEvent.Klv):
            events.append({
                "event": "klv",
                "program": ev.stream.program_number,
                "pid": ev.stream.pid,
                "stream_type": stream_type_hex(ev.stream.pid, 0x06),
                "set": _klv_set_from_ul(ev.payload),
            })

        elif isinstance(ev, DemuxEvent.UnknownSample):
            events.append({"event": "unknown", "pid": ev.stream.pid})

        elif isinstance(ev, (
            DemuxEvent.Discontinuity,
            DemuxEvent.NonConformant,
            DemuxEvent.ReconnectDiscontinuity,
        )):
            pass  # diagnostic — skip from output

        # No 'else' fall-through — if a new DemuxEvent subclass is added and
        # the normaliser doesn't handle it, the type-coverage gap becomes
        # visible via test failure on the golden comparison.

    return events


# ── Per-kind runners ──────────────────────────────────────────────────────────

def _run_demux(scenario_id: str, input_path: Path) -> list[dict[str, Any]]:
    """Run a ``kind=demux`` scenario and return the normalised core event list."""
    return _demux_to_core_events(input_path.read_bytes())


def _synthetic_h264_idr() -> bytes:
    """Reproduce the Rust generator's ``synthetic_h264_idr()``.

    4-byte Annex-B start code + 0x65 (IDR, nal_ref_idc=3) +
    15 deterministic filler bytes (0xA5 ^ i for i in 0..15).
    """
    hdr = bytes([0x00, 0x00, 0x00, 0x01, 0x65])
    body = bytes(0xA5 ^ i for i in range(15))
    return hdr + body


def _video_roundtrip_ts_bytes() -> bytes:
    """Python mirror of ``video_roundtrip_ts_bytes()`` in the Rust integration crate.

    Reproduces the same muxer recipe byte-for-byte:
      - program_number=1, pmt_pid=0x1000
      - video pid=0x1011, VideoCodec.H264
      - push_video(synthetic_h264_idr(), pts=0, key_frame=True)

    The Python Muxer is deterministic when given the same config and input
    sequence — verified byte-identical against the committed output.ts artifact.
    """
    prog = (
        MuxerProgramConfigBuilder(program_number=1, pmt_pid=0x1000)
        .add_video(pid=0x1011, codec=VideoCodec.H264)
        .build()
    )
    cfg = MuxerConfigBuilder().add_program(prog).build()
    mux = Muxer(cfg)
    mux.push_video(_synthetic_h264_idr(), pts=Pts90khz.from_raw(0), key_frame=True)
    out = bytearray()
    buf = bytearray(1316)  # 7 × 188
    while True:
        n = mux.pull(buf)
        if n == 0:
            break
        out.extend(buf[:n])
    return bytes(out)


def _run_roundtrip(
    scenario_id: str,
    input_path: Path,
    golden_extensions: dict[str, Any],
) -> list[dict[str, Any]]:
    """Run a ``kind=roundtrip`` scenario.

    Reproduces the mux recipe via the Python muxer, asserts byte-identity
    against the committed ``output.ts``, and asserts the sha256 matches
    ``extensions.output_sha256`` from the golden.

    Returns ``[]`` — roundtrip scenarios carry no media events.
    """
    if scenario_id == "video-roundtrip":
        fresh = _video_roundtrip_ts_bytes()
    else:
        pytest.fail(f"unknown roundtrip scenario id: {scenario_id!r}")

    committed = input_path.read_bytes()  # output.ts IS the input for roundtrip
    assert fresh == committed, (
        f"[{scenario_id}] Python muxer output differs from committed output.ts "
        f"({len(fresh)} bytes vs {len(committed)} bytes)"
    )

    expected_sha256 = golden_extensions.get("output_sha256", "")
    actual_sha256 = _sha256_hex(fresh)
    assert actual_sha256 == expected_sha256, (
        f"[{scenario_id}] sha256 mismatch: got {actual_sha256!r}, "
        f"expected {expected_sha256!r}"
    )
    return []


def _run_binding_contract(
    scenario_id: str,
    input_path: Path,
) -> list[dict[str, Any]]:
    """Run a ``kind=binding_contract`` scenario.

    For ``strict-rejection``:
    - Feeds garbage bytes to the default-config Demuxer.
    - Asserts a ``DemuxError`` is raised.
    - Maps it to ``{"event":"error","code":"STRICT_REJECTION"}``.
    - Asserts close/drop idempotence: the Demuxer can be destroyed after
      an error without crashing.
    """
    if scenario_id == "strict-rejection":
        input_bytes = input_path.read_bytes()

        # Feed garbage — must raise DemuxError.
        demuxer = Demuxer()
        raised = False
        try:
            demuxer.feed(input_bytes)
            # If feed somehow doesn't raise (extremely unlikely for 8192 × 0xFF
            # which has no sync byte), flush and drain to give the demuxer a
            # chance to surface the error.
            demuxer.flush()
            list(demuxer)
        except DemuxError:
            raised = True

        # Idempotent drop: destroy the demuxer after an error — must not panic.
        del demuxer

        if not raised:
            pytest.fail(
                f"[{scenario_id}] expected DemuxError on garbage input, got none"
            )

        return [{"event": "error", "code": "STRICT_REJECTION"}]

    pytest.fail(f"unknown binding_contract scenario id: {scenario_id!r}")


# ── Golden validation ─────────────────────────────────────────────────────────

_KNOWN_EVENT_TAGS = frozenset({"video", "audio", "klv", "unknown", "error"})


def _validate_golden_tags(golden: dict[str, Any], scenario_id: str) -> None:
    """Fail loudly if the committed golden contains an unrecognised event tag.

    An older consumer encountering an unknown tag must never silently skip —
    it must fail so the adapter author knows it needs updating.  Mirrors the
    Rust ``#[serde(deny_unknown_fields)]`` / no ``#[serde(other)]`` stance.
    """
    for entry in golden.get("core", []):
        tag = entry.get("event", "<missing>")
        if tag not in _KNOWN_EVENT_TAGS:
            pytest.fail(
                f"[{scenario_id}] committed golden contains unrecognised event "
                f"tag {tag!r}; this Python adapter must be updated to handle it."
            )


# ── Parametrised test ─────────────────────────────────────────────────────────

def _scenario_ids() -> list[str]:
    """Scenario ids for parametrization. NEVER returns an empty list silently:
    on any load failure it returns a single sentinel id so the parametrised
    test runs and fails loudly with the underlying reason (see the test body).

    Deliberately does NOT delegate to conftest's ``_load_manifest``: that helper
    calls ``pytest.skip`` on failure, which would silently empty the
    parametrization rather than emit a loud sentinel."""
    if not _TOML_AVAILABLE:
        return ["__error__:toml-parser-unavailable"]
    manifest_path = scenarios_dir() / "scenarios.toml"
    if not manifest_path.is_file():
        return [f"__error__:manifest-not-found:{manifest_path}"]
    try:
        if sys.version_info >= (3, 11):
            import tomllib as _toml
        else:
            import tomli as _toml  # type: ignore[import-not-found]
        with open(manifest_path, "rb") as fh:
            data = _toml.load(fh)
        ids = [e["id"] for e in data.get("scenario", [])]
    except Exception as exc:  # noqa: BLE001 — surfaced as a loud test failure below
        return [f"__error__:manifest-parse-failed:{exc!r}"]
    return ids if ids else ["__error__:manifest-has-zero-scenarios"]


@pytest.mark.parametrize("scenario_id", _scenario_ids())
def test_scenario_matches_committed_golden(scenario_id: str) -> None:
    """For each scenario in scenarios.toml, run the Python adapter and assert
    the result equals the committed golden.json."""
    if scenario_id.startswith("__error__:"):
        pytest.fail(
            "scenario collection failed: "
            + scenario_id.removeprefix("__error__:")
            + " — the Python cross-binding contract suite collected no real "
            "scenarios. Fix the manifest/TOML parser; do not let this pass."
        )
    manifest_entry, sdir = require_scenario(scenario_id)
    kind = manifest_entry["kind"]
    input_rel = Path(manifest_entry["input"])
    golden_rel = Path(manifest_entry["golden"])

    sd = scenarios_dir()
    input_path = (sd / input_rel).resolve()
    golden_path = (sd / golden_rel).resolve()

    if not input_path.is_file():
        pytest.skip(f"input artifact missing: {input_path}")
    if not golden_path.is_file():
        pytest.skip(f"golden missing: {golden_path}")

    with open(golden_path) as fh:
        committed = json.load(fh)

    _validate_golden_tags(committed, scenario_id)

    if kind == "demux":
        observed_core = _run_demux(scenario_id, input_path)
    elif kind == "roundtrip":
        observed_core = _run_roundtrip(
            scenario_id,
            input_path,
            committed.get("extensions") or {},
        )
    elif kind == "binding_contract":
        observed_core = _run_binding_contract(scenario_id, input_path)
    else:
        pytest.fail(
            f"unknown scenario kind {kind!r} for scenario {scenario_id!r}"
        )

    observed = {
        "schema_version": committed["schema_version"],
        "lossy": committed["lossy"],
        "core": observed_core,
        "extensions": committed.get("extensions"),
    }

    assert observed == committed, (
        f"scenario '{scenario_id}': Python adapter output differs from "
        f"committed golden.\n"
        f"  observed_core: {json.dumps(observed_core, indent=2)}\n"
        f"  committed_core: {json.dumps(committed['core'], indent=2)}"
    )
