//! tst-py — codec module (Phase 5).
//!
//! All codec PyO3 wraps for H.264/H.265/H.266/AV1/AAC/MPEG-2 audio,
//! plus shared types (ChromaFormat, Rational, ColorInfo, primaries/transfer/
//! matrix enums), plus NalUnit / Obu / ObuExtension typed wrappers used by
//! `Sample.payload`.

// PyO3 0.22 + Rust 2024 edition: the #[pymethods] macro generates calls to
// internal unsafe functions inside unsafe fn bodies. The `unsafe_op_in_unsafe_fn`
// lint (now a warning in edition 2024) fires on macro-generated code; suppress
// here exactly as in the sibling modules (errors.rs, klv.rs, mpegts.rs, mux.rs).
#![allow(unsafe_op_in_unsafe_fn, clippy::useless_conversion)]

use pyo3::prelude::*;
use pyo3::types::PyBytes;

use tst_core::codec::{
    ChromaFormat as RustChromaFormat, ColorInfo as RustColorInfo,
    ColourPrimaries as RustColourPrimaries, MatrixCoefficients as RustMatrixCoefficients,
    Rational as RustRational, TransferCharacteristics as RustTransferCharacteristics,
};

// === Shared enums ===

/// Chroma subsampling format. Mirrors `tst_core::codec::ChromaFormat`.
#[pyclass(eq, eq_int, name = "ChromaFormat", module = "tstrans.codec")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaFormatPy {
    #[pyo3(name = "MONOCHROME")]
    Monochrome,
    #[pyo3(name = "YUV420")]
    Yuv420,
    #[pyo3(name = "YUV422")]
    Yuv422,
    #[pyo3(name = "YUV444")]
    Yuv444,
    /// Catch-all for spec-reserved / future extension values.
    #[pyo3(name = "INVALID")]
    Invalid,
}

impl From<RustChromaFormat> for ChromaFormatPy {
    fn from(v: RustChromaFormat) -> Self {
        match v {
            RustChromaFormat::Monochrome => Self::Monochrome,
            RustChromaFormat::Yuv420 => Self::Yuv420,
            RustChromaFormat::Yuv422 => Self::Yuv422,
            RustChromaFormat::Yuv444 => Self::Yuv444,
        }
    }
}

/// ITU-T H.273 V4 §8.1 Table 2 — colour primaries.
/// Mirrors `tst_core::codec::ColourPrimaries`.
/// `Reserved(u8)` is collapsed to `RESERVED` (raw value not preserved in v1).
#[pyclass(eq, eq_int, name = "ColourPrimaries", module = "tstrans.codec")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourPrimariesPy {
    #[pyo3(name = "BT709")]
    Bt709,
    #[pyo3(name = "UNSPECIFIED")]
    Unspecified,
    #[pyo3(name = "BT470M")]
    Bt470M,
    #[pyo3(name = "BT470BG")]
    Bt470Bg,
    #[pyo3(name = "SMPTE170M")]
    Smpte170M,
    #[pyo3(name = "SMPTE240M")]
    Smpte240M,
    #[pyo3(name = "FILM")]
    Film,
    #[pyo3(name = "BT2020")]
    Bt2020,
    #[pyo3(name = "SMPTE_ST428")]
    SmpteSt428,
    #[pyo3(name = "SMPTE_ST431_2")]
    SmpteSt431_2,
    #[pyo3(name = "SMPTE_ST432_1")]
    SmpteSt432_1,
    #[pyo3(name = "EBU3213E")]
    Ebu3213E,
    /// Spec-reserved or registry-extension value.
    #[pyo3(name = "RESERVED")]
    Reserved,
}

impl From<RustColourPrimaries> for ColourPrimariesPy {
    fn from(v: RustColourPrimaries) -> Self {
        match v {
            RustColourPrimaries::Bt709 => Self::Bt709,
            RustColourPrimaries::Unspecified => Self::Unspecified,
            RustColourPrimaries::Bt470M => Self::Bt470M,
            RustColourPrimaries::Bt470Bg => Self::Bt470Bg,
            RustColourPrimaries::Smpte170M => Self::Smpte170M,
            RustColourPrimaries::Smpte240M => Self::Smpte240M,
            RustColourPrimaries::Film => Self::Film,
            RustColourPrimaries::Bt2020 => Self::Bt2020,
            RustColourPrimaries::SmpteSt428 => Self::SmpteSt428,
            RustColourPrimaries::SmpteSt431_2 => Self::SmpteSt431_2,
            RustColourPrimaries::SmpteSt432_1 => Self::SmpteSt432_1,
            RustColourPrimaries::Ebu3213E => Self::Ebu3213E,
            // Reserved(u8) + any future #[non_exhaustive] variants.
            _ => Self::Reserved,
        }
    }
}

/// ITU-T H.273 V4 §8.2 Table 3 — transfer characteristics.
/// Mirrors `tst_core::codec::TransferCharacteristics`.
#[pyclass(eq, eq_int, name = "TransferCharacteristics", module = "tstrans.codec")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferCharacteristicsPy {
    #[pyo3(name = "BT709")]
    Bt709,
    #[pyo3(name = "UNSPECIFIED")]
    Unspecified,
    #[pyo3(name = "GAMMA22")]
    Gamma22,
    #[pyo3(name = "GAMMA28")]
    Gamma28,
    #[pyo3(name = "SMPTE170M")]
    Smpte170M,
    #[pyo3(name = "SMPTE240M")]
    Smpte240M,
    #[pyo3(name = "LINEAR")]
    Linear,
    #[pyo3(name = "LOG100")]
    Log100,
    #[pyo3(name = "LOG_SQRT")]
    LogSqrt,
    #[pyo3(name = "IEC61966_2_4")]
    Iec61966_2_4,
    #[pyo3(name = "BT1361E")]
    Bt1361E,
    #[pyo3(name = "IEC61966_2_1")]
    Iec61966_2_1,
    #[pyo3(name = "BT2020_BITS10")]
    Bt2020Bits10,
    #[pyo3(name = "BT2020_BITS12")]
    Bt2020Bits12,
    /// SMPTE ST 2084 — perceptual quantizer (HDR PQ).
    #[pyo3(name = "SMPTE_ST2084")]
    SmpteSt2084,
    #[pyo3(name = "SMPTE_ST428")]
    SmpteSt428,
    /// ARIB STD-B67 — hybrid log-gamma (HDR HLG).
    #[pyo3(name = "ARIB_STD_B67")]
    AribStdB67,
    /// Spec-reserved value.
    #[pyo3(name = "RESERVED")]
    Reserved,
}

impl From<RustTransferCharacteristics> for TransferCharacteristicsPy {
    fn from(v: RustTransferCharacteristics) -> Self {
        match v {
            RustTransferCharacteristics::Bt709 => Self::Bt709,
            RustTransferCharacteristics::Unspecified => Self::Unspecified,
            RustTransferCharacteristics::Gamma22 => Self::Gamma22,
            RustTransferCharacteristics::Gamma28 => Self::Gamma28,
            RustTransferCharacteristics::Smpte170M => Self::Smpte170M,
            RustTransferCharacteristics::Smpte240M => Self::Smpte240M,
            RustTransferCharacteristics::Linear => Self::Linear,
            RustTransferCharacteristics::Log100 => Self::Log100,
            RustTransferCharacteristics::LogSqrt => Self::LogSqrt,
            RustTransferCharacteristics::Iec61966_2_4 => Self::Iec61966_2_4,
            RustTransferCharacteristics::Bt1361E => Self::Bt1361E,
            RustTransferCharacteristics::Iec61966_2_1 => Self::Iec61966_2_1,
            RustTransferCharacteristics::Bt2020Bits10 => Self::Bt2020Bits10,
            RustTransferCharacteristics::Bt2020Bits12 => Self::Bt2020Bits12,
            RustTransferCharacteristics::SmpteSt2084 => Self::SmpteSt2084,
            RustTransferCharacteristics::SmpteSt428 => Self::SmpteSt428,
            RustTransferCharacteristics::AribStdB67 => Self::AribStdB67,
            // Reserved(u8) + any future #[non_exhaustive] variants.
            _ => Self::Reserved,
        }
    }
}

/// ITU-T H.273 V4 §8.3 Table 4 — matrix coefficients.
/// Mirrors `tst_core::codec::MatrixCoefficients`.
#[pyclass(eq, eq_int, name = "MatrixCoefficients", module = "tstrans.codec")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixCoefficientsPy {
    #[pyo3(name = "IDENTITY")]
    Identity,
    #[pyo3(name = "BT709")]
    Bt709,
    #[pyo3(name = "UNSPECIFIED")]
    Unspecified,
    #[pyo3(name = "FCC_MC")]
    FccMc,
    #[pyo3(name = "BT470BG")]
    Bt470Bg,
    #[pyo3(name = "SMPTE170M")]
    Smpte170M,
    #[pyo3(name = "SMPTE240M")]
    Smpte240M,
    #[pyo3(name = "YCGCO")]
    YCgCo,
    #[pyo3(name = "BT2020_NON_CONSTANT")]
    Bt2020NonConstant,
    #[pyo3(name = "BT2020_CONSTANT")]
    Bt2020Constant,
    #[pyo3(name = "SMPTE_ST2085")]
    SmpteSt2085,
    #[pyo3(name = "CHROMA_DERIVED_NON_CONSTANT")]
    ChromaDerivedNonConstant,
    #[pyo3(name = "CHROMA_DERIVED_CONSTANT")]
    ChromaDerivedConstant,
    #[pyo3(name = "ICTCP")]
    IctCp,
    /// IPT-C2 (SMPTE IPT-PQ-C2). Added in H.273 V4.
    #[pyo3(name = "IPT_C2")]
    IptC2,
    /// YCgCo-Re — YCgCo-R with even bit-depth offset. Added in H.273 V4.
    #[pyo3(name = "YCGCO_RE")]
    YCgCoRe,
    /// YCgCo-Ro — YCgCo-R with odd bit-depth offset. Added in H.273 V4.
    #[pyo3(name = "YCGCO_RO")]
    YCgCoRo,
    /// Spec-reserved value.
    #[pyo3(name = "RESERVED")]
    Reserved,
}

impl From<RustMatrixCoefficients> for MatrixCoefficientsPy {
    fn from(v: RustMatrixCoefficients) -> Self {
        match v {
            RustMatrixCoefficients::Identity => Self::Identity,
            RustMatrixCoefficients::Bt709 => Self::Bt709,
            RustMatrixCoefficients::Unspecified => Self::Unspecified,
            RustMatrixCoefficients::FccMc => Self::FccMc,
            RustMatrixCoefficients::Bt470Bg => Self::Bt470Bg,
            RustMatrixCoefficients::Smpte170M => Self::Smpte170M,
            RustMatrixCoefficients::Smpte240M => Self::Smpte240M,
            RustMatrixCoefficients::YCgCo => Self::YCgCo,
            RustMatrixCoefficients::Bt2020NonConstant => Self::Bt2020NonConstant,
            RustMatrixCoefficients::Bt2020Constant => Self::Bt2020Constant,
            RustMatrixCoefficients::SmpteSt2085 => Self::SmpteSt2085,
            RustMatrixCoefficients::ChromaDerivedNonConstant => Self::ChromaDerivedNonConstant,
            RustMatrixCoefficients::ChromaDerivedConstant => Self::ChromaDerivedConstant,
            RustMatrixCoefficients::IctCp => Self::IctCp,
            RustMatrixCoefficients::IptC2 => Self::IptC2,
            RustMatrixCoefficients::YCgCoRe => Self::YCgCoRe,
            RustMatrixCoefficients::YCgCoRo => Self::YCgCoRo,
            // Reserved(u8) + any future #[non_exhaustive] variants.
            _ => Self::Reserved,
        }
    }
}

// === Rational + ColorInfo ===

/// Numerator/denominator pair. Mirrors `tst_core::codec::Rational`.
#[pyclass(name = "Rational", module = "tstrans.codec", frozen)]
#[derive(Debug, Clone, Copy)]
pub struct RationalPy {
    pub num: u32,
    pub den: u32,
}

#[pymethods]
impl RationalPy {
    #[new]
    fn new(num: u32, den: u32) -> Self {
        Self { num, den }
    }

    #[getter]
    fn num(&self) -> u32 {
        self.num
    }

    #[getter]
    fn den(&self) -> u32 {
        self.den
    }

    /// Return `num / den` as a floating-point value.
    fn as_float(&self) -> f64 {
        self.num as f64 / self.den as f64
    }

    fn __repr__(&self) -> String {
        format!("Rational(num={}, den={})", self.num, self.den)
    }
}

impl From<RustRational> for RationalPy {
    fn from(r: RustRational) -> Self {
        Self {
            num: r.num,
            den: r.den,
        }
    }
}

/// VUI / video signal type metadata. Mirrors `tst_core::codec::ColorInfo`.
///
/// All fields decoded per ITU-T H.273 / ISO/IEC 23091-2 (the codec-independent
/// registry referenced by both H.264 §E.2.1 and H.265 §E.2.1).
#[pyclass(name = "ColorInfo", module = "tstrans.codec", frozen)]
#[derive(Debug, Clone)]
pub struct ColorInfoPy {
    pub primaries: ColourPrimariesPy,
    pub transfer: TransferCharacteristicsPy,
    pub matrix: MatrixCoefficientsPy,
    pub full_range: bool,
}

#[pymethods]
impl ColorInfoPy {
    #[new]
    fn new(
        primaries: ColourPrimariesPy,
        transfer: TransferCharacteristicsPy,
        matrix: MatrixCoefficientsPy,
        full_range: bool,
    ) -> Self {
        Self {
            primaries,
            transfer,
            matrix,
            full_range,
        }
    }

    #[getter]
    fn primaries(&self) -> ColourPrimariesPy {
        self.primaries
    }

    #[getter]
    fn transfer(&self) -> TransferCharacteristicsPy {
        self.transfer
    }

    #[getter]
    fn matrix(&self) -> MatrixCoefficientsPy {
        self.matrix
    }

    #[getter]
    fn full_range(&self) -> bool {
        self.full_range
    }

    fn __repr__(&self) -> String {
        format!(
            "ColorInfo(primaries={:?}, transfer={:?}, matrix={:?}, full_range={})",
            self.primaries, self.transfer, self.matrix, self.full_range
        )
    }
}

impl From<RustColorInfo> for ColorInfoPy {
    fn from(c: RustColorInfo) -> Self {
        Self {
            primaries: c.primaries.into(),
            transfer: c.transfer.into(),
            matrix: c.matrix.into(),
            full_range: c.full_range,
        }
    }
}

// === NalUnit (tagged union) ===

/// One H.264 / H.265 / H.266 NAL unit. Tagged with `kind` so Python
/// callers can pattern-match on the discriminant without needing a Rust
/// enum hierarchy.
///
/// Construct via the codec-specific static methods:
/// - `NalUnit.h264(nal_type, ref_idc, payload)`
/// - `NalUnit.h265(nal_type, layer_id, temporal_id_plus1, payload)`
/// - `NalUnit.h266(nal_type, layer_id, temporal_id_plus1, payload)`
#[pyclass(name = "NalUnit", module = "tstrans.codec", frozen)]
#[derive(Debug, Clone)]
pub struct NalUnitPy {
    /// Codec discriminant — one of `"H264"`, `"H265"`, `"H266"`.
    pub kind: String,
    /// NAL unit type integer. Semantics depend on `kind`:
    /// H.264 §7.3.1 (5-bit), H.265 §7.3.1.2 (6-bit), H.266 §7.3.1.2 (5-bit).
    pub nal_type: u8,
    /// H.264 only — 2-bit `nal_ref_idc`. `None` for H.265 / H.266.
    pub ref_idc: Option<u8>,
    /// H.265 / H.266 only — `nuh_layer_id`. `None` for H.264.
    pub layer_id: Option<u8>,
    /// H.265 / H.266 only — `nuh_temporal_id_plus1`. `None` for H.264.
    pub temporal_id_plus1: Option<u8>,
    // RBSP payload bytes; exposed via getter returning PyBytes.
    pub payload: Vec<u8>,
}

#[pymethods]
impl NalUnitPy {
    /// Construct an H.264 NAL unit.
    ///
    /// `nal_type` is the 5-bit `nal_unit_type` from H.264 §7.3.1.
    /// `ref_idc` is the 2-bit `nal_ref_idc` from H.264 §7.3.1.
    /// `payload` carries RBSP bytes (Annex-B start codes stripped;
    /// emulation-prevention bytes preserved — consumer's decoder removes them).
    #[staticmethod]
    fn h264(nal_type: u8, ref_idc: u8, payload: Vec<u8>) -> Self {
        Self {
            kind: "H264".into(),
            nal_type,
            ref_idc: Some(ref_idc),
            layer_id: None,
            temporal_id_plus1: None,
            payload,
        }
    }

    /// Construct an H.265 NAL unit.
    ///
    /// `nal_type` is the 6-bit `nal_unit_type` from H.265 §7.3.1.2.
    /// `layer_id` is the 6-bit `nuh_layer_id`.
    /// `temporal_id_plus1` is the 3-bit `nuh_temporal_id_plus1`.
    #[staticmethod]
    fn h265(nal_type: u8, layer_id: u8, temporal_id_plus1: u8, payload: Vec<u8>) -> Self {
        Self {
            kind: "H265".into(),
            nal_type,
            ref_idc: None,
            layer_id: Some(layer_id),
            temporal_id_plus1: Some(temporal_id_plus1),
            payload,
        }
    }

    /// Construct an H.266 / VVC NAL unit.
    ///
    /// `nal_type` is the 5-bit `nal_unit_type` from H.266 V4 §7.3.1.2.
    /// `layer_id` is the 6-bit `nuh_layer_id`.
    /// `temporal_id_plus1` is the 3-bit `nuh_temporal_id_plus1`.
    #[staticmethod]
    fn h266(nal_type: u8, layer_id: u8, temporal_id_plus1: u8, payload: Vec<u8>) -> Self {
        Self {
            kind: "H266".into(),
            nal_type,
            ref_idc: None,
            layer_id: Some(layer_id),
            temporal_id_plus1: Some(temporal_id_plus1),
            payload,
        }
    }

    #[getter]
    fn kind(&self) -> &str {
        &self.kind
    }

    #[getter]
    fn nal_type(&self) -> u8 {
        self.nal_type
    }

    #[getter]
    fn ref_idc(&self) -> Option<u8> {
        self.ref_idc
    }

    #[getter]
    fn layer_id(&self) -> Option<u8> {
        self.layer_id
    }

    #[getter]
    fn temporal_id_plus1(&self) -> Option<u8> {
        self.temporal_id_plus1
    }

    /// RBSP payload bytes. Annex-B start codes stripped; emulation-prevention
    /// bytes preserved (consumer's decoder removes 0x03 escapes).
    #[getter]
    fn payload<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.payload)
    }

    fn __repr__(&self) -> String {
        format!(
            "NalUnit(kind='{}', nal_type={}, payload_len={})",
            self.kind,
            self.nal_type,
            self.payload.len()
        )
    }
}

// === Obu + ObuExtension ===

/// AV1 OBU extension header. Present when `obu_extension_flag = 1`.
/// Per AV1 Bitstream Spec §5.3.3.
#[pyclass(name = "ObuExtension", module = "tstrans.codec", frozen)]
#[derive(Debug, Clone, Copy)]
pub struct ObuExtensionPy {
    /// 3-bit `temporal_id` (AV1 §5.3.3).
    pub temporal_id: u8,
    /// 2-bit `spatial_id` (AV1 §5.3.3).
    pub spatial_id: u8,
}

#[pymethods]
impl ObuExtensionPy {
    #[new]
    fn new(temporal_id: u8, spatial_id: u8) -> Self {
        Self {
            temporal_id,
            spatial_id,
        }
    }

    #[getter]
    fn temporal_id(&self) -> u8 {
        self.temporal_id
    }

    #[getter]
    fn spatial_id(&self) -> u8 {
        self.spatial_id
    }

    fn __repr__(&self) -> String {
        format!(
            "ObuExtension(temporal_id={}, spatial_id={})",
            self.temporal_id, self.spatial_id
        )
    }
}

/// One AV1 Open Bitstream Unit. Per AV1 Bitstream Spec §5.3.2.
///
/// The OBU header byte, any extension byte, and the LEB128 `obu_size`
/// field are stripped during split; `payload` carries only the OBU body.
#[pyclass(name = "Obu", module = "tstrans.codec", frozen)]
#[derive(Debug, Clone)]
pub struct ObuPy {
    /// 4-bit `obu_type` (AV1 §5.3.2).
    /// Common values: 1=SequenceHeader, 2=TemporalDelimiter, 3=FrameHeader,
    /// 4=TileGroup, 5=Metadata, 6=Frame, 7=RedundantFrameHeader,
    /// 8=TileList, 15=Padding.
    pub obu_type: u8,
    /// Extension header, present when `obu_extension_flag = 1`.
    pub extension: Option<ObuExtensionPy>,
    // OBU body bytes; exposed via #[getter] returning PyBytes.
    pub payload: Vec<u8>,
}

#[pymethods]
impl ObuPy {
    #[new]
    #[pyo3(signature = (obu_type, extension, payload))]
    fn new(obu_type: u8, extension: Option<ObuExtensionPy>, payload: Vec<u8>) -> Self {
        Self {
            obu_type,
            extension,
            payload,
        }
    }

    #[getter]
    fn obu_type(&self) -> u8 {
        self.obu_type
    }

    #[getter]
    fn extension(&self) -> Option<ObuExtensionPy> {
        self.extension
    }

    /// OBU body bytes — header, extension byte, and LEB128 size field stripped.
    #[getter]
    fn payload<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.payload)
    }

    fn __repr__(&self) -> String {
        format!(
            "Obu(obu_type={}, payload_len={})",
            self.obu_type,
            self.payload.len()
        )
    }
}

// === H.264 ===

use tst_core::codec::h264::{
    EntropyCodingMode as RustEntropyCodingMode, H264ParameterSets as RustH264ParameterSets,
    H264Pps as RustH264Pps, H264SliceHeaderLight as RustH264SliceHeaderLight,
    H264SliceType as RustH264SliceType, H264Sps as RustH264Sps,
    parse_parameter_sets as rust_parse_h264_parameter_sets, parse_pps as rust_parse_h264_pps,
    parse_slice_header_light as rust_parse_h264_slice_header_light,
    parse_sps as rust_parse_h264_sps,
};

/// H.264 entropy coding mode signalled in the PPS.
/// Mirrors `tst_core::codec::h264::EntropyCodingMode`.
#[pyclass(eq, eq_int, name = "EntropyCodingMode", module = "tstrans.codec")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntropyCodingModePy {
    /// Context-Adaptive Variable Length Coding (Baseline/Main profiles).
    #[pyo3(name = "CAVLC")]
    Cavlc,
    /// Context-Adaptive Binary Arithmetic Coding (Main/High profiles).
    #[pyo3(name = "CABAC")]
    Cabac,
}

impl From<RustEntropyCodingMode> for EntropyCodingModePy {
    fn from(v: RustEntropyCodingMode) -> Self {
        match v {
            RustEntropyCodingMode::Cavlc => Self::Cavlc,
            RustEntropyCodingMode::Cabac => Self::Cabac,
        }
    }
}

/// H.264 slice type, normalised via `slice_type % 5` per H.264 §7.4.3.
/// Mirrors `tst_core::codec::h264::H264SliceType`.
#[pyclass(eq, eq_int, name = "H264SliceType", module = "tstrans.codec")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264SliceTypePy {
    /// P slice — predicted.
    #[pyo3(name = "P")]
    P,
    /// B slice — bidirectionally predicted.
    #[pyo3(name = "B")]
    B,
    /// I slice — intra-coded.
    #[pyo3(name = "I")]
    I,
    /// SP slice — switching P.
    #[pyo3(name = "Sp")]
    Sp,
    /// SI slice — switching I.
    #[pyo3(name = "Si")]
    Si,
    /// Unknown slice type — returned when the Rust parser produces a
    /// `#[non_exhaustive]` variant not yet mapped to a Python constant.
    #[pyo3(name = "Unknown")]
    Unknown,
}

impl From<RustH264SliceType> for H264SliceTypePy {
    fn from(v: RustH264SliceType) -> Self {
        match v {
            RustH264SliceType::P => Self::P,
            RustH264SliceType::B => Self::B,
            RustH264SliceType::I => Self::I,
            RustH264SliceType::Sp => Self::Sp,
            RustH264SliceType::Si => Self::Si,
            // #[non_exhaustive] catch-all — maps any future variant to Unknown
            // rather than mis-classifying it as intra (I) which would cause callers
            // to treat an unrecognised slice type as a keyframe indicator.
            _ => Self::Unknown,
        }
    }
}

/// Parsed H.264 Sequence Parameter Set.
/// Mirrors `tst_core::codec::h264::H264Sps`.
#[pyclass(name = "H264Sps", module = "tstrans.codec")]
#[derive(Debug, Clone)]
pub struct H264SpsPy {
    inner: RustH264Sps,
}

#[pymethods]
impl H264SpsPy {
    /// `seq_parameter_set_id` — identifies this SPS (H.264 §7.4.2.1.1).
    #[getter]
    fn seq_parameter_set_id(&self) -> u8 {
        self.inner.seq_parameter_set_id
    }

    /// Post-crop display width in luma samples.
    #[getter]
    fn width(&self) -> u32 {
        self.inner.width
    }

    /// Post-crop display height in luma samples.
    #[getter]
    fn height(&self) -> u32 {
        self.inner.height
    }

    /// `profile_idc` (66=Baseline, 77=Main, 100=High, …).
    #[getter]
    fn profile_idc(&self) -> u8 {
        self.inner.profile_idc
    }

    /// `level_idc` — e.g. 40 for Level 4.0.
    #[getter]
    fn level_idc(&self) -> u8 {
        self.inner.level_idc
    }

    /// `constraint_set_flags` byte (bits 7-2 = flags; bits 1-0 = reserved zero).
    #[getter]
    fn constraint_set_flags(&self) -> u8 {
        self.inner.constraint_set_flags
    }

    /// Luma bit depth (8 + `bit_depth_luma_minus8`).
    #[getter]
    fn bit_depth_luma(&self) -> u8 {
        self.inner.bit_depth_luma
    }

    /// Chroma bit depth (8 + `bit_depth_chroma_minus8`).
    #[getter]
    fn bit_depth_chroma(&self) -> u8 {
        self.inner.bit_depth_chroma
    }

    /// Chroma subsampling format.
    #[getter]
    fn chroma_format(&self) -> ChromaFormatPy {
        self.inner.chroma_format.into()
    }

    /// True for progressive encoding (`frame_mbs_only_flag=1`).
    #[getter]
    fn frame_mbs_only(&self) -> bool {
        self.inner.frame_mbs_only
    }

    /// True when `fixed_frame_rate_flag=1` in the VUI.
    #[getter]
    fn fixed_frame_rate(&self) -> bool {
        self.inner.fixed_frame_rate
    }

    /// True when the stream may contain B-frames (heuristic; see Rust docs).
    #[getter]
    fn has_b_frames(&self) -> bool {
        self.inner.has_b_frames
    }

    /// Frame rate as `Rational(num, den)`, or `None` when the VUI is absent.
    #[getter]
    fn frame_rate(&self) -> Option<RationalPy> {
        self.inner.frame_rate.map(Into::into)
    }

    /// VUI colour info, or `None` when the VUI is absent or video_signal_type
    /// is not present.
    #[getter]
    fn color(&self) -> Option<ColorInfoPy> {
        self.inner.color.clone().map(Into::into)
    }

    /// Left crop offset in luma samples (H.264 §6.4 after SubWidthC scaling).
    #[getter]
    fn crop_left(&self) -> u32 {
        self.inner.crop_left
    }

    /// Right crop offset in luma samples.
    #[getter]
    fn crop_right(&self) -> u32 {
        self.inner.crop_right
    }

    /// Top crop offset in luma samples.
    #[getter]
    fn crop_top(&self) -> u32 {
        self.inner.crop_top
    }

    /// Bottom crop offset in luma samples.
    #[getter]
    fn crop_bottom(&self) -> u32 {
        self.inner.crop_bottom
    }

    /// `log2_max_frame_num_minus4` — determines the bit width of `frame_num`
    /// in slice headers (`frame_num` width = this + 4).
    #[getter]
    fn log2_max_frame_num_minus4(&self) -> u8 {
        self.inner.log2_max_frame_num_minus4
    }

    /// Original RBSP bytes as supplied to `parse_h264_sps`.
    #[getter]
    fn raw_rbsp<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.raw_rbsp)
    }

    /// Coded picture width before `frame_crop` is applied (luma samples).
    /// Equal to `width + crop_left + crop_right`.
    fn coded_width(&self) -> u32 {
        self.inner.coded_width()
    }

    /// Coded picture height before `frame_crop` is applied (luma samples).
    /// Equal to `height + crop_top + crop_bottom`.
    fn coded_height(&self) -> u32 {
        self.inner.coded_height()
    }

    fn __repr__(&self) -> String {
        format!(
            "H264Sps(profile={}, level={}, {}x{}, sps_id={})",
            self.inner.profile_idc,
            self.inner.level_idc,
            self.inner.width,
            self.inner.height,
            self.inner.seq_parameter_set_id,
        )
    }
}

/// Parsed H.264 Picture Parameter Set.
/// Mirrors `tst_core::codec::h264::H264Pps`.
#[pyclass(name = "H264Pps", module = "tstrans.codec")]
#[derive(Debug, Clone)]
pub struct H264PpsPy {
    inner: RustH264Pps,
}

#[pymethods]
impl H264PpsPy {
    /// `pic_parameter_set_id` ∈ [0, 255].
    #[getter]
    fn pic_parameter_set_id(&self) -> u8 {
        self.inner.pic_parameter_set_id
    }

    /// `seq_parameter_set_id` — links this PPS to an SPS. ∈ [0, 31].
    #[getter]
    fn seq_parameter_set_id(&self) -> u8 {
        self.inner.seq_parameter_set_id
    }

    /// Entropy coding mode: `CAVLC` or `CABAC`.
    #[getter]
    fn entropy_coding_mode(&self) -> EntropyCodingModePy {
        self.inner.entropy_coding_mode.into()
    }

    /// Original RBSP bytes as supplied to `parse_h264_pps`.
    #[getter]
    fn raw_rbsp<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.raw_rbsp)
    }

    fn __repr__(&self) -> String {
        format!(
            "H264Pps(pps_id={}, sps_id={})",
            self.inner.pic_parameter_set_id, self.inner.seq_parameter_set_id
        )
    }
}

/// Light-weight H.264 slice header — fields required for keyframe detection
/// and frame-type classification.
/// Mirrors `tst_core::codec::h264::H264SliceHeaderLight`.
#[pyclass(name = "H264SliceHeaderLight", module = "tstrans.codec")]
#[derive(Debug, Clone)]
pub struct H264SliceHeaderLightPy {
    inner: RustH264SliceHeaderLight,
}

#[pymethods]
impl H264SliceHeaderLightPy {
    /// True when `first_mb_in_slice == 0` — start of a new frame.
    #[getter]
    fn first_in_pic(&self) -> bool {
        self.inner.first_in_pic
    }

    /// Slice type (normalised via `slice_type % 5` per H.264 §7.4.3).
    #[getter]
    fn slice_type(&self) -> H264SliceTypePy {
        self.inner.slice_type.into()
    }

    /// `pic_parameter_set_id` — links this slice to a PPS.
    #[getter]
    fn pps_id(&self) -> u8 {
        self.inner.pps_id
    }

    /// `frame_num` using the bit width from the referenced SPS, or `None`
    /// when no SPS context was passed to `parse_h264_slice_header_light`.
    #[getter]
    fn frame_num(&self) -> Option<u32> {
        self.inner.frame_num
    }

    /// True when `nal_unit_type == 5` (IDR slice).
    #[getter]
    fn idr(&self) -> bool {
        self.inner.idr
    }

    /// Original RBSP bytes as supplied to `parse_h264_slice_header_light`.
    #[getter]
    fn raw_rbsp<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.raw_rbsp)
    }

    fn __repr__(&self) -> String {
        format!(
            "H264SliceHeaderLight(first={}, slice_type={:?}, idr={})",
            self.inner.first_in_pic, self.inner.slice_type, self.inner.idr,
        )
    }
}

/// All SPS and PPS NAL units parsed from an access unit.
/// Mirrors `tst_core::codec::h264::H264ParameterSets`.
#[pyclass(name = "H264ParameterSets", module = "tstrans.codec")]
#[derive(Debug, Clone)]
pub struct H264ParameterSetsPy {
    inner: RustH264ParameterSets,
}

#[pymethods]
impl H264ParameterSetsPy {
    /// Mapping of `sps_id → H264Sps`. Keys are `int`, values are `H264Sps`.
    #[getter]
    fn sps_by_id<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let dict = pyo3::types::PyDict::new_bound(py);
        for (k, v) in &self.inner.sps_by_id {
            let sps_py = Py::new(py, H264SpsPy { inner: v.clone() })?;
            dict.set_item(*k, sps_py)?;
        }
        Ok(dict)
    }

    /// Mapping of `pps_id → H264Pps`. Keys are `int`, values are `H264Pps`.
    #[getter]
    fn pps_by_id<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let dict = pyo3::types::PyDict::new_bound(py);
        for (k, v) in &self.inner.pps_by_id {
            let pps_py = Py::new(py, H264PpsPy { inner: v.clone() })?;
            dict.set_item(*k, pps_py)?;
        }
        Ok(dict)
    }

    fn __repr__(&self) -> String {
        format!(
            "H264ParameterSets(n_sps={}, n_pps={})",
            self.inner.sps_by_id.len(),
            self.inner.pps_by_id.len()
        )
    }
}

/// Parse a single H.264 SPS RBSP.
///
/// `rbsp` must be the raw RBSP body — Annex B start code stripped, NAL header
/// byte stripped, emulation-prevention bytes preserved (matches
/// ``NalUnit.h264(...).payload``).
///
/// Raises `CodecError` with ``kind=CodecErrorKind.TRUNCATED_RBSP`` for empty
/// input; ``kind=CodecErrorKind.ENGINE_ERROR`` for unparseable bitstreams.
#[pyfunction]
#[pyo3(name = "parse_h264_sps")]
fn parse_h264_sps_py(py: Python<'_>, rbsp: &[u8]) -> PyResult<H264SpsPy> {
    rust_parse_h264_sps(rbsp)
        .map(|inner| H264SpsPy { inner })
        .map_err(|e| crate::errors::codec_parse_error_to_pyerr(py, &e, "h264"))
}

/// Parse a single H.264 PPS RBSP.
///
/// `rbsp` must be the raw RBSP body — same contract as `parse_h264_sps`.
///
/// Raises `CodecError` on parse failure.
#[pyfunction]
#[pyo3(name = "parse_h264_pps")]
fn parse_h264_pps_py(py: Python<'_>, rbsp: &[u8]) -> PyResult<H264PpsPy> {
    rust_parse_h264_pps(rbsp)
        .map(|inner| H264PpsPy { inner })
        .map_err(|e| crate::errors::codec_parse_error_to_pyerr(py, &e, "h264"))
}

/// Parse all H.264 SPS and PPS NAL units from a list of `NalUnit` objects.
///
/// Non-H.264 NAL units in the list are silently skipped. Non-SPS/PPS H.264
/// NAL units are also skipped. Parse is partial-success-tolerant: bad
/// individual parameter-set NALs emit a warning and are skipped.
///
/// Raises `CodecError` only when every parameter-set NAL in the input
/// failed to parse.
#[pyfunction]
#[pyo3(name = "parse_h264_parameter_sets")]
fn parse_h264_parameter_sets_py(
    py: Python<'_>,
    nals: Vec<PyRef<'_, NalUnitPy>>,
) -> PyResult<H264ParameterSetsPy> {
    use tst_core::mpegts::demux::event::NalUnit as RustNalUnit;
    // Convert each NalUnitPy to the Rust NalUnit::H264 variant.
    // Non-H.264 entries (H265, H266) are silently filtered out — the Rust
    // parse_parameter_sets fn already ignores non-H264 variants, but filtering
    // here avoids allocating dummy payloads for non-H264 discriminants.
    let rust_nals: Vec<RustNalUnit> = nals
        .iter()
        .filter_map(|n| {
            if n.kind != "H264" {
                return None;
            }
            Some(RustNalUnit::H264 {
                nal_type: n.nal_type,
                ref_idc: n.ref_idc.unwrap_or(3),
                payload: n.payload.clone(),
            })
        })
        .collect();
    rust_parse_h264_parameter_sets(&rust_nals)
        .map(|inner| H264ParameterSetsPy { inner })
        .map_err(|e| crate::errors::codec_parse_error_to_pyerr(py, &e, "h264"))
}

/// Parse a light H.264 slice header from a RBSP byte slice.
///
/// `rbsp` carries the RBSP body of a slice NAL — Annex B start code and NAL
/// header byte stripped, emulation-prevention bytes preserved.
///
/// `sps` is optional SPS context — when supplied, `frame_num` is read from
/// the bitstream using the bit width `log2_max_frame_num_minus4 + 4`. When
/// `None`, `H264SliceHeaderLight.frame_num` is `None`.
///
/// `nal_unit_type` is the 5-bit NAL type from the NAL header (`& 0x1F`) —
/// used to derive `H264SliceHeaderLight.idr` (`== 5`) without re-parsing.
///
/// Raises `CodecError` on parse failure.
#[pyfunction]
#[pyo3(name = "parse_h264_slice_header_light", signature = (rbsp, sps, nal_unit_type))]
fn parse_h264_slice_header_light_py(
    py: Python<'_>,
    rbsp: &[u8],
    sps: Option<&H264SpsPy>,
    nal_unit_type: u8,
) -> PyResult<H264SliceHeaderLightPy> {
    rust_parse_h264_slice_header_light(rbsp, sps.map(|s| &s.inner), nal_unit_type)
        .map(|inner| H264SliceHeaderLightPy { inner })
        .map_err(|e| crate::errors::codec_parse_error_to_pyerr(py, &e, "h264"))
}

// === H.265 ===

use tst_core::codec::h265::{
    H265ParameterSets as RustH265ParameterSets, H265Pps as RustH265Pps,
    H265SliceHeaderLight as RustH265SliceHeaderLight, H265SliceType as RustH265SliceType,
    H265Sps as RustH265Sps, H265Vps as RustH265Vps,
    parse_parameter_sets as rust_parse_h265_parameter_sets, parse_pps as rust_parse_h265_pps,
    parse_slice_header_light as rust_parse_h265_slice_header_light,
    parse_sps as rust_parse_h265_sps, parse_vps as rust_parse_h265_vps,
};

/// H.265 slice type (B / P / I). Only three values are defined by H.265
/// §7.4.7.1 Table 7-7.
/// Mirrors `tst_core::codec::h265::H265SliceType`.
#[pyclass(eq, eq_int, name = "H265SliceType", module = "tstrans.codec")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H265SliceTypePy {
    /// B slice — bidirectionally predicted.
    #[pyo3(name = "B")]
    B,
    /// P slice — predicted.
    #[pyo3(name = "P")]
    P,
    /// I slice — intra-coded.
    #[pyo3(name = "I")]
    I,
    /// Unknown slice type — returned when the Rust parser produces a
    /// `#[non_exhaustive]` variant not yet mapped to a Python constant.
    #[pyo3(name = "Unknown")]
    Unknown,
}

impl From<RustH265SliceType> for H265SliceTypePy {
    fn from(v: RustH265SliceType) -> Self {
        match v {
            RustH265SliceType::B => Self::B,
            RustH265SliceType::P => Self::P,
            RustH265SliceType::I => Self::I,
            // #[non_exhaustive] catch-all — maps any future variant to Unknown
            // rather than mis-classifying it as intra (I) which would cause callers
            // to treat an unrecognised slice type as a keyframe indicator.
            _ => Self::Unknown,
        }
    }
}

/// H.265 profile/tier/level fields parsed from a VPS or SPS NAL unit.
/// Mirrors the fields from `tst_core::codec::h265::H265ProfileTierLevel`.
///
/// These fields are decoded from the `profile_tier_level()` syntax structure
/// at H.265 §7.3.3. Both [`H265SpsPy`] and [`H265VpsPy`] expose the same
/// values via their `profile_tier_level()` method, which returns this object.
/// The fields are also available directly as getters on both SPS and VPS
/// classes for convenience.
///
/// Note: `general_profile_space` is always 0 when reconstructed from an SPS
/// or VPS object because `H265Sps` / `H265Vps` do not store that field
/// separately (it is always 0 for all ITU-T registered profiles).
#[pyclass(name = "H265ProfileTierLevel", module = "tstrans.codec")]
#[derive(Debug, Clone, Copy)]
pub struct H265ProfileTierLevelPy {
    general_profile_space: u8,
    general_tier_flag: bool,
    general_profile_idc: u8,
    general_profile_compatibility_flags: u32,
    general_progressive_source_flag: bool,
    general_interlaced_source_flag: bool,
    general_non_packed_constraint_flag: bool,
    general_frame_only_constraint_flag: bool,
    general_level_idc: u8,
}

#[pymethods]
impl H265ProfileTierLevelPy {
    /// 2-bit `general_profile_space` (§7.3.3). In practice always 0 for
    /// ITU-T registered profiles.
    #[getter]
    fn general_profile_space(&self) -> u8 {
        self.general_profile_space
    }

    /// `general_tier_flag` (§7.4.4): true = High tier, false = Main tier.
    #[getter]
    fn general_tier_flag(&self) -> bool {
        self.general_tier_flag
    }

    /// 5-bit `general_profile_idc` (§7.3.3). Common values: 1=Main,
    /// 2=Main10, 4=Rext, 5=HEVC-HM, 6=Multiview-Main, 7=Scalable-Main.
    #[getter]
    fn general_profile_idc(&self) -> u8 {
        self.general_profile_idc
    }

    /// 32-bit `general_profile_compatibility_flags` (§7.3.3). Bit `i` set
    /// means the stream conforms to profile `i`. MSB-first: spec-bit j lives
    /// at `flags & (1 << (31 - j))`. ffmpeg uses bit 2 (= 1 << 29) to
    /// disambiguate Main vs Main10 vs Main10-Intra.
    #[getter]
    fn general_profile_compatibility_flags(&self) -> u32 {
        self.general_profile_compatibility_flags
    }

    /// `general_progressive_source_flag` (§7.4.4): stream is progressive.
    #[getter]
    fn general_progressive_source_flag(&self) -> bool {
        self.general_progressive_source_flag
    }

    /// `general_interlaced_source_flag` (§7.4.4): stream is interlaced.
    #[getter]
    fn general_interlaced_source_flag(&self) -> bool {
        self.general_interlaced_source_flag
    }

    /// `general_non_packed_constraint_flag` (§7.4.4): no frame-packing
    /// arrangement SEI in the bitstream.
    #[getter]
    fn general_non_packed_constraint_flag(&self) -> bool {
        self.general_non_packed_constraint_flag
    }

    /// `general_frame_only_constraint_flag` (§7.4.4): stream contains only
    /// frames (no field pictures).
    #[getter]
    fn general_frame_only_constraint_flag(&self) -> bool {
        self.general_frame_only_constraint_flag
    }

    /// `general_level_idc` (§7.3.3). Level encoded as `30 * level_major +
    /// 3 * level_minor` for levels up to 6.2: e.g. 120 = Level 4.0,
    /// 150 = Level 5.0, 180 = Level 6.0.
    #[getter]
    fn general_level_idc(&self) -> u8 {
        self.general_level_idc
    }

    fn __repr__(&self) -> String {
        format!(
            "H265ProfileTierLevel(profile_idc={}, tier={}, level_idc={})",
            self.general_profile_idc, self.general_tier_flag, self.general_level_idc,
        )
    }
}

/// Construct an `H265ProfileTierLevelPy` from the fields stored on an
/// `H265Sps` or `H265Vps` (which both carry the PTL fields flattened).
/// `general_profile_space` is not stored on either of those types (it is
/// always 0 for all ITU-T registered profiles).
fn ptl_from_sps_fields(
    general_tier_flag: bool,
    general_profile_idc: u8,
    general_profile_compatibility_flags: u32,
    general_progressive_source_flag: bool,
    general_interlaced_source_flag: bool,
    general_non_packed_constraint_flag: bool,
    general_frame_only_constraint_flag: bool,
    general_level_idc: u8,
) -> H265ProfileTierLevelPy {
    H265ProfileTierLevelPy {
        general_profile_space: 0,
        general_tier_flag,
        general_profile_idc,
        general_profile_compatibility_flags,
        general_progressive_source_flag,
        general_interlaced_source_flag,
        general_non_packed_constraint_flag,
        general_frame_only_constraint_flag,
        general_level_idc,
    }
}

/// Parsed H.265 Sequence Parameter Set.
/// Mirrors `tst_core::codec::h265::H265Sps`.
#[pyclass(name = "H265Sps", module = "tstrans.codec")]
#[derive(Debug, Clone)]
pub struct H265SpsPy {
    inner: RustH265Sps,
}

#[pymethods]
impl H265SpsPy {
    /// `sps_seq_parameter_set_id` — identifies this SPS (H.265 §7.4.3.2.1).
    #[getter]
    fn sps_seq_parameter_set_id(&self) -> u8 {
        self.inner.sps_seq_parameter_set_id
    }

    /// `sps_video_parameter_set_id` — links this SPS to a VPS.
    #[getter]
    fn sps_video_parameter_set_id(&self) -> u8 {
        self.inner.sps_video_parameter_set_id
    }

    /// Post-crop display width in luma samples (after conformance window is
    /// applied).
    #[getter]
    fn width(&self) -> u32 {
        self.inner.width
    }

    /// Post-crop display height in luma samples.
    #[getter]
    fn height(&self) -> u32 {
        self.inner.height
    }

    /// `general_profile_idc` (1=Main, 2=Main10, …). Decoded from the
    /// `profile_tier_level()` block inside the SPS (§7.3.3).
    #[getter]
    fn general_profile_idc(&self) -> u8 {
        self.inner.general_profile_idc
    }

    /// `general_tier_flag` — true = High tier, false = Main tier.
    #[getter]
    fn general_tier_flag(&self) -> bool {
        self.inner.general_tier_flag
    }

    /// `general_level_idc` — e.g. 120 for Level 4.0, 150 for Level 5.0.
    #[getter]
    fn general_level_idc(&self) -> u8 {
        self.inner.general_level_idc
    }

    /// 32-bit `general_profile_compatibility_flags`. See
    /// [`H265ProfileTierLevel.general_profile_compatibility_flags`] for
    /// bit-ordering details.
    #[getter]
    fn general_profile_compatibility_flags(&self) -> u32 {
        self.inner.general_profile_compatibility_flags
    }

    /// `general_progressive_source_flag` (§7.4.4).
    #[getter]
    fn general_progressive_source_flag(&self) -> bool {
        self.inner.general_progressive_source_flag
    }

    /// `general_interlaced_source_flag` (§7.4.4).
    #[getter]
    fn general_interlaced_source_flag(&self) -> bool {
        self.inner.general_interlaced_source_flag
    }

    /// `general_non_packed_constraint_flag` (§7.4.4).
    #[getter]
    fn general_non_packed_constraint_flag(&self) -> bool {
        self.inner.general_non_packed_constraint_flag
    }

    /// `general_frame_only_constraint_flag` (§7.4.4).
    #[getter]
    fn general_frame_only_constraint_flag(&self) -> bool {
        self.inner.general_frame_only_constraint_flag
    }

    /// Luma bit depth (8 + `bit_depth_luma_minus8`).
    #[getter]
    fn bit_depth_luma(&self) -> u8 {
        self.inner.bit_depth_luma
    }

    /// Chroma bit depth (8 + `bit_depth_chroma_minus8`).
    #[getter]
    fn bit_depth_chroma(&self) -> u8 {
        self.inner.bit_depth_chroma
    }

    /// Chroma subsampling format.
    #[getter]
    fn chroma_format(&self) -> ChromaFormatPy {
        self.inner.chroma_format.into()
    }

    /// `sps_max_sub_layers_minus1` — max sub-layer temporal scalability.
    #[getter]
    fn max_sub_layers_minus1(&self) -> u8 {
        self.inner.max_sub_layers_minus1
    }

    /// Frame rate as `Rational(num, den)`, or `None` when VUI timing is absent.
    #[getter]
    fn frame_rate(&self) -> Option<RationalPy> {
        self.inner.frame_rate.map(Into::into)
    }

    /// VUI colour info, or `None` when VUI or `video_signal_type_present_flag`
    /// is absent.
    #[getter]
    fn color(&self) -> Option<ColorInfoPy> {
        self.inner.color.clone().map(Into::into)
    }

    /// Left crop offset in luma samples (after `SubWidthC` scaling).
    #[getter]
    fn crop_left(&self) -> u32 {
        self.inner.crop_left
    }

    /// Right crop offset in luma samples.
    #[getter]
    fn crop_right(&self) -> u32 {
        self.inner.crop_right
    }

    /// Top crop offset in luma samples.
    #[getter]
    fn crop_top(&self) -> u32 {
        self.inner.crop_top
    }

    /// Bottom crop offset in luma samples.
    #[getter]
    fn crop_bottom(&self) -> u32 {
        self.inner.crop_bottom
    }

    /// `log2_max_pic_order_cnt_lsb_minus4` (H.265 §7.4.3.2.1). The bit width
    /// of `pic_order_cnt_lsb` in slice headers equals this value plus 4.
    #[getter]
    fn log2_max_pic_order_cnt_lsb_minus4(&self) -> u8 {
        self.inner.log2_max_pic_order_cnt_lsb_minus4
    }

    /// Original RBSP bytes as supplied to `parse_h265_sps`.
    #[getter]
    fn raw_rbsp<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.raw_rbsp)
    }

    /// Reconstructed `H265ProfileTierLevel` from the fields decoded inside
    /// this SPS's `profile_tier_level()` block.
    fn profile_tier_level(&self) -> H265ProfileTierLevelPy {
        ptl_from_sps_fields(
            self.inner.general_tier_flag,
            self.inner.general_profile_idc,
            self.inner.general_profile_compatibility_flags,
            self.inner.general_progressive_source_flag,
            self.inner.general_interlaced_source_flag,
            self.inner.general_non_packed_constraint_flag,
            self.inner.general_frame_only_constraint_flag,
            self.inner.general_level_idc,
        )
    }

    /// Coded picture width before conformance-window crop is applied
    /// (luma samples). Equal to `width + crop_left + crop_right`.
    fn coded_width(&self) -> u32 {
        self.inner.coded_width()
    }

    /// Coded picture height before conformance-window crop is applied
    /// (luma samples). Equal to `height + crop_top + crop_bottom`.
    fn coded_height(&self) -> u32 {
        self.inner.coded_height()
    }

    fn __repr__(&self) -> String {
        format!(
            "H265Sps(profile={}, level={}, {}x{}, sps_id={})",
            self.inner.general_profile_idc,
            self.inner.general_level_idc,
            self.inner.width,
            self.inner.height,
            self.inner.sps_seq_parameter_set_id,
        )
    }
}

/// Parsed H.265 Picture Parameter Set.
/// Mirrors `tst_core::codec::h265::H265Pps`.
#[pyclass(name = "H265Pps", module = "tstrans.codec")]
#[derive(Debug, Clone)]
pub struct H265PpsPy {
    inner: RustH265Pps,
}

#[pymethods]
impl H265PpsPy {
    /// `pps_pic_parameter_set_id` ∈ [0, 63].
    #[getter]
    fn pps_pic_parameter_set_id(&self) -> u8 {
        self.inner.pps_pic_parameter_set_id
    }

    /// `pps_seq_parameter_set_id` — links this PPS to an SPS. ∈ [0, 15].
    #[getter]
    fn pps_seq_parameter_set_id(&self) -> u8 {
        self.inner.pps_seq_parameter_set_id
    }

    /// Original RBSP bytes as supplied to `parse_h265_pps`.
    #[getter]
    fn raw_rbsp<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.raw_rbsp)
    }

    fn __repr__(&self) -> String {
        format!(
            "H265Pps(pps_id={}, sps_id={})",
            self.inner.pps_pic_parameter_set_id, self.inner.pps_seq_parameter_set_id
        )
    }
}

/// Parsed H.265 Video Parameter Set.
/// Mirrors `tst_core::codec::h265::H265Vps`.
#[pyclass(name = "H265Vps", module = "tstrans.codec")]
#[derive(Debug, Clone)]
pub struct H265VpsPy {
    inner: RustH265Vps,
}

#[pymethods]
impl H265VpsPy {
    /// 4-bit `vps_video_parameter_set_id` — identifies this VPS.
    #[getter]
    fn vps_video_parameter_set_id(&self) -> u8 {
        self.inner.vps_video_parameter_set_id
    }

    /// `vps_max_layers_minus1` — max number of spatial layers (6-bit).
    #[getter]
    fn max_layers_minus1(&self) -> u8 {
        self.inner.max_layers_minus1
    }

    /// `vps_max_sub_layers_minus1` — max temporal sub-layer count (3-bit).
    #[getter]
    fn max_sub_layers_minus1(&self) -> u8 {
        self.inner.max_sub_layers_minus1
    }

    /// `vps_temporal_id_nesting_flag`: when true, temporal sub-layer
    /// `j < max_sub_layers_minus1` nests inside sub-layer `j+1`.
    #[getter]
    fn temporal_id_nesting_flag(&self) -> bool {
        self.inner.temporal_id_nesting_flag
    }

    /// `general_profile_idc` decoded from the VPS profile_tier_level block.
    #[getter]
    fn general_profile_idc(&self) -> u8 {
        self.inner.general_profile_idc
    }

    /// `general_tier_flag` decoded from the VPS profile_tier_level block.
    #[getter]
    fn general_tier_flag(&self) -> bool {
        self.inner.general_tier_flag
    }

    /// `general_level_idc` decoded from the VPS profile_tier_level block.
    #[getter]
    fn general_level_idc(&self) -> u8 {
        self.inner.general_level_idc
    }

    /// 32-bit `general_profile_compatibility_flags` from the VPS.
    #[getter]
    fn general_profile_compatibility_flags(&self) -> u32 {
        self.inner.general_profile_compatibility_flags
    }

    /// `general_progressive_source_flag` (§7.4.4).
    #[getter]
    fn general_progressive_source_flag(&self) -> bool {
        self.inner.general_progressive_source_flag
    }

    /// `general_interlaced_source_flag` (§7.4.4).
    #[getter]
    fn general_interlaced_source_flag(&self) -> bool {
        self.inner.general_interlaced_source_flag
    }

    /// `general_non_packed_constraint_flag` (§7.4.4).
    #[getter]
    fn general_non_packed_constraint_flag(&self) -> bool {
        self.inner.general_non_packed_constraint_flag
    }

    /// `general_frame_only_constraint_flag` (§7.4.4).
    #[getter]
    fn general_frame_only_constraint_flag(&self) -> bool {
        self.inner.general_frame_only_constraint_flag
    }

    /// Original RBSP bytes as supplied to `parse_h265_vps`.
    #[getter]
    fn raw_rbsp<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.raw_rbsp)
    }

    /// Reconstructed `H265ProfileTierLevel` from the fields decoded inside
    /// this VPS's `profile_tier_level()` block.
    fn profile_tier_level(&self) -> H265ProfileTierLevelPy {
        ptl_from_sps_fields(
            self.inner.general_tier_flag,
            self.inner.general_profile_idc,
            self.inner.general_profile_compatibility_flags,
            self.inner.general_progressive_source_flag,
            self.inner.general_interlaced_source_flag,
            self.inner.general_non_packed_constraint_flag,
            self.inner.general_frame_only_constraint_flag,
            self.inner.general_level_idc,
        )
    }

    fn __repr__(&self) -> String {
        format!(
            "H265Vps(vps_id={}, profile={}, level={})",
            self.inner.vps_video_parameter_set_id,
            self.inner.general_profile_idc,
            self.inner.general_level_idc,
        )
    }
}

/// Light-weight H.265 slice segment header — fields required for keyframe
/// detection and frame-type classification.
/// Mirrors `tst_core::codec::h265::H265SliceHeaderLight`.
#[pyclass(name = "H265SliceHeaderLight", module = "tstrans.codec")]
#[derive(Debug, Clone)]
pub struct H265SliceHeaderLightPy {
    inner: RustH265SliceHeaderLight,
}

#[pymethods]
impl H265SliceHeaderLightPy {
    /// True when `first_slice_segment_in_pic_flag == 1` — start of a new frame.
    #[getter]
    fn first_in_pic(&self) -> bool {
        self.inner.first_in_pic
    }

    /// Slice type (B / P / I / Unknown).
    #[getter]
    fn slice_type(&self) -> H265SliceTypePy {
        self.inner.slice_type.into()
    }

    /// `slice_pic_parameter_set_id` — links this slice to a PPS.
    #[getter]
    fn pps_id(&self) -> u8 {
        self.inner.pps_id
    }

    /// `pic_order_cnt_lsb` read using the bit width from the supplied SPS, or
    /// `None` when no SPS context was passed to `parse_h265_slice_header_light`.
    /// `Some(0)` for IDR slices (implicit per spec).
    #[getter]
    fn pic_order_cnt_lsb(&self) -> Option<u16> {
        self.inner.pic_order_cnt_lsb
    }

    /// True when `nal_unit_type` is IDR_W_RADL (19) or IDR_N_LP (20).
    #[getter]
    fn idr(&self) -> bool {
        self.inner.idr
    }

    /// Original RBSP bytes as supplied to `parse_h265_slice_header_light`.
    #[getter]
    fn raw_rbsp<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new_bound(py, &self.inner.raw_rbsp)
    }

    fn __repr__(&self) -> String {
        format!(
            "H265SliceHeaderLight(first={}, slice_type={:?}, idr={})",
            self.inner.first_in_pic, self.inner.slice_type, self.inner.idr,
        )
    }
}

/// All VPS, SPS, and PPS NAL units parsed from a slice.
/// Mirrors `tst_core::codec::h265::H265ParameterSets`.
#[pyclass(name = "H265ParameterSets", module = "tstrans.codec")]
#[derive(Debug, Clone)]
pub struct H265ParameterSetsPy {
    inner: RustH265ParameterSets,
}

#[pymethods]
impl H265ParameterSetsPy {
    /// Mapping of `vps_id → H265Vps`. Keys are `int`, values are `H265Vps`.
    #[getter]
    fn vps_by_id<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let dict = pyo3::types::PyDict::new_bound(py);
        for (k, v) in &self.inner.vps_by_id {
            let vps_py = Py::new(py, H265VpsPy { inner: v.clone() })?;
            dict.set_item(*k, vps_py)?;
        }
        Ok(dict)
    }

    /// Mapping of `sps_id → H265Sps`. Keys are `int`, values are `H265Sps`.
    #[getter]
    fn sps_by_id<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let dict = pyo3::types::PyDict::new_bound(py);
        for (k, v) in &self.inner.sps_by_id {
            let sps_py = Py::new(py, H265SpsPy { inner: v.clone() })?;
            dict.set_item(*k, sps_py)?;
        }
        Ok(dict)
    }

    /// Mapping of `pps_id → H265Pps`. Keys are `int`, values are `H265Pps`.
    #[getter]
    fn pps_by_id<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let dict = pyo3::types::PyDict::new_bound(py);
        for (k, v) in &self.inner.pps_by_id {
            let pps_py = Py::new(py, H265PpsPy { inner: v.clone() })?;
            dict.set_item(*k, pps_py)?;
        }
        Ok(dict)
    }

    fn __repr__(&self) -> String {
        format!(
            "H265ParameterSets(n_vps={}, n_sps={}, n_pps={})",
            self.inner.vps_by_id.len(),
            self.inner.sps_by_id.len(),
            self.inner.pps_by_id.len()
        )
    }
}

/// Parse a single H.265 SPS RBSP.
///
/// `rbsp` must be the raw RBSP body — Annex B start code stripped, NAL header
/// (2 bytes for H.265) stripped, emulation-prevention bytes preserved (matches
/// ``NalUnit.h265(...).payload``).
///
/// Raises `CodecError` with ``kind=CodecErrorKind.TRUNCATED_RBSP`` for empty
/// input; ``kind=CodecErrorKind.ENGINE_ERROR`` for unparseable bitstreams.
#[pyfunction]
#[pyo3(name = "parse_h265_sps")]
fn parse_h265_sps_py(py: Python<'_>, rbsp: &[u8]) -> PyResult<H265SpsPy> {
    rust_parse_h265_sps(rbsp)
        .map(|inner| H265SpsPy { inner })
        .map_err(|e| crate::errors::codec_parse_error_to_pyerr(py, &e, "h265"))
}

/// Parse a single H.265 PPS RBSP.
///
/// `rbsp` must be the raw RBSP body — same contract as `parse_h265_sps`.
///
/// Raises `CodecError` on parse failure.
#[pyfunction]
#[pyo3(name = "parse_h265_pps")]
fn parse_h265_pps_py(py: Python<'_>, rbsp: &[u8]) -> PyResult<H265PpsPy> {
    rust_parse_h265_pps(rbsp)
        .map(|inner| H265PpsPy { inner })
        .map_err(|e| crate::errors::codec_parse_error_to_pyerr(py, &e, "h265"))
}

/// Parse a single H.265 VPS RBSP.
///
/// `rbsp` must be the raw RBSP body — same contract as `parse_h265_sps`.
///
/// Raises `CodecError` on parse failure.
#[pyfunction]
#[pyo3(name = "parse_h265_vps")]
fn parse_h265_vps_py(py: Python<'_>, rbsp: &[u8]) -> PyResult<H265VpsPy> {
    rust_parse_h265_vps(rbsp)
        .map(|inner| H265VpsPy { inner })
        .map_err(|e| crate::errors::codec_parse_error_to_pyerr(py, &e, "h265"))
}

/// Parse all H.265 VPS, SPS, and PPS NAL units from a list of `NalUnit` objects.
///
/// Non-H.265 NAL units in the list are silently skipped. Parse is
/// partial-success-tolerant: bad individual parameter-set NALs emit a warning
/// and are skipped.
///
/// Raises `CodecError` only when every parameter-set NAL in the input
/// failed to parse.
#[pyfunction]
#[pyo3(name = "parse_h265_parameter_sets")]
fn parse_h265_parameter_sets_py(
    py: Python<'_>,
    nals: Vec<PyRef<'_, NalUnitPy>>,
) -> PyResult<H265ParameterSetsPy> {
    use tst_core::mpegts::demux::event::NalUnit as RustNalUnit;
    // Convert each NalUnitPy to the Rust NalUnit::H265 variant.
    // Non-H265 entries are silently filtered out.
    let rust_nals: Vec<RustNalUnit> = nals
        .iter()
        .filter_map(|n| {
            if n.kind != "H265" {
                return None;
            }
            Some(RustNalUnit::H265 {
                nal_type: n.nal_type,
                layer_id: n.layer_id.unwrap_or(0),
                temporal_id_plus1: n.temporal_id_plus1.unwrap_or(1),
                payload: n.payload.clone(),
            })
        })
        .collect();
    rust_parse_h265_parameter_sets(&rust_nals)
        .map(|inner| H265ParameterSetsPy { inner })
        .map_err(|e| crate::errors::codec_parse_error_to_pyerr(py, &e, "h265"))
}

/// Parse a light H.265 slice segment header from a RBSP byte slice.
///
/// `rbsp` carries the RBSP body of a slice NAL — Annex B start code and NAL
/// header (2 bytes) stripped, emulation-prevention bytes preserved.
///
/// `sps` is optional SPS context — when supplied, `pic_order_cnt_lsb` is
/// read from the bitstream using the bit width
/// `log2_max_pic_order_cnt_lsb_minus4 + 4`. When `None`,
/// `H265SliceHeaderLight.pic_order_cnt_lsb` is `None`.
///
/// `nal_unit_type` is the 6-bit NAL type from the NAL header
/// — used to derive `idr` (IDR_W_RADL=19 or IDR_N_LP=20) and to gate
/// IRAP-specific fields.
///
/// Raises `CodecError` on parse failure.
#[pyfunction]
#[pyo3(name = "parse_h265_slice_header_light", signature = (rbsp, sps, nal_unit_type))]
fn parse_h265_slice_header_light_py(
    py: Python<'_>,
    rbsp: &[u8],
    sps: Option<&H265SpsPy>,
    nal_unit_type: u8,
) -> PyResult<H265SliceHeaderLightPy> {
    rust_parse_h265_slice_header_light(rbsp, sps.map(|s| &s.inner), nal_unit_type)
        .map(|inner| H265SliceHeaderLightPy { inner })
        .map_err(|e| crate::errors::codec_parse_error_to_pyerr(py, &e, "h265"))
}

// === Module registration ===

/// Register all codec classes on `m` (`tstrans._native`).
///
/// Classes are added flat on the native extension module — matching the
/// pattern established by `mpegts::register` and `klv::register`. The
/// Python-side `tstrans.codec` module then re-imports them by name. Per-codec
/// classes added by Tasks 9-14 extend this same function.
pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Shared enums
    m.add_class::<ChromaFormatPy>()?;
    m.add_class::<ColourPrimariesPy>()?;
    m.add_class::<TransferCharacteristicsPy>()?;
    m.add_class::<MatrixCoefficientsPy>()?;
    // Shared structs
    m.add_class::<RationalPy>()?;
    m.add_class::<ColorInfoPy>()?;
    // Typed NAL / OBU wrappers (used on Sample.payload)
    m.add_class::<NalUnitPy>()?;
    m.add_class::<ObuExtensionPy>()?;
    m.add_class::<ObuPy>()?;
    // H.264 enums
    m.add_class::<EntropyCodingModePy>()?;
    m.add_class::<H264SliceTypePy>()?;
    // H.264 structs
    m.add_class::<H264SpsPy>()?;
    m.add_class::<H264PpsPy>()?;
    m.add_class::<H264SliceHeaderLightPy>()?;
    m.add_class::<H264ParameterSetsPy>()?;
    // H.264 parser functions
    m.add_function(wrap_pyfunction!(parse_h264_sps_py, m)?)?;
    m.add_function(wrap_pyfunction!(parse_h264_pps_py, m)?)?;
    m.add_function(wrap_pyfunction!(parse_h264_parameter_sets_py, m)?)?;
    m.add_function(wrap_pyfunction!(parse_h264_slice_header_light_py, m)?)?;
    // H.265 enums
    m.add_class::<H265SliceTypePy>()?;
    // H.265 structs
    m.add_class::<H265ProfileTierLevelPy>()?;
    m.add_class::<H265SpsPy>()?;
    m.add_class::<H265PpsPy>()?;
    m.add_class::<H265VpsPy>()?;
    m.add_class::<H265SliceHeaderLightPy>()?;
    m.add_class::<H265ParameterSetsPy>()?;
    // H.265 parser functions
    m.add_function(wrap_pyfunction!(parse_h265_sps_py, m)?)?;
    m.add_function(wrap_pyfunction!(parse_h265_pps_py, m)?)?;
    m.add_function(wrap_pyfunction!(parse_h265_vps_py, m)?)?;
    m.add_function(wrap_pyfunction!(parse_h265_parameter_sets_py, m)?)?;
    m.add_function(wrap_pyfunction!(parse_h265_slice_header_light_py, m)?)?;
    Ok(())
}
