//! ADTS field decode tables.
//!
//! Spec: ISO/IEC 13818-7 Annex 1.A (ADTS framing) + ISO/IEC 14496-3
//! Tables 1.16 (sampling frequency) and 1.19 (channel configuration).

use crate::codec::ParseError;

/// Sampling frequency table per ISO 14496-3 Table 1.16.
/// Index 13/14 are reserved; index 15 is "explicit" (not meaningful in ADTS).
#[allow(dead_code)]
const SAMPLING_FREQUENCY: [u32; 13] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050,
    16000, 12000, 11025, 8000, 7350,
];

/// Decode `sampling_frequency_index` (4 bits) to sample rate (Hz).
/// Errors `ReservedValue` for indices 13/14/15.
#[allow(dead_code)]
pub(crate) fn decode_sample_rate(idx: u8) -> Result<u32, ParseError> {
    if idx >= 13 {
        return Err(ParseError::ReservedValue {
            field: "sampling_frequency_index",
            value: idx as u32,
        });
    }
    Ok(SAMPLING_FREQUENCY[idx as usize])
}

/// Decode `channel_configuration` (3 bits) to canonical channel count
/// per ISO 14496-3 Table 1.19. Index 0 = PCE-defined (we don't walk PCE).
#[allow(dead_code)]
pub(crate) fn decode_channels(channel_config: u8) -> Result<u8, ParseError> {
    match channel_config {
        0 => Err(ParseError::ReservedValue {
            field: "channel_configuration",
            value: 0,
        }),
        1 => Ok(1),
        2 => Ok(2),
        3 => Ok(3),
        4 => Ok(4),
        5 => Ok(5),
        6 => Ok(6),
        7 => Ok(8),
        _ => Err(ParseError::ReservedValue {
            field: "channel_configuration",
            value: channel_config as u32,
        }),
    }
}

/// Decode `profile` (2 bits, ADTS Annex 1.A) to typed profile.
#[allow(dead_code)]
pub(crate) fn decode_profile(profile: u8) -> super::AacProfile {
    match profile & 0b11 {
        0 => super::AacProfile::Main,
        1 => super::AacProfile::Lc,
        2 => super::AacProfile::Ssr,
        3 => super::AacProfile::LongTermPrediction,
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::aac::AacProfile;

    #[test]
    fn sample_rate_44100() {
        assert_eq!(decode_sample_rate(4).unwrap(), 44100);
    }
    #[test]
    fn sample_rate_index_13_reserved() {
        assert!(matches!(
            decode_sample_rate(13).unwrap_err(),
            ParseError::ReservedValue { field: "sampling_frequency_index", value: 13 }
        ));
    }
    #[test]
    fn channels_stereo_is_2() {
        assert_eq!(decode_channels(2).unwrap(), 2);
    }
    #[test]
    fn channels_7_1_is_8() {
        assert_eq!(decode_channels(7).unwrap(), 8);
    }
    #[test]
    fn channels_pce_defined_is_reserved() {
        assert!(matches!(
            decode_channels(0).unwrap_err(),
            ParseError::ReservedValue { field: "channel_configuration", value: 0 }
        ));
    }
    #[test]
    fn profile_lc() {
        assert_eq!(decode_profile(1), AacProfile::Lc);
    }
}
