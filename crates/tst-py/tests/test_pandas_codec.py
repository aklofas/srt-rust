"""Phase 6: NAL / OBU / audio frame DataFrame adapter tests."""

import pytest

pytestmark = pytest.mark.pandas

import pandas as pd  # noqa: E402

from tstrans.codec import NalUnit, Obu, ObuExtension
from tstrans.pandas import nals_to_dataframe, obus_to_dataframe


def test_nals_to_dataframe_h264_basic():
    nals = [
        NalUnit.h264(nal_type=7, ref_idc=3, payload=b"\x00\x01"),
        NalUnit.h264(nal_type=8, ref_idc=3, payload=b"\x02"),
        NalUnit.h264(nal_type=5, ref_idc=3, payload=b"\x03\x04\x05"),
    ]
    df = nals_to_dataframe(nals)
    assert len(df) == 3
    assert df["kind"].tolist() == ["H264", "H264", "H264"]
    assert df["nal_type"].tolist() == [7, 8, 5]
    assert df["nal_type_name"].tolist() == ["SPS", "PPS", "IDR_SLICE"]
    assert df["payload_len"].tolist() == [2, 1, 3]


def test_nals_to_dataframe_h264_has_ref_idc_not_layer():
    nals = [NalUnit.h264(nal_type=5, ref_idc=2, payload=b"")]
    df = nals_to_dataframe(nals)
    assert df["ref_idc"].iloc[0] == 2
    assert pd.isna(df["layer_id"].iloc[0])
    assert pd.isna(df["temporal_id_plus1"].iloc[0])


def test_nals_to_dataframe_h265_has_layer_not_ref_idc():
    nals = [NalUnit.h265(nal_type=19, layer_id=1, temporal_id_plus1=2, payload=b"")]
    df = nals_to_dataframe(nals)
    assert df["layer_id"].iloc[0] == 1
    assert df["temporal_id_plus1"].iloc[0] == 2
    assert pd.isna(df["ref_idc"].iloc[0])


def test_nals_to_dataframe_unknown_nal_type():
    nals = [NalUnit.h264(nal_type=31, ref_idc=0, payload=b"")]
    df = nals_to_dataframe(nals)
    assert df["nal_type_name"].iloc[0] == "unknown_31"


def test_nals_to_dataframe_empty():
    df = nals_to_dataframe([])
    assert isinstance(df, pd.DataFrame)
    assert len(df) == 0


def test_nals_to_dataframe_pts_broadcast():
    nals = [NalUnit.h264(nal_type=5, ref_idc=3, payload=b"\x00")]
    df = nals_to_dataframe(nals, pts=123.0)
    assert "pts_ms" in df.columns
    assert df["pts_ms"].iloc[0] == 123.0


def test_nals_to_dataframe_no_pts_omits_column():
    nals = [NalUnit.h264(nal_type=5, ref_idc=3, payload=b"\x00")]
    df = nals_to_dataframe(nals)
    assert "pts_ms" not in df.columns


def test_obus_to_dataframe_basic():
    obus = [
        Obu(obu_type=1, extension=None, payload=b"\xaa"),
        Obu(obu_type=6, extension=ObuExtension(temporal_id=1, spatial_id=0), payload=b"\xbb\xcc"),
    ]
    df = obus_to_dataframe(obus)
    assert len(df) == 2
    assert df["obu_type_name"].tolist() == ["SEQUENCE_HEADER", "FRAME"]
    assert df["payload_len"].tolist() == [1, 2]
    assert pd.isna(df["temporal_id"].iloc[0])
    assert df["temporal_id"].iloc[1] == 1
    assert df["spatial_id"].iloc[1] == 0


def test_obus_to_dataframe_pts_broadcast():
    obus = [Obu(obu_type=6, extension=None, payload=b"")]
    df = obus_to_dataframe(obus, pts=42.5)
    assert df["pts_ms"].iloc[0] == 42.5


def test_obus_to_dataframe_empty():
    df = obus_to_dataframe([])
    assert isinstance(df, pd.DataFrame)
    assert len(df) == 0
