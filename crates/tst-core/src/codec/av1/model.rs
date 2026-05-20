//! AV1 parsed type definitions.

use crate::codec::{ChromaFormat, CodecParseError, ColorInfo, Rational};

#[must_use]
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Av1SequenceHeader {
    pub profile: u8,
    /// `seq_level_idx[0]` — operating point 0 level index.
    pub level: u8,
    /// `seq_tier[0]` — operating point 0 tier (0 unless level > 7).
    pub tier: u8,
    /// `max_frame_width_minus_1 + 1`.
    pub max_frame_width: u32,
    /// `max_frame_height_minus_1 + 1`.
    pub max_frame_height: u32,
    /// 8, 10, or 12 per `BitDepth` derivation in §5.5.2.
    pub bit_depth: u8,
    pub monochrome: bool,
    pub chroma_format: ChromaFormat,
    pub still_picture: bool,
    pub reduced_still_picture_header: bool,
    /// Color metadata. AV1 always carries a `color_range` bit in the wire
    /// format (see §6.4.2), so a successful parse populates this with at
    /// least the dynamic-range signal. When `color_description_present_flag
    /// == 0`, `primaries`/`transfer`/`matrix` default to `Unspecified` per
    /// §5.5.2 and only `full_range` carries observed data. The `Option`
    /// wrapper is kept for `ColorInfo` parity with the H.26x parsers and for
    /// forward-compatibility with future error-recovery paths.
    pub color_info: Option<ColorInfo>,
    /// Frame rate derived from `time_scale / num_units_in_display_tick`,
    /// only populated when `timing_info_present_flag == 1` and
    /// `equal_picture_interval == 1`. Otherwise `None`.
    pub frame_rate: Option<Rational>,
    pub raw: Vec<u8>,
}

#[must_use]
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Av1FrameHeaderLight {
    /// `frame_type` per AV1 §5.9.1: 0=KEY_FRAME, 1=INTER_FRAME,
    /// 2=INTRA_ONLY_FRAME, 3=SWITCH_FRAME.
    pub frame_type: u8,
    pub show_frame: bool,
    pub show_existing_frame: bool,
    /// Per-frame size override. Current scope always returns `None` — the bit
    /// position of the override field depends on frame_type and
    /// frame_id_numbers_present_flag in ways we don't fully decode here.
    /// Consumers needing per-frame size should drive a full decoder.
    pub frame_size: Option<(u32, u32)>,
    pub raw: Vec<u8>,
}

#[must_use]
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Av1ObuStream {
    pub sequence_headers: Vec<Av1SequenceHeader>,
    pub frame_headers: Vec<Av1FrameHeaderLight>,
    /// `(obu_type, parse_error)` for each OBU we attempted but failed.
    /// Frame-header OBUs that arrive before a Sequence Header land
    /// here too with a synthesized "frame header before sequence header"
    /// engine error.
    pub unparseable: Vec<(u8, CodecParseError)>,
}
