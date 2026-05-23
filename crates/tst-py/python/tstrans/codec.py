"""tstrans.codec — codec frame parsers.

Wraps `tst_core::codec::*`. Decode-only in v1; encoders / NumPy views
arrive in later phases per `docs/specs/2026-05-22-tst-py-design.md`.

Shared types (ChromaFormat, Rational, ColorInfo, colour primaries,
transfer characteristics, matrix coefficients) and typed NAL / OBU
wrappers land in Phase 5 Task 8. Per-codec classes and parsers for
H.264/H.265/H.266/AV1/AAC/MPEG-2 audio land in Tasks 9-14.
"""

from tstrans import _native as _native_mod

ChromaFormat = _native_mod.ChromaFormat
ColorInfo = _native_mod.ColorInfo
ColourPrimaries = _native_mod.ColourPrimaries
MatrixCoefficients = _native_mod.MatrixCoefficients
NalUnit = _native_mod.NalUnit
Obu = _native_mod.Obu
ObuExtension = _native_mod.ObuExtension
Rational = _native_mod.Rational
TransferCharacteristics = _native_mod.TransferCharacteristics

# H.264
EntropyCodingMode = _native_mod.EntropyCodingMode
H264ParameterSets = _native_mod.H264ParameterSets
H264Pps = _native_mod.H264Pps
H264SliceHeaderLight = _native_mod.H264SliceHeaderLight
H264SliceType = _native_mod.H264SliceType
H264Sps = _native_mod.H264Sps
parse_h264_parameter_sets = _native_mod.parse_h264_parameter_sets
parse_h264_pps = _native_mod.parse_h264_pps
parse_h264_slice_header_light = _native_mod.parse_h264_slice_header_light
parse_h264_sps = _native_mod.parse_h264_sps

# H.265
H265ParameterSets = _native_mod.H265ParameterSets
H265Pps = _native_mod.H265Pps
H265ProfileTierLevel = _native_mod.H265ProfileTierLevel
H265SliceHeaderLight = _native_mod.H265SliceHeaderLight
H265SliceType = _native_mod.H265SliceType
H265Sps = _native_mod.H265Sps
H265Vps = _native_mod.H265Vps
parse_h265_parameter_sets = _native_mod.parse_h265_parameter_sets
parse_h265_pps = _native_mod.parse_h265_pps
parse_h265_slice_header_light = _native_mod.parse_h265_slice_header_light
parse_h265_sps = _native_mod.parse_h265_sps
parse_h265_vps = _native_mod.parse_h265_vps

# H.266
H266ParameterSets = _native_mod.H266ParameterSets
H266Pps = _native_mod.H266Pps
H266ProfileTierLevel = _native_mod.H266ProfileTierLevel
H266SliceHeaderLight = _native_mod.H266SliceHeaderLight
H266SliceType = _native_mod.H266SliceType
H266Sps = _native_mod.H266Sps
H266Vps = _native_mod.H266Vps
parse_h266_parameter_sets = _native_mod.parse_h266_parameter_sets
parse_h266_pps = _native_mod.parse_h266_pps
parse_h266_slice_header_light = _native_mod.parse_h266_slice_header_light
parse_h266_sps = _native_mod.parse_h266_sps
parse_h266_vps = _native_mod.parse_h266_vps

__all__ = [
    "ChromaFormat",
    "ColorInfo",
    "ColourPrimaries",
    "MatrixCoefficients",
    "NalUnit",
    "Obu",
    "ObuExtension",
    "Rational",
    "TransferCharacteristics",
    # H.264
    "EntropyCodingMode",
    "H264ParameterSets",
    "H264Pps",
    "H264SliceHeaderLight",
    "H264SliceType",
    "H264Sps",
    "parse_h264_parameter_sets",
    "parse_h264_pps",
    "parse_h264_slice_header_light",
    "parse_h264_sps",
    # H.265
    "H265ParameterSets",
    "H265Pps",
    "H265ProfileTierLevel",
    "H265SliceHeaderLight",
    "H265SliceType",
    "H265Sps",
    "H265Vps",
    "parse_h265_parameter_sets",
    "parse_h265_pps",
    "parse_h265_slice_header_light",
    "parse_h265_sps",
    "parse_h265_vps",
    # H.266
    "H266ParameterSets",
    "H266Pps",
    "H266ProfileTierLevel",
    "H266SliceHeaderLight",
    "H266SliceType",
    "H266Sps",
    "H266Vps",
    "parse_h266_parameter_sets",
    "parse_h266_pps",
    "parse_h266_slice_header_light",
    "parse_h266_sps",
    "parse_h266_vps",
]
