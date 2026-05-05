//! H.266 VUI parsing (frame rate + color_info).
//!
//! Per H.266 V4 §7.3.2.5 (VUI parameters) and §E.2.1 (color description /
//! H.273 mapping). This is a stub — the SPS parser sets
//! `sps_vui_parameters_present_flag=0` paths to skip the call entirely
//! today, so the in-tree corpus never exercises this. Real-world streams
//! may set it; flesh out from §7.3.2.5 syntax when a consumer asks.

use crate::codec::h265::bitreader::BitReader;
use crate::codec::{ColorInfo, ParseError, Rational};

/// Stub — returns `(None, None)`. Real implementation per H.266 V4
/// §7.3.2.5 is deferred. See `docs/deferred-features.md` for trigger.
#[allow(dead_code)] // Wired in when VUI walk lands; today only kept-warm
// by an unused reference inside parse_sps.
pub(super) fn parse_h266_vui(
    _br: &mut BitReader<'_>,
    _payload_size_bytes: usize,
) -> Result<(Option<ColorInfo>, Option<Rational>), ParseError> {
    Ok((None, None))
}
