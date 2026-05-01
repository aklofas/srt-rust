//! ST 0601 UAS Datalink Local Set typed layer.
//!
//! `UasDatalinkLs` is a flat plain struct that mirrors the wire format
//! directly. Composite types (`GeoPoint`, `Attitude`, `FieldOfView`,
//! `Corners`) are derived read-only views layered on top.
//!
//! Three decode entry points:
//! - [`decode`] — verifies checksum, accepts any UL.
//! - [`decode_unchecked`] — skips checksum verification (useful for known-
//!   broken-checksum captures), accepts any UL.
//! - [`decode_strict`] — verifies checksum + requires the ST 0601 family UL
//!   pattern.

pub(crate) mod mapping;
pub(crate) mod tags;

use crate::error::{KlvDecodeError, KlvEncodeError};
use crate::klv::pack::OwnedRawField;
use crate::klv::universal_label::UniversalLabel;
use crate::error::KlvFieldError;

#[derive(Debug, Clone, PartialEq)]
pub struct UasDatalinkLs {
    pub universal_label: UniversalLabel,
    pub declared_version: u8,

    // Identity
    pub mission_id: Option<String>,
    pub platform_tail_number: Option<String>,
    pub platform_designation: Option<String>,
    pub image_source_sensor: Option<String>,
    pub image_coordinate_system: Option<String>,
    pub platform_call_sign: Option<String>,
    pub uas_ls_version: Option<u8>,

    // Time
    pub timestamp_us: Option<u64>,

    // Platform state
    pub platform_heading_deg: Option<f64>,
    pub platform_pitch_deg: Option<f64>,
    pub platform_roll_deg: Option<f64>,
    pub platform_true_airspeed: Option<f64>,
    pub platform_indicated_airspeed: Option<f64>,

    // Sensor pose & position
    pub sensor_lat_deg: Option<f64>,
    pub sensor_lon_deg: Option<f64>,
    pub sensor_alt_m: Option<f64>,
    pub sensor_hfov_deg: Option<f64>,
    pub sensor_vfov_deg: Option<f64>,
    pub sensor_rel_az_deg: Option<f64>,
    pub sensor_rel_el_deg: Option<f64>,
    pub sensor_rel_roll_deg: Option<f64>,

    // Ranging & frame center
    pub slant_range_m: Option<f64>,
    pub target_width_m: Option<f64>,
    pub frame_center_lat_deg: Option<f64>,
    pub frame_center_lon_deg: Option<f64>,
    pub frame_center_elev_m: Option<f64>,

    // Image corners — offsets from frame center (tags 26-33)
    pub corner_lat_offset_p1_deg: Option<f64>,
    pub corner_lon_offset_p1_deg: Option<f64>,
    pub corner_lat_offset_p2_deg: Option<f64>,
    pub corner_lon_offset_p2_deg: Option<f64>,
    pub corner_lat_offset_p3_deg: Option<f64>,
    pub corner_lon_offset_p3_deg: Option<f64>,
    pub corner_lat_offset_p4_deg: Option<f64>,
    pub corner_lon_offset_p4_deg: Option<f64>,

    // Image corners — full lat/lon (tags 82-89, ST 0601.13+)
    pub corner_lat_p1_deg: Option<f64>,
    pub corner_lon_p1_deg: Option<f64>,
    pub corner_lat_p2_deg: Option<f64>,
    pub corner_lon_p2_deg: Option<f64>,
    pub corner_lat_p3_deg: Option<f64>,
    pub corner_lon_p3_deg: Option<f64>,
    pub corner_lat_p4_deg: Option<f64>,
    pub corner_lon_p4_deg: Option<f64>,

    // Misc
    pub generic_flag_data: Option<u8>,
    pub security_local_set: Option<Vec<u8>>,

    // Pass-through
    pub unknown: Vec<OwnedRawField>,
    pub field_errors: Vec<KlvFieldError>,
}

impl Default for UasDatalinkLs {
    fn default() -> Self {
        Self {
            universal_label: UniversalLabel::ST_0601_LS,
            declared_version: UniversalLabel::ST_0601_LS.version_byte(),
            mission_id: None,
            platform_tail_number: None,
            platform_designation: None,
            image_source_sensor: None,
            image_coordinate_system: None,
            platform_call_sign: None,
            uas_ls_version: None,
            timestamp_us: None,
            platform_heading_deg: None,
            platform_pitch_deg: None,
            platform_roll_deg: None,
            platform_true_airspeed: None,
            platform_indicated_airspeed: None,
            sensor_lat_deg: None,
            sensor_lon_deg: None,
            sensor_alt_m: None,
            sensor_hfov_deg: None,
            sensor_vfov_deg: None,
            sensor_rel_az_deg: None,
            sensor_rel_el_deg: None,
            sensor_rel_roll_deg: None,
            slant_range_m: None,
            target_width_m: None,
            frame_center_lat_deg: None,
            frame_center_lon_deg: None,
            frame_center_elev_m: None,
            corner_lat_offset_p1_deg: None,
            corner_lon_offset_p1_deg: None,
            corner_lat_offset_p2_deg: None,
            corner_lon_offset_p2_deg: None,
            corner_lat_offset_p3_deg: None,
            corner_lon_offset_p3_deg: None,
            corner_lat_offset_p4_deg: None,
            corner_lon_offset_p4_deg: None,
            corner_lat_p1_deg: None,
            corner_lon_p1_deg: None,
            corner_lat_p2_deg: None,
            corner_lon_p2_deg: None,
            corner_lat_p3_deg: None,
            corner_lon_p3_deg: None,
            corner_lat_p4_deg: None,
            corner_lon_p4_deg: None,
            generic_flag_data: None,
            security_local_set: None,
            unknown: Vec::new(),
            field_errors: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EncodeOptions {
    pub universal_label: UniversalLabel,
    /// Version byte to use for Tag 65 if the struct's `uas_ls_version` is None.
    pub version: u8,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            universal_label: UniversalLabel::ST_0601_LS,
            version: UniversalLabel::ST_0601_LS.version_byte(),
        }
    }
}

// ============================================================================
// Composite types
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoPoint {
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub alt_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Attitude {
    pub heading_deg: f64,
    pub pitch_deg: f64,
    pub roll_deg: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldOfView {
    pub horizontal_deg: f64,
    pub vertical_deg: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corners {
    /// (lat, lon); upper-left looking forward.
    pub p1: (f64, f64),
    pub p2: (f64, f64),
    pub p3: (f64, f64),
    pub p4: (f64, f64),
}

impl UasDatalinkLs {
    pub fn sensor_position(&self) -> Option<GeoPoint> {
        Some(GeoPoint {
            lat_deg: self.sensor_lat_deg?,
            lon_deg: self.sensor_lon_deg?,
            alt_m: self.sensor_alt_m?,
        })
    }

    pub fn sensor_attitude(&self) -> Option<Attitude> {
        Some(Attitude {
            heading_deg: self.sensor_rel_az_deg?,
            pitch_deg: self.sensor_rel_el_deg?,
            roll_deg: self.sensor_rel_roll_deg?,
        })
    }

    pub fn sensor_fov(&self) -> Option<FieldOfView> {
        Some(FieldOfView {
            horizontal_deg: self.sensor_hfov_deg?,
            vertical_deg: self.sensor_vfov_deg?,
        })
    }

    pub fn platform_attitude(&self) -> Option<Attitude> {
        Some(Attitude {
            heading_deg: self.platform_heading_deg?,
            pitch_deg: self.platform_pitch_deg?,
            roll_deg: self.platform_roll_deg?,
        })
    }

    pub fn frame_center(&self) -> Option<GeoPoint> {
        Some(GeoPoint {
            lat_deg: self.frame_center_lat_deg?,
            lon_deg: self.frame_center_lon_deg?,
            alt_m: self.frame_center_elev_m?,
        })
    }

    pub fn corners(&self) -> Option<Corners> {
        // Prefer absolute (tags 82-89) when fully populated.
        if let (Some(l1), Some(o1), Some(l2), Some(o2), Some(l3), Some(o3), Some(l4), Some(o4)) = (
            self.corner_lat_p1_deg,
            self.corner_lon_p1_deg,
            self.corner_lat_p2_deg,
            self.corner_lon_p2_deg,
            self.corner_lat_p3_deg,
            self.corner_lon_p3_deg,
            self.corner_lat_p4_deg,
            self.corner_lon_p4_deg,
        ) {
            return Some(Corners {
                p1: (l1, o1),
                p2: (l2, o2),
                p3: (l3, o3),
                p4: (l4, o4),
            });
        }
        // Fall back to offsets + frame center.
        let lat0 = self.frame_center_lat_deg?;
        let lon0 = self.frame_center_lon_deg?;
        let (dl1, do1) = (self.corner_lat_offset_p1_deg?, self.corner_lon_offset_p1_deg?);
        let (dl2, do2) = (self.corner_lat_offset_p2_deg?, self.corner_lon_offset_p2_deg?);
        let (dl3, do3) = (self.corner_lat_offset_p3_deg?, self.corner_lon_offset_p3_deg?);
        let (dl4, do4) = (self.corner_lat_offset_p4_deg?, self.corner_lon_offset_p4_deg?);
        Some(Corners {
            p1: (lat0 + dl1, lon0 + do1),
            p2: (lat0 + dl2, lon0 + do2),
            p3: (lat0 + dl3, lon0 + do3),
            p4: (lat0 + dl4, lon0 + do4),
        })
    }
}

// ============================================================================
// Encode / decode — stubs filled in Tasks 12 and 13
// ============================================================================

pub fn encode(_record: &UasDatalinkLs, _out: &mut [u8]) -> Result<usize, KlvEncodeError> {
    Err(KlvEncodeError::RecordTooLarge) // placeholder; replaced in Task 12
}

pub fn encode_with(
    _record: &UasDatalinkLs,
    _opts: &EncodeOptions,
    _out: &mut [u8],
) -> Result<usize, KlvEncodeError> {
    Err(KlvEncodeError::RecordTooLarge) // placeholder; replaced in Task 12
}

pub fn encode_to_vec(record: &UasDatalinkLs) -> Result<Vec<u8>, KlvEncodeError> {
    let n = encoded_len(record);
    let mut buf = vec![0u8; n];
    let written = encode(record, &mut buf)?;
    buf.truncate(written);
    Ok(buf)
}

pub fn encoded_len(_record: &UasDatalinkLs) -> usize {
    0 // placeholder; replaced in Task 12
}

pub fn encoded_len_with(_record: &UasDatalinkLs, _opts: &EncodeOptions) -> usize {
    0 // placeholder; replaced in Task 12
}

pub fn decode(_buf: &[u8]) -> Result<UasDatalinkLs, KlvDecodeError> {
    Err(KlvDecodeError::Truncated {
        offset: 0,
        needed: 0,
        have: 0,
    }) // placeholder; replaced in Task 13
}

pub fn decode_unchecked(_buf: &[u8]) -> Result<UasDatalinkLs, KlvDecodeError> {
    Err(KlvDecodeError::Truncated {
        offset: 0,
        needed: 0,
        have: 0,
    }) // placeholder; replaced in Task 13
}

pub fn decode_strict(_buf: &[u8]) -> Result<UasDatalinkLs, KlvDecodeError> {
    Err(KlvDecodeError::Truncated {
        offset: 0,
        needed: 0,
        have: 0,
    }) // placeholder; replaced in Task 13
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_uses_st0601_ul() {
        let r = UasDatalinkLs::default();
        assert_eq!(r.universal_label, UniversalLabel::ST_0601_LS);
        assert_eq!(r.declared_version, 0x13);
    }

    #[test]
    fn sensor_position_requires_all_three() {
        let mut r = UasDatalinkLs::default();
        assert!(r.sensor_position().is_none());
        r.sensor_lat_deg = Some(45.0);
        r.sensor_lon_deg = Some(-122.0);
        assert!(r.sensor_position().is_none(), "alt missing");
        r.sensor_alt_m = Some(1500.0);
        let p = r.sensor_position().unwrap();
        assert_eq!(p.lat_deg, 45.0);
        assert_eq!(p.lon_deg, -122.0);
        assert_eq!(p.alt_m, 1500.0);
    }

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn corners_prefer_full() {
        let mut r = UasDatalinkLs::default();
        // Set both forms with different values; full should win.
        r.frame_center_lat_deg = Some(0.0);
        r.frame_center_lon_deg = Some(0.0);
        r.corner_lat_offset_p1_deg = Some(0.01);
        r.corner_lon_offset_p1_deg = Some(0.01);
        r.corner_lat_offset_p2_deg = Some(0.01);
        r.corner_lon_offset_p2_deg = Some(-0.01);
        r.corner_lat_offset_p3_deg = Some(-0.01);
        r.corner_lon_offset_p3_deg = Some(-0.01);
        r.corner_lat_offset_p4_deg = Some(-0.01);
        r.corner_lon_offset_p4_deg = Some(0.01);
        r.corner_lat_p1_deg = Some(45.0);
        r.corner_lon_p1_deg = Some(-122.0);
        r.corner_lat_p2_deg = Some(45.0);
        r.corner_lon_p2_deg = Some(-121.0);
        r.corner_lat_p3_deg = Some(44.0);
        r.corner_lon_p3_deg = Some(-121.0);
        r.corner_lat_p4_deg = Some(44.0);
        r.corner_lon_p4_deg = Some(-122.0);
        let c = r.corners().unwrap();
        assert_eq!(c.p1, (45.0, -122.0));
        assert_eq!(c.p3, (44.0, -121.0));
    }

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn corners_fall_back_to_offsets() {
        let mut r = UasDatalinkLs::default();
        r.frame_center_lat_deg = Some(45.0);
        r.frame_center_lon_deg = Some(-122.0);
        r.corner_lat_offset_p1_deg = Some(0.01);
        r.corner_lon_offset_p1_deg = Some(0.01);
        r.corner_lat_offset_p2_deg = Some(0.01);
        r.corner_lon_offset_p2_deg = Some(-0.01);
        r.corner_lat_offset_p3_deg = Some(-0.01);
        r.corner_lon_offset_p3_deg = Some(-0.01);
        r.corner_lat_offset_p4_deg = Some(-0.01);
        r.corner_lon_offset_p4_deg = Some(0.01);
        let c = r.corners().unwrap();
        assert!((c.p1.0 - 45.01).abs() < 1e-9);
        assert!((c.p1.1 - -121.99).abs() < 1e-9);
    }

    #[test]
    fn corners_none_when_neither_form_complete() {
        let r = UasDatalinkLs::default();
        assert!(r.corners().is_none());
    }

    #[test]
    fn encode_options_default() {
        let opts = EncodeOptions::default();
        assert_eq!(opts.universal_label, UniversalLabel::ST_0601_LS);
        assert_eq!(opts.version, 0x13);
    }
}
