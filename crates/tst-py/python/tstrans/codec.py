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
]
