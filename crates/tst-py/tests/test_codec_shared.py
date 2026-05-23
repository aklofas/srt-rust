"""Phase 5: shared codec types + NalUnit + Obu typed access."""

import pytest

from tstrans import codec
from tstrans.codec import (
    ChromaFormat,
    ColorInfo,
    ColourPrimaries,
    MatrixCoefficients,
    NalUnit,
    Obu,
    ObuExtension,
    Rational,
    TransferCharacteristics,
)


def test_chroma_format_enum_variants():
    assert ChromaFormat.MONOCHROME
    assert ChromaFormat.YUV420
    assert ChromaFormat.YUV422
    assert ChromaFormat.YUV444


def test_rational_constructs_and_evaluates():
    r = Rational(num=24000, den=1001)
    assert r.num == 24000
    assert r.den == 1001
    assert pytest.approx(r.as_float(), rel=1e-6) == 24000 / 1001


def test_color_info_construct():
    ci = ColorInfo(
        primaries=ColourPrimaries.BT709,
        transfer=TransferCharacteristics.BT709,
        matrix=MatrixCoefficients.BT709,
        full_range=False,
    )
    assert ci.primaries == ColourPrimaries.BT709


def test_nal_unit_h264_construct_and_fields():
    nal = NalUnit.h264(nal_type=5, ref_idc=3, payload=b"\x01\x02\x03")
    assert nal.kind == "H264"
    assert nal.nal_type == 5
    assert nal.ref_idc == 3
    assert nal.payload == b"\x01\x02\x03"


def test_nal_unit_h265_construct():
    nal = NalUnit.h265(nal_type=19, layer_id=0, temporal_id_plus1=1, payload=b"\x42\x01")
    assert nal.kind == "H265"
    assert nal.nal_type == 19
    assert nal.layer_id == 0
    assert nal.temporal_id_plus1 == 1


def test_nal_unit_h266_construct():
    nal = NalUnit.h266(nal_type=7, layer_id=0, temporal_id_plus1=1, payload=b"\x00")
    assert nal.kind == "H266"


def test_obu_construct():
    obu = Obu(obu_type=6, extension=None, payload=b"\xab")
    assert obu.obu_type == 6
    assert obu.extension is None
    assert obu.payload == b"\xab"


def test_obu_with_extension():
    ext = ObuExtension(temporal_id=2, spatial_id=1)
    obu = Obu(obu_type=6, extension=ext, payload=b"")
    assert obu.extension.temporal_id == 2
    assert obu.extension.spatial_id == 1


def test_codec_module_exports():
    assert hasattr(codec, "NalUnit")
    assert hasattr(codec, "Obu")
    assert hasattr(codec, "ObuExtension")
