"""Phase 6: NumPy zero-copy accessor tests for tstrans.codec types."""

import pytest

pytestmark = pytest.mark.pandas

import numpy as np  # noqa: E402

from tstrans.codec import (
    AdtsFrame,
    Av1FrameHeaderLight,
    Av1SequenceHeader,
    H264Pps,
    H264SliceHeaderLight,
    H264Sps,
    H265Pps,
    H265SliceHeaderLight,
    H265Sps,
    H265Vps,
    H266Pps,
    H266SliceHeaderLight,
    H266Sps,
    H266Vps,
    Mpeg2AudioFrame,
    NalUnit,
    Obu,
)


def test_nal_unit_payload_np_returns_ndarray():
    nal = NalUnit.h264(nal_type=5, ref_idc=3, payload=b"\x01\x02\x03")
    arr = nal.payload_np
    assert isinstance(arr, np.ndarray)
    assert arr.dtype == np.uint8
    assert arr.shape == (3,)
    assert bytes(arr) == b"\x01\x02\x03"


def test_nal_unit_payload_np_is_read_only():
    nal = NalUnit.h264(nal_type=5, ref_idc=3, payload=b"\x01\x02\x03")
    arr = nal.payload_np
    with pytest.raises(ValueError, match="read-only"):
        arr[0] = 99


def test_nal_unit_payload_np_empty_bytes():
    nal = NalUnit.h264(nal_type=0, ref_idc=0, payload=b"")
    arr = nal.payload_np
    assert arr.shape == (0,)


def test_obu_payload_np_returns_ndarray():
    obu = Obu(obu_type=6, extension=None, payload=b"\xaa\xbb")
    arr = obu.payload_np
    assert arr.dtype == np.uint8
    assert bytes(arr) == b"\xaa\xbb"


def test_h265_nal_unit_payload_np():
    nal = NalUnit.h265(nal_type=19, layer_id=0, temporal_id_plus1=1, payload=b"\xff")
    assert nal.payload_np.dtype == np.uint8


def test_h266_nal_unit_payload_np():
    nal = NalUnit.h266(nal_type=7, layer_id=0, temporal_id_plus1=1, payload=b"\xab")
    assert nal.payload_np.dtype == np.uint8


# Parametrize over every byte-bearing class to enforce coverage
@pytest.mark.parametrize("cls_name,attr", [
    ("NalUnit", "payload_np"),
    ("Obu", "payload_np"),
    ("AdtsFrame", "payload_np"),
    ("Mpeg2AudioFrame", "payload_np"),
    ("H264Sps", "raw_rbsp_np"),
    ("H264Pps", "raw_rbsp_np"),
    ("H264SliceHeaderLight", "raw_rbsp_np"),
    ("H265Sps", "raw_rbsp_np"),
    ("H265Pps", "raw_rbsp_np"),
    ("H265Vps", "raw_rbsp_np"),
    ("H265SliceHeaderLight", "raw_rbsp_np"),
    ("H266Sps", "raw_rbsp_np"),
    ("H266Pps", "raw_rbsp_np"),
    ("H266Vps", "raw_rbsp_np"),
    ("H266SliceHeaderLight", "raw_rbsp_np"),
    ("Av1SequenceHeader", "raw_np"),
    ("Av1FrameHeaderLight", "raw_np"),
])
def test_class_has_numpy_accessor(cls_name, attr):
    import tstrans.codec as c
    cls = getattr(c, cls_name)
    assert hasattr(cls, attr), f"{cls_name} missing {attr}"
