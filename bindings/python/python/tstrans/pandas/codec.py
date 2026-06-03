"""tstrans.pandas.codec — NAL/OBU/audio frame DataFrame adapters."""

from typing import Any, Iterable

from tstrans.pandas._imports import require_pandas
from tstrans.pandas._nal_type_names import nal_name, obu_name


def _enum_name(value: Any) -> str | None:
    """Extract just the variant name from a qualified enum string.

    `AacProfile.LC` → `LC`. Falls back to str() for non-dotted forms
    (e.g., `AacChannelLayout(channels=2)` stays as-is — that's a struct).
    """
    if value is None:
        return None
    s = str(value)
    return s.rsplit(".", 1)[-1] if "." in s and "(" not in s else s


_ADTS_COLUMNS = (
    "profile", "sample_rate_hz", "channel_configuration",
    "channel_layout", "frame_length_bytes", "samples_per_frame",
    "num_raw_data_blocks", "has_crc", "mpeg_version",
    "raw_header_len", "payload_len", "byte_offset",
)

_MP2_COLUMNS = (
    "layer", "version", "bitrate_kbps", "sample_rate_hz",
    "channel_mode", "channels", "frame_length_bytes",
    "samples_per_frame", "has_crc", "payload_len", "byte_offset",
)


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


def audio_frames_to_dataframe(frames: Iterable[Any]):
    """Convert a list of AdtsFrame or Mpeg2AudioFrame to a DataFrame.

    Polymorphic — detects type from first element.

    Args:
        frames: iterable of AdtsFrame or Mpeg2AudioFrame instances
            (homogeneous required).

    Returns:
        pandas.DataFrame. AdtsFrame schema: profile, sample_rate_hz,
        channel_configuration, channel_layout, frame_length_bytes,
        samples_per_frame, num_raw_data_blocks, has_crc, mpeg_version,
        raw_header_len, payload_len, byte_offset. Mpeg2AudioFrame schema:
        layer, version, bitrate_kbps, sample_rate_hz, channel_mode,
        channels, frame_length_bytes, samples_per_frame, has_crc,
        payload_len, byte_offset.

    Raises:
        TypeError: on mixed-type input.
        ImportError: if [pandas] extra not installed.
    """
    pd, np = require_pandas()
    frames_list = list(frames)
    if not frames_list:
        # Empty: return empty AdtsFrame-schema DataFrame
        return pd.DataFrame(columns=list(_ADTS_COLUMNS))

    type_names = {type(f).__name__ for f in frames_list}
    if len(type_names) > 1:
        raise TypeError(
            f"audio_frames_to_dataframe requires homogeneous frame types; "
            f"got mixed {sorted(type_names)}"
        )

    first = frames_list[0]
    first_type = type(first).__name__

    if first_type == "AdtsFrame":
        return _adts_frames_to_dataframe(frames_list, pd, np)
    elif first_type == "Mpeg2AudioFrame":
        return _mpeg2_audio_frames_to_dataframe(frames_list, pd, np)
    else:
        raise TypeError(f"unsupported audio frame type: {first_type}")


def _adts_frames_to_dataframe(frames, pd, np):
    rows = []
    cumulative = 0
    for f in frames:
        rows.append({
            "profile": _enum_name(f.profile),
            "sample_rate_hz": f.sample_rate_hz,
            "channel_configuration": f.channel_configuration,
            "channel_layout": _enum_name(f.channel_layout),
            "frame_length_bytes": f.frame_length_bytes,
            "samples_per_frame": f.samples_per_frame,
            "num_raw_data_blocks": f.num_raw_data_blocks,
            "has_crc": f.has_crc,
            "mpeg_version": _enum_name(f.mpeg_version),
            "raw_header_len": len(f.raw_header),
            "payload_len": len(f.payload),
            "byte_offset": cumulative,
        })
        cumulative += f.frame_length_bytes
    return pd.DataFrame(rows, columns=list(_ADTS_COLUMNS))


def _mpeg2_audio_frames_to_dataframe(frames, pd, np):
    rows = []
    cumulative = 0
    for f in frames:
        rows.append({
            "layer": _enum_name(f.layer),
            "version": _enum_name(f.version),
            "bitrate_kbps": f.bitrate_kbps,
            "sample_rate_hz": f.sample_rate_hz,
            "channel_mode": _enum_name(f.channel_mode),
            "channels": f.channels,
            "frame_length_bytes": f.frame_length_bytes,
            "samples_per_frame": f.samples_per_frame,
            "has_crc": f.has_crc,
            "payload_len": len(f.payload),
            "byte_offset": cumulative,
        })
        cumulative += f.frame_length_bytes
    return pd.DataFrame(rows, columns=list(_MP2_COLUMNS))
