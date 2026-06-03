"""tstrans.pandas.klv — KLV record DataFrame adapters.

Polymorphic dispatcher over the 4 supported MISB sets:
  - UasDatalinkLs (ST 0601) → DataFrame with DatetimeIndex from `timestamp_us`
  - SecurityLs (ST 0102) → DataFrame with RangeIndex (no internal timestamp)
  - PrecisionTimeStampPack (ST 0605) → DataFrame with DatetimeIndex; the
    typed `TimeStatus` byte decomposes into 4 bool columns.
  - VmtiLs (ST 0903) — mode="summary" (1 row per record with `num_targets`)
    or mode="targets" (1 row per VTargetPack with MultiIndex [pts, target_id])

Column selection rule: only data fields — `dataclass.fields()` source-of-
truth, NOT `dir(obj)`. This excludes method synthesizers (`frame_center`,
`sensor_position`, etc., which compute composites on demand from the
underlying flat scalars) and class-level helpers. `field_errors` collapses
to a `|`-joined string column (per-error format `tag<N>:<kind>:<message>`).

The [pandas] extra must be installed; missing import yields a friendly
ImportError via `require_pandas()`.
"""

from dataclasses import fields, is_dataclass
from typing import Any, Iterable

from tstrans.klv import (
    PrecisionTimeStampPack,
    SecurityLs,
    UasDatalinkLs,
    VmtiLs,
)
from tstrans.pandas._imports import require_pandas


def klv_to_dataframe(records: Iterable[Any], *, mode: str = "summary"):
    """Polymorphic dispatcher. Records must be homogeneous (one set type per call).

    Args:
        records: iterable of KLV LS records (UasDatalinkLs, SecurityLs,
            PrecisionTimeStampPack, or VmtiLs). Empty iterable returns an
            empty DataFrame.
        mode: "summary" (default, 1 row per record) or "targets"
            (VmtiLs only — 1 row per VTargetPack with MultiIndex
            [pts, target_id]). Ignored for non-VMTI inputs.

    Returns:
        pandas.DataFrame.

    Raises:
        TypeError: if records contain mixed types or an unsupported type.
        ImportError: if [pandas] extra not installed.
    """
    if mode not in {"summary", "targets"}:
        raise ValueError(
            f"mode must be 'summary' or 'targets'; got {mode!r}"
        )
    pd, np = require_pandas()
    records_list = list(records)
    if not records_list:
        return pd.DataFrame()

    types = {type(r) for r in records_list}
    if len(types) > 1:
        type_names = sorted(t.__name__ for t in types)
        raise TypeError(
            f"klv_to_dataframe requires homogeneous record types; got mixed {type_names}"
        )
    rec_type = next(iter(types))

    if rec_type is UasDatalinkLs:
        return _uas_to_dataframe(records_list, pd, np)
    if rec_type is SecurityLs:
        return _security_to_dataframe(records_list, pd, np)
    if rec_type is PrecisionTimeStampPack:
        return _timestamp_pack_to_dataframe(records_list, pd, np)
    if rec_type is VmtiLs:
        if mode == "targets":
            return _vmti_targets_to_dataframe(records_list, pd, np)
        return _vmti_summary_to_dataframe(records_list, pd, np)
    raise TypeError(f"unsupported KLV record type: {rec_type.__name__}")


# --- per-set builders ----------------------------------------------------


def _uas_to_dataframe(records, pd, np):
    """Build DataFrame for UasDatalinkLs. DatetimeIndex from `timestamp_us`.

    Iterates dataclass fields directly so method synthesizers like
    `frame_center()` / `sensor_position()` / `corners()` do NOT leak as
    columns — the underlying flat scalars (`frame_center_lat_deg`, etc.)
    carry the data.
    """
    field_names = _dataclass_field_names(UasDatalinkLs)
    rows = []
    timestamps = []
    for rec in records:
        row = {}
        for attr in field_names:
            val = getattr(rec, attr)
            if attr == "field_errors":
                row[attr] = _field_errors_to_str(val)
            else:
                row[attr] = val
        rows.append(row)
        timestamps.append(getattr(rec, "timestamp_us", None))

    df = pd.DataFrame(rows, columns=field_names)
    if any(t is not None for t in timestamps):
        df.index = pd.DatetimeIndex(
            [pd.Timestamp(t, unit="us", tz="UTC") if t is not None else pd.NaT for t in timestamps],
            name="pts",
        )
    return df


def _security_to_dataframe(records, pd, np):
    """Build DataFrame for SecurityLs. RangeIndex (no internal timestamp)."""
    field_names = _dataclass_field_names(SecurityLs)
    rows = []
    for rec in records:
        row = {}
        for attr in field_names:
            val = getattr(rec, attr)
            if attr == "field_errors":
                row[attr] = _field_errors_to_str(val)
            elif _is_enum_like(val):
                row[attr] = val.name
            else:
                row[attr] = val
        rows.append(row)
    return pd.DataFrame(rows, columns=field_names)


def _timestamp_pack_to_dataframe(records, pd, np):
    """Build DataFrame for PrecisionTimeStampPack (ST 0605).

    Decomposes the typed `TimeStatus` byte into 4 bool columns via the
    Rust-side property accessors (is_locked / has_discontinuity /
    is_reverse_jump / reserved_bits_valid). DatetimeIndex from
    `timestamp_us`.
    """
    rows = []
    timestamps = []
    for rec in records:
        ts = rec.timestamp_us
        time_status = rec.time_status
        row = {
            "timestamp_us": ts,
            "is_locked": time_status.is_locked,
            "has_discontinuity": time_status.has_discontinuity,
            "is_reverse_jump": time_status.is_reverse_jump,
            "reserved_bits_valid": time_status.reserved_bits_valid,
            "time_status_raw": time_status.raw,
        }
        rows.append(row)
        timestamps.append(ts)
    df = pd.DataFrame(rows)
    df.index = pd.DatetimeIndex(
        [pd.Timestamp(t, unit="us", tz="UTC") for t in timestamps], name="pts"
    )
    return df


def _vmti_summary_to_dataframe(records, pd, np):
    """One row per VMTI record. `targets` collapses to `num_targets` count."""
    field_names = _dataclass_field_names(VmtiLs)
    rows = []
    timestamps = []
    for rec in records:
        row = {}
        for attr in field_names:
            val = getattr(rec, attr)
            if attr == "targets":
                row["num_targets"] = len(val) if val is not None else 0
                continue
            if attr == "field_errors":
                row[attr] = _field_errors_to_str(val)
                continue
            row[attr] = val
        rows.append(row)
        # VmtiLs.precision_time_stamp is `int | None` (microseconds since Unix epoch),
        # NOT a PrecisionTimeStampPack — see bindings/python/python/tstrans/klv.py:360
        # and bindings/python/src/klv.rs:521-523. Use the raw int directly.
        timestamps.append(getattr(rec, "precision_time_stamp", None))

    df = pd.DataFrame(rows)
    if any(t is not None for t in timestamps):
        df.index = pd.DatetimeIndex(
            [pd.Timestamp(t, unit="us", tz="UTC") if t is not None else pd.NaT for t in timestamps],
            name="pts",
        )
    return df


def _vmti_targets_to_dataframe(records, pd, np):
    """One row per VTargetPack across all records. MultiIndex [pts, target_id]."""
    # Lazy import — VTargetPack isn't referenced at module level.
    from tstrans.klv import VTargetPack

    field_names = _dataclass_field_names(VTargetPack)
    rows = []
    pts_values = []
    target_ids = []
    for rec in records:
        # VmtiLs.precision_time_stamp is `int | None` (microseconds since Unix
        # epoch), NOT a PrecisionTimeStampPack — use the raw int directly.
        ts = getattr(rec, "precision_time_stamp", None)
        targets = getattr(rec, "targets", None) or ()
        for target in targets:
            row = {}
            tid = None
            for attr in field_names:
                val = getattr(target, attr)
                if attr == "target_id":
                    tid = val
                    continue
                if attr == "field_errors":
                    row[attr] = _field_errors_to_str(val)
                    continue
                row[attr] = val
            rows.append(row)
            pts_values.append(ts)
            target_ids.append(tid)

    df = pd.DataFrame(rows)
    if pts_values:
        ts_index = [
            pd.Timestamp(t, unit="us", tz="UTC") if t is not None else pd.NaT
            for t in pts_values
        ]
        df.index = pd.MultiIndex.from_arrays(
            [ts_index, target_ids], names=["pts", "target_id"]
        )
    return df


# --- helpers -------------------------------------------------------------


def _dataclass_field_names(cls) -> list[str]:
    """Returns dataclass field names (declaration order). Source-of-truth for
    column selection — excludes method synthesizers and class helpers.
    """
    if not is_dataclass(cls):
        raise TypeError(f"{cls.__name__} is not a dataclass")
    return [f.name for f in fields(cls)]


def _field_errors_to_str(val) -> str:
    """Collapse a tuple/list of KlvFieldError values to a string.

    Uses `|` as the per-error joiner (NOT comma) and a deterministic
    `tag<N>:<kind>:<message>` per-error format so the column is parseable
    even when `message` contains commas. Kind is projected to its enum
    `.value` (e.g. "out_of_range") when available, falling back to `str`.
    """
    if not val:
        return ""

    def _fmt(e):
        kind = getattr(e, "kind", None)
        kind_str = kind.value if hasattr(kind, "value") else str(kind)
        return f"tag{e.tag}:{kind_str}:{e.message}"

    return " | ".join(_fmt(e) for e in val)


def _is_enum_like(val) -> bool:
    """True if val looks like an enum (has `.name` and is not str/bytes)."""
    if val is None or isinstance(val, (str, bytes, int, float, bool)):
        return False
    return hasattr(val, "name") and not callable(getattr(val, "name", None))
