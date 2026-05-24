"""tstrans.pandas.codec — NAL/OBU/audio frame DataFrame adapters."""

from typing import Any, Iterable

from tstrans.pandas._imports import require_pandas
from tstrans.pandas._nal_type_names import nal_name, obu_name


def nals_to_dataframe(nals: Iterable[Any], pts: float | None = None):
    """Convert a list of NalUnit instances to a DataFrame.

    Args:
        nals: iterable of NalUnit instances (H264/H265/H266 mixed allowed).
        pts: optional ms timestamp to broadcast across all rows as `pts_ms`.
            If None, the `pts_ms` column is omitted entirely.

    Returns:
        pandas.DataFrame with columns kind, nal_type, nal_type_name, ref_idc,
        layer_id, temporal_id_plus1, payload_len, (pts_ms if pts supplied).

    Raises:
        ImportError: if [pandas] extra not installed.
    """
    pd, np = require_pandas()
    rows = []
    for nal in nals:
        rows.append({
            "kind": nal.kind,
            "nal_type": nal.nal_type,
            "nal_type_name": nal_name(nal.kind, nal.nal_type),
            "ref_idc": nal.ref_idc,  # None for H.265/H.266 → becomes NaN
            "layer_id": nal.layer_id,  # None for H.264 → becomes NaN
            "temporal_id_plus1": nal.temporal_id_plus1,
            "payload_len": len(nal.payload),
        })
    df = pd.DataFrame(rows, columns=[
        "kind", "nal_type", "nal_type_name",
        "ref_idc", "layer_id", "temporal_id_plus1", "payload_len",
    ])
    if pts is not None:
        df["pts_ms"] = pts
    return df


def obus_to_dataframe(obus: Iterable[Any], pts: float | None = None):
    """Convert a list of Obu instances to a DataFrame.

    Args:
        obus: iterable of Obu instances.
        pts: optional ms timestamp to broadcast across all rows as `pts_ms`.

    Returns:
        pandas.DataFrame with columns obu_type, obu_type_name, temporal_id,
        spatial_id, payload_len, (pts_ms if pts supplied).

    Raises:
        ImportError: if [pandas] extra not installed.
    """
    pd, np = require_pandas()
    rows = []
    for obu in obus:
        ext = obu.extension
        rows.append({
            "obu_type": obu.obu_type,
            "obu_type_name": obu_name(obu.obu_type),
            "temporal_id": ext.temporal_id if ext is not None else None,
            "spatial_id": ext.spatial_id if ext is not None else None,
            "payload_len": len(obu.payload),
        })
    df = pd.DataFrame(rows, columns=[
        "obu_type", "obu_type_name", "temporal_id", "spatial_id", "payload_len",
    ])
    if pts is not None:
        df["pts_ms"] = pts
    return df


# audio_frames_to_dataframe defined in Task 7
def audio_frames_to_dataframe(frames):
    """Stub — replaced in Task 7."""
    raise NotImplementedError("audio_frames_to_dataframe is implemented in Phase 6 Task 7")
