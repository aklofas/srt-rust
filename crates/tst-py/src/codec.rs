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
    Ok(())
}
