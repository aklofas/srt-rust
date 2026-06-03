"""NAL / OBU type number → spec name lookup tables.

References:
  - H.264 §Table 7-1
  - H.265 §Table 7-1
  - H.266 V4 §Table 5
  - AV1 §5.3.2 Table 5-1
"""

H264_NAL_NAMES = {
    0: "UNSPEC",
    1: "NON_IDR_SLICE",
    2: "SLICE_A",
    3: "SLICE_B",
    4: "SLICE_C",
    5: "IDR_SLICE",
    6: "SEI",
    7: "SPS",
    8: "PPS",
    9: "AUD",
    10: "END_OF_SEQUENCE",
    11: "END_OF_STREAM",
    12: "FILLER",
    13: "SPS_EXT",
    14: "PREFIX_NAL_UNIT",
    15: "SUBSET_SPS",
    16: "DEPTH_PARAMETER_SET",
    19: "AUX_SLICE_NO_PART",
    20: "SLICE_EXT",
    21: "SLICE_EXT_DEPTH",
}

H265_NAL_NAMES = {
    0: "TRAIL_N", 1: "TRAIL_R", 2: "TSA_N", 3: "TSA_R",
    4: "STSA_N", 5: "STSA_R", 6: "RADL_N", 7: "RADL_R",
    8: "RASL_N", 9: "RASL_R",
    16: "BLA_W_LP", 17: "BLA_W_RADL", 18: "BLA_N_LP",
    19: "IDR_W_RADL", 20: "IDR_N_LP", 21: "CRA_NUT",
    22: "RSV_IRAP_VCL22", 23: "RSV_IRAP_VCL23",
    32: "VPS_NUT", 33: "SPS_NUT", 34: "PPS_NUT",
    35: "AUD_NUT", 36: "EOS_NUT", 37: "EOB_NUT",
    38: "FD_NUT", 39: "PREFIX_SEI_NUT", 40: "SUFFIX_SEI_NUT",
}

H266_NAL_NAMES = {
    0: "TRAIL_NUT", 1: "STSA_NUT", 2: "RADL_NUT", 3: "RASL_NUT",
    4: "RSV_VCL_4", 5: "RSV_VCL_5", 6: "RSV_VCL_6",
    7: "IDR_W_RADL", 8: "IDR_N_LP", 9: "CRA_NUT", 10: "GDR_NUT",
    11: "RSV_IRAP_11",
    14: "VPS_NUT", 15: "SPS_NUT", 16: "PPS_NUT",
    17: "PREFIX_APS_NUT", 18: "SUFFIX_APS_NUT",
    19: "PH_NUT", 20: "AUD_NUT", 21: "EOS_NUT", 22: "EOB_NUT",
    23: "PREFIX_SEI_NUT", 24: "SUFFIX_SEI_NUT", 25: "FD_NUT",
    26: "RSV_NVCL_26", 27: "RSV_NVCL_27",
}

AV1_OBU_NAMES = {
    1: "SEQUENCE_HEADER",
    2: "TEMPORAL_DELIMITER",
    3: "FRAME_HEADER",
    4: "TILE_GROUP",
    5: "METADATA",
    6: "FRAME",
    7: "REDUNDANT_FRAME_HEADER",
    8: "TILE_LIST",
    15: "PADDING",
}


def nal_name(kind: str, nal_type: int) -> str:
    """Look up the spec name for a NAL type. Falls back to `unknown_<n>`."""
    table = {
        "H264": H264_NAL_NAMES,
        "H265": H265_NAL_NAMES,
        "H266": H266_NAL_NAMES,
    }.get(kind, {})
    return table.get(nal_type, f"unknown_{nal_type}")


def obu_name(obu_type: int) -> str:
    """Look up the AV1 OBU name. Falls back to `unknown_<n>`."""
    return AV1_OBU_NAMES.get(obu_type, f"unknown_{obu_type}")
