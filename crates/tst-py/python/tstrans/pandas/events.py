"""tstrans.pandas.events — DemuxEvent DataFrame adapter.

Union-schema DataFrame across all DemuxEvent kinds. Payloads (NAL lists,
KLV bytes, etc.) DO NOT land in the DataFrame — they stay on the original
event objects. The DataFrame is for analysis / filtering / plotting only.
"""

from typing import Any, Iterable

from tstrans.pandas._imports import require_pandas

# Columns in the union schema, in canonical order
_COLUMNS = [
    "kind",
    "pts_raw",
    "pts_ms",
    "dts_ms",
    "pid",
    "stream_type",
    "codec",
    "payload_len",
    "nal_count",
    "random_access",
    "has_codec_parse_error",
    "issue",
    "issue_kind",
]


def events_to_dataframe(events: Iterable[Any]):
    """Convert a sequence of DemuxEvents to a pandas DataFrame.

    Args:
        events: iterable of DemuxEvent subclass instances from
            `tstrans.io.parse_file()` or similar.

    Returns:
        pandas.DataFrame with RangeIndex; columns are the union schema
        across all event kinds (NaN-padded where columns don't apply).

    Raises:
        ImportError: if [pandas] extra not installed.
    """
    pd, _np = require_pandas()
    rows = [_event_to_row(e) for e in events]
    df = pd.DataFrame(rows, columns=_COLUMNS)
    return df


def _event_to_row(event: Any) -> dict:
    """Map a DemuxEvent instance to a dict matching _COLUMNS keys.

    Uses duck-typing on event attributes to handle all subclass shapes
    (Video / Audio / Subtitle / Klv / ProgramMap / NonConformant /
    Discontinuity / ReconnectDiscontinuity) without importing them
    explicitly.
    """
    row: dict = {c: None for c in _COLUMNS}
    kind = type(event).__name__
    # Strip leading underscore if present (e.g., _VideoEvent → VideoEvent)
    if kind.startswith("_"):
        kind = kind[1:]
    # Normalize to the design-doc kind labels. Video/Audio/Subtitle all
    # collapse to "Sample" because they share the same shape (stream,
    # pts, dts, codec, payload, codec_parse_error). KlvEvent maps to
    # "Metadata" per the design contract.
    kind_map = {
        "VideoEvent": "Sample",
        "AudioEvent": "Sample",
        "SubtitleEvent": "Sample",
        "KlvEvent": "Metadata",
        "ProgramMapEvent": "ProgramMap",
        "NonConformantEvent": "NonConformant",
        "DiscontinuityEvent": "Discontinuity",
        "ReconnectDiscontinuityEvent": "ReconnectDiscontinuity",
    }
    row["kind"] = kind_map.get(kind, kind)

    # PTS (Pts90khz exposes .raw and .ms)
    pts = getattr(event, "pts", None)
    if pts is not None:
        row["pts_raw"] = getattr(pts, "raw", None)
        row["pts_ms"] = getattr(pts, "ms", None)

    # DTS (only on some video samples)
    dts = getattr(event, "dts", None)
    if dts is not None:
        row["dts_ms"] = getattr(dts, "ms", None)

    # Stream / pid / codec (only on Sample / Metadata / NonConformant /
    # Discontinuity events; StreamId is `None` on NonConformant rows
    # that describe a global PSI issue rather than a per-stream issue).
    stream = getattr(event, "stream", None)
    if stream is not None:
        row["pid"] = getattr(stream, "pid", None)
        kind_tag = getattr(stream, "kind", None)
        row["stream_type"] = (
            kind_tag if isinstance(kind_tag, str) else getattr(kind_tag, "name", None)
        )
        # Sample events carry their own .codec attribute (more precise than
        # stream.codec — e.g. WebVTT subtitle codec). Fall back to
        # stream.codec when the event lacks one.
        codec = getattr(event, "codec", None)
        if codec is None:
            codec = getattr(stream, "codec", None)
        row["codec"] = (
            codec if isinstance(codec, str) else getattr(codec, "name", None)
        )
    else:
        # No stream — still try the event's own codec attribute.
        codec = getattr(event, "codec", None)
        if codec is not None:
            row["codec"] = (
                codec if isinstance(codec, str) else getattr(codec, "name", None)
            )

    # Payload (typed list for Video/Audio when codec parsed; bytes for KLV,
    # subtitle, and audio when codec_parse_error fell back).
    payload = getattr(event, "payload", None)
    if payload is not None:
        if isinstance(payload, (bytes, bytearray)):
            row["payload_len"] = len(payload)
        elif hasattr(payload, "__len__"):
            row["payload_len"] = len(payload)
            # nal_count is video-only. Audio events (_AudioEvent) also carry
            # typed lists (AdtsFrame / Mpeg2AudioFrame) that satisfy
            # hasattr(__len__), but those are NOT NAL units — populating
            # nal_count for them would give analysts filtering
            # `df[df["nal_count"] > N]` false positives on audio rows.
            if type(event).__name__ == "_VideoEvent":
                row["nal_count"] = len(payload)

    # Random access indicator (video samples only)
    rai = getattr(event, "random_access_indicator", None)
    if rai is not None:
        row["random_access"] = rai

    # codec_parse_error (Phase 5 codec-parse fallback — present on Video
    # and Audio events; truthy iff parsing failed and payload is raw bytes)
    cpe = getattr(event, "codec_parse_error", None)
    # Only set the column when the attribute exists on this event kind;
    # leave None for events that never carry codec_parse_error.
    if hasattr(event, "codec_parse_error"):
        row["has_codec_parse_error"] = cpe is not None

    # NonConformantEvent fields (issue str + kind enum). DiscontinuityEvent
    # and KlvEvent also have `.kind` but no `.issue`; the issue check gates
    # the issue_kind assignment so we only populate it on NonConformant.
    issue = getattr(event, "issue", None)
    if issue is not None:
        row["issue"] = issue
        issue_kind = getattr(event, "kind", None)
        if issue_kind is not None:
            row["issue_kind"] = (
                issue_kind if isinstance(issue_kind, str) else getattr(issue_kind, "name", str(issue_kind))
            )

    return row
