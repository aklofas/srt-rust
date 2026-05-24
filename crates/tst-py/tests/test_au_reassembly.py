"""Python-side coverage for multi-cell AU cell reassembly.

End-to-end reassembly behavior is verified in the Rust integration tests
(Task 5). Python tests here cover only the binding surface — that the new
fields exist on `_KlvEvent`, that `MultiCellAuReason` is importable as a
PyO3 `eq_int` enum, and that `_NonConformantEvent` carries the optional
typed reason.
"""

from __future__ import annotations

import enum

from tstrans.mpegts import (
    DemuxEvent,
    MultiCellAuReason,
    NonConformantKind,
    Pts90khz,
    StreamId,
    StreamKindTag,
)


def test_multi_cell_au_reason_is_an_enum_with_four_variants() -> None:
    """The reason enum mirrors Rust's `MultiCellAuReason`."""
    assert MultiCellAuReason.ORPHAN is not None
    assert MultiCellAuReason.SEQUENCE_GAP is not None
    assert MultiCellAuReason.CONCURRENT_FIRST is not None
    assert MultiCellAuReason.OVERFLOW is not None


def test_multi_cell_au_reason_equality_uses_eq_int_semantics() -> None:
    """PyO3 `eq_int` enums compare with `==` (not `is`).

    See `reference_pyo3_0_22_rust_2024_patterns` memory.
    """
    assert MultiCellAuReason.ORPHAN == MultiCellAuReason.ORPHAN
    assert MultiCellAuReason.ORPHAN != MultiCellAuReason.OVERFLOW
    assert MultiCellAuReason.SEQUENCE_GAP != MultiCellAuReason.CONCURRENT_FIRST


def test_klv_event_has_was_reassembled_and_cell_count_with_defaults() -> None:
    """Reassembly fields exist on `_KlvEvent` with backward-compat defaults."""
    from tstrans.mpegts import MetadataKindTag, _KlvEvent

    stream = StreamId(
        pid=0x100,
        kind=StreamKindTag.KLV_SYNC,
        codec=None,
        program_number=1,
    )
    # Construct without the new fields — defaults must keep the call working.
    ev = _KlvEvent(
        stream=stream,
        pts=Pts90khz.from_raw(90000),
        kind=MetadataKindTag.KLV_SYNC_AU_CELL,
        payload=b"",
    )
    assert ev.was_reassembled is False
    assert ev.cell_count == 1


def test_klv_event_accepts_explicit_reassembly_fields() -> None:
    """Multi-cell reassembled AU populates the new fields."""
    from tstrans.mpegts import MetadataKindTag, _KlvEvent

    stream = StreamId(
        pid=0x100,
        kind=StreamKindTag.KLV_SYNC,
        codec=None,
        program_number=1,
    )
    ev = _KlvEvent(
        stream=stream,
        pts=Pts90khz.from_raw(90000),
        kind=MetadataKindTag.KLV_SYNC_AU_CELL,
        payload=b"\x01\x02\x03",
        was_reassembled=True,
        cell_count=3,
    )
    assert ev.was_reassembled is True
    assert ev.cell_count == 3


def test_non_conformant_event_has_optional_multi_cell_reason() -> None:
    """`_NonConformantEvent` carries the typed reason on MULTI_CELL_AU."""
    from tstrans.mpegts import _NonConformantEvent

    stream = StreamId(
        pid=0x100,
        kind=StreamKindTag.KLV_SYNC,
        codec=None,
        program_number=1,
    )

    # Default: None for any non-MultiCellAu issue.
    ev = _NonConformantEvent(
        stream=stream,
        issue="some other issue",
        kind=NonConformantKind.PCR_ANOMALY,
    )
    assert ev.multi_cell_au_reason is None

    # MultiCellAu issues carry the typed reason.
    ev2 = _NonConformantEvent(
        stream=stream,
        issue="orphan continuation",
        kind=NonConformantKind.MULTI_CELL_AU,
        multi_cell_au_reason=MultiCellAuReason.ORPHAN,
    )
    assert ev2.multi_cell_au_reason == MultiCellAuReason.ORPHAN


def test_multi_cell_au_reason_in_module_all() -> None:
    """`MultiCellAuReason` is in `tstrans.mpegts.__all__`."""
    from tstrans import mpegts

    assert "MultiCellAuReason" in mpegts.__all__


def test_klv_event_via_demuxevent_namespace_attribute() -> None:
    """`DemuxEvent.Klv` is the same class as `_KlvEvent` and has the new fields."""
    from tstrans.mpegts import _KlvEvent

    assert DemuxEvent.Klv is _KlvEvent
    # dataclass field set — sanity check on the public surface.
    field_names = {f.name for f in DemuxEvent.Klv.__dataclass_fields__.values()}
    assert "was_reassembled" in field_names
    assert "cell_count" in field_names
