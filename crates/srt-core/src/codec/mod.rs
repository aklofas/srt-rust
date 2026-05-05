//! Typed codec parameter-set parsers.
//!
//! Stateless parsers for video codec parameter sets (SPS / VPS / PPS).
//! Each codec lives in its own submodule with consistent function shape.
//! Consumers receive raw NAL units from [`crate::mpegts::demux`] and call
//! the parser explicitly when typed fields are needed.
//!
//! Shipped this slice: H.264 ([`h264`]) and H.265 ([`h265`]).
//! H.266 ([`h266`]) is scaffolded — per-set parsers are stubs that
//! return `ParseError::EngineError` until Tasks 8–11 of the AV1/H.266
//! plan land. AV1 ([`av1`]) is scaffolded — Sequence Header / Frame
//! Header parsers stub out until Tasks 23–25 land; the [`av1::leb128`]
//! primitive is live and used by `mpegts::demux` OBU framing today.
//! Future slices in the same umbrella: audio framing, subtitle
//! parsers — each will appear here as `codec::<name>`.

pub mod av1;
pub mod h264;
pub mod h265;
pub mod h266;

/// Chroma subsampling format. From H.264 / H.265 `chroma_format_idc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaFormat {
    /// 4:0:0 — luma only.
    Monochrome,
    /// 4:2:0 — half horizontal + half vertical chroma.
    Yuv420,
    /// 4:2:2 — half horizontal chroma only.
    Yuv422,
    /// 4:4:4 — full chroma.
    Yuv444,
}

/// Numerator/denominator pair (no implicit reduction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rational {
    pub num: u32,
    pub den: u32,
}

/// VUI / video signal type metadata. All fields decoded per
/// ITU-T H.273 / ISO/IEC 23091-2 (the codec-independent registry
/// referenced by both H.264 §E.2.1 and H.265 §E.2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorInfo {
    pub primaries: ColourPrimaries,
    pub transfer: TransferCharacteristics,
    pub matrix: MatrixCoefficients,
    /// `false` = limited range (16-235 for 8-bit luma); `true` = full range (0-255).
    pub full_range: bool,
    /// 0..=5 per H.264 §E.2.1 / H.265 §E.2.1; `None` if not signaled.
    pub chroma_loc: Option<u8>,
    /// Pixel aspect ratio. `None` when `aspect_ratio_idc == 0` (unspecified).
    pub sample_aspect_ratio: Option<Rational>,
}

/// ITU-T H.273 / ISO/IEC 23091-2 colour primaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourPrimaries {
    Bt709,
    Unspecified,
    Bt470M,
    Bt470Bg,
    Smpte170M,
    Smpte240M,
    Film,
    Bt2020,
    SmpteSt428,
    SmpteSt431_2,
    SmpteSt432_1,
    Ebu3213E,
    /// Spec-reserved or registry-extension value; preserved verbatim.
    Reserved(u8),
}

impl ColourPrimaries {
    /// Decode from H.273 `colour_primaries` byte.
    pub fn from_h273(v: u8) -> Self {
        match v {
            1 => Self::Bt709,
            2 => Self::Unspecified,
            4 => Self::Bt470M,
            5 => Self::Bt470Bg,
            6 => Self::Smpte170M,
            7 => Self::Smpte240M,
            8 => Self::Film,
            9 => Self::Bt2020,
            10 => Self::SmpteSt428,
            11 => Self::SmpteSt431_2,
            12 => Self::SmpteSt432_1,
            22 => Self::Ebu3213E,
            other => Self::Reserved(other),
        }
    }
}

/// ITU-T H.273 / ISO/IEC 23091-2 transfer characteristics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferCharacteristics {
    Bt709,
    Unspecified,
    Gamma22,
    Gamma28,
    Smpte170M,
    Smpte240M,
    Linear,
    Log100,
    LogSqrt,
    Iec61966_2_4,
    Bt1361E,
    Iec61966_2_1,
    Bt2020Bits10,
    Bt2020Bits12,
    /// SMPTE ST 2084 — perceptual quantizer (HDR PQ).
    SmpteSt2084,
    SmpteSt428,
    /// ARIB STD-B67 — hybrid log-gamma (HDR HLG).
    AribStdB67,
    Reserved(u8),
}

impl TransferCharacteristics {
    pub fn from_h273(v: u8) -> Self {
        match v {
            1 => Self::Bt709,
            2 => Self::Unspecified,
            4 => Self::Gamma22,
            5 => Self::Gamma28,
            6 => Self::Smpte170M,
            7 => Self::Smpte240M,
            8 => Self::Linear,
            9 => Self::Log100,
            10 => Self::LogSqrt,
            11 => Self::Iec61966_2_4,
            12 => Self::Bt1361E,
            13 => Self::Iec61966_2_1,
            14 => Self::Bt2020Bits10,
            15 => Self::Bt2020Bits12,
            16 => Self::SmpteSt2084,
            17 => Self::SmpteSt428,
            18 => Self::AribStdB67,
            other => Self::Reserved(other),
        }
    }
}

/// ITU-T H.273 V4 (07/2024) §8.3 Table 4 — matrix coefficients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixCoefficients {
    Identity,
    Bt709,
    Unspecified,
    FccMc,
    Bt470Bg,
    Smpte170M,
    Smpte240M,
    YCgCo,
    Bt2020NonConstant,
    Bt2020Constant,
    SmpteSt2085,
    ChromaDerivedNonConstant,
    ChromaDerivedConstant,
    IctCp,
    /// `15`: IPT-C2 (SMPTE IPT-PQ-C2). Added in H.273 V4.
    IptC2,
    /// `16`: YCgCo-Re — YCgCo-R with even bit-depth offset. Added in H.273 V4.
    YCgCoRe,
    /// `17`: YCgCo-Ro — YCgCo-R with odd bit-depth offset. Added in H.273 V4.
    YCgCoRo,
    /// Codepoints 18..255 reserved per H.273 V4 Table 4. Preserved verbatim.
    Reserved(u8),
}

impl MatrixCoefficients {
    pub fn from_h273(v: u8) -> Self {
        match v {
            0 => Self::Identity,
            1 => Self::Bt709,
            2 => Self::Unspecified,
            4 => Self::FccMc,
            5 => Self::Bt470Bg,
            6 => Self::Smpte170M,
            7 => Self::Smpte240M,
            8 => Self::YCgCo,
            9 => Self::Bt2020NonConstant,
            10 => Self::Bt2020Constant,
            11 => Self::SmpteSt2085,
            12 => Self::ChromaDerivedNonConstant,
            13 => Self::ChromaDerivedConstant,
            14 => Self::IctCp,
            15 => Self::IptC2,
            16 => Self::YCgCoRe,
            17 => Self::YCgCoRo,
            other => Self::Reserved(other),
        }
    }
}

/// Errors returned by codec parameter-set parsers.
///
/// All variants are non-panicking: bitstream walks are bounded, Golomb
/// decoders are capped at 32 leading zeros, enum casts use try-from with
/// `Err → ReservedValue`. See [crate root](crate::codec) for partial-success
/// behavioral rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Bitstream cursor walked past end of input. `needed_bits` is the
    /// shortfall in bits at the position where parsing failed.
    TruncatedRbsp { offset_bits: u32, needed_bits: u32 },

    /// Malformed Exp-Golomb code (leading zeros run > 32, or missing
    /// terminator bit).
    InvalidGolomb { offset_bits: u32 },

    /// Field carries a spec-reserved value that affects framing or
    /// downstream parsing semantics. `field` is a `&'static str` for
    /// cheap formatting; consumers should not pattern-match on it.
    ReservedValue { field: &'static str, value: u32 },

    /// Profile we cannot VUI-extract for. Fires only for profiles whose
    /// downstream framing semantics differ enough to produce wrong
    /// fields. Most uncommon profiles are supported as opaque integers.
    UnsupportedProfile { profile_idc: u8 },

    /// `parse_pps` standalone references an SPS id that wasn't seen.
    /// Does not fire from `parse_parameter_sets` (which collects SPSes
    /// before resolving PPSes).
    DanglingSpsReference { sps_id: u8 },

    /// H.265 only: `parse_sps` standalone references a VPS id that
    /// wasn't seen. Does not fire from `parse_parameter_sets`.
    DanglingVpsReference { vps_id: u8 },

    /// Underlying engine returned an error that doesn't map cleanly to
    /// our enum. The string is for diagnostics only — consumers should
    /// not pattern-match on it.
    EngineError(String),

    /// LEB128-encoded value (AV1 OBU size, uvlc) walked past 8 bytes
    /// or had the continuation bit set on the 8th byte (per AV1 spec
    /// `Leb128()` algorithm).
    InvalidLeb128 { offset_bytes: u32 },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TruncatedRbsp {
                offset_bits,
                needed_bits,
            } => {
                write!(
                    f,
                    "truncated RBSP at bit {offset_bits} (needed {needed_bits} more bits)"
                )
            }
            Self::InvalidGolomb { offset_bits } => {
                write!(f, "invalid Exp-Golomb code at bit {offset_bits}")
            }
            Self::ReservedValue { field, value } => {
                write!(f, "reserved value {value} in field '{field}'")
            }
            Self::UnsupportedProfile { profile_idc } => {
                write!(f, "unsupported profile_idc {profile_idc}")
            }
            Self::DanglingSpsReference { sps_id } => {
                write!(
                    f,
                    "PPS references SPS id {sps_id} which was not in the input"
                )
            }
            Self::DanglingVpsReference { vps_id } => {
                write!(
                    f,
                    "SPS references VPS id {vps_id} which was not in the input"
                )
            }
            Self::EngineError(msg) => write!(f, "parser engine: {msg}"),
            Self::InvalidLeb128 { offset_bytes } => {
                write!(f, "invalid LEB128 at byte {offset_bytes}")
            }
        }
    }
}

impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chroma_format_is_copy_eq() {
        let a = ChromaFormat::Yuv420;
        let b = a;
        assert_eq!(a, b);
    }

    #[test]
    fn rational_basic_construction() {
        let r = Rational {
            num: 30000,
            den: 1001,
        };
        assert_eq!(r.num, 30000);
        assert_eq!(r.den, 1001);
    }

    #[test]
    fn colour_primaries_known_arms_round_trip() {
        assert_eq!(ColourPrimaries::from_h273(1), ColourPrimaries::Bt709);
        assert_eq!(ColourPrimaries::from_h273(9), ColourPrimaries::Bt2020);
        assert_eq!(ColourPrimaries::from_h273(2), ColourPrimaries::Unspecified);
    }

    #[test]
    fn colour_primaries_unknown_arm_preserves_byte() {
        assert_eq!(
            ColourPrimaries::from_h273(99),
            ColourPrimaries::Reserved(99)
        );
    }

    #[test]
    fn transfer_characteristics_pq_and_hlg() {
        assert_eq!(
            TransferCharacteristics::from_h273(16),
            TransferCharacteristics::SmpteSt2084
        );
        assert_eq!(
            TransferCharacteristics::from_h273(18),
            TransferCharacteristics::AribStdB67
        );
    }

    #[test]
    fn matrix_coefficients_bt2020() {
        assert_eq!(
            MatrixCoefficients::from_h273(9),
            MatrixCoefficients::Bt2020NonConstant
        );
        assert_eq!(
            MatrixCoefficients::from_h273(10),
            MatrixCoefficients::Bt2020Constant
        );
    }

    /// Per ITU-T H.273 V4 (07/2024) §8.3 Table 4 (PDF p.13), three matrix
    /// codepoints were added beyond what V3 covered:
    ///   15 — IPT-C2 (SMPTE IPT-PQ-C2)
    ///   16 — YCgCo-Re (YCgCo-R with even bit-depth offset)
    ///   17 — YCgCo-Ro (YCgCo-R with odd bit-depth offset)
    /// Codepoints 18-255 remain Reserved.
    #[test]
    fn matrix_coefficients_h273_v4_codepoints_15_16_17() {
        assert_eq!(MatrixCoefficients::from_h273(15), MatrixCoefficients::IptC2);
        assert_eq!(
            MatrixCoefficients::from_h273(16),
            MatrixCoefficients::YCgCoRe
        );
        assert_eq!(
            MatrixCoefficients::from_h273(17),
            MatrixCoefficients::YCgCoRo
        );
        // Boundary: 18 remains Reserved per V4 Table 4.
        assert_eq!(
            MatrixCoefficients::from_h273(18),
            MatrixCoefficients::Reserved(18)
        );
    }

    #[test]
    fn parse_error_displays_helpfully() {
        let e = ParseError::TruncatedRbsp {
            offset_bits: 80,
            needed_bits: 5,
        };
        let s = format!("{e}");
        assert!(s.contains("truncated"));
        assert!(s.contains("80"));
    }

    #[test]
    fn parse_error_is_std_error() {
        fn assert_error<E: std::error::Error>(_: &E) {}
        let e = ParseError::EngineError("test".into());
        assert_error(&e);
    }

    #[test]
    fn parse_error_reserved_value_carries_field_name() {
        let e = ParseError::ReservedValue {
            field: "chroma_format_idc",
            value: 4,
        };
        let s = format!("{e}");
        assert!(s.contains("chroma_format_idc"));
    }
}
