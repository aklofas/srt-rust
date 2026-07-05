//! ST 0601 typed model — the `UasDatalinkLs` flat struct plus derived
//! read-only views (`GeoPoint`, `Attitude`, `FieldOfView`, `Corners`)
//! and the encode-side `EncodeConfig`.

use crate::error::KlvFieldError;
use crate::klv::pack::OwnedRawField;
use crate::klv::universal_label::UniversalLabel;
use alloc::string::String;
use alloc::vec::Vec;

#[must_use]
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
    /// Item 90: Platform Pitch Angle (Full) — int32 mapped to ±90°.
    pub platform_pitch_full_deg: Option<f64>,
    /// Item 91: Platform Roll Angle (Full) — int32 mapped to ±90°.
    pub platform_roll_full_deg: Option<f64>,
    /// Item 50: Platform Angle of Attack — int16 mapped to ±20°.
    pub platform_angle_of_attack_deg: Option<f64>,

    // Sensor pose & position
    pub sensor_lat_deg: Option<f64>,
    pub sensor_lon_deg: Option<f64>,
    pub sensor_alt_m: Option<f64>,
    /// Item 75: Sensor Ellipsoid Height (WGS84-relative meters).
    pub sensor_ellipsoid_height_m: Option<f64>,
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
    /// Item 78: Frame Center Height Above Ellipsoid (WGS84-relative meters).
    pub frame_center_ellipsoid_height_m: Option<f64>,

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
    /// Tag 74 — VMTI Local Set (MISB ST 0903). Pass-through bytes;
    /// consumers needing typed access call `klv::st0903::decode` on
    /// `vmti.as_deref()?`. See `klv::st0903` for the typed layer.
    /// Sibling-layer pattern matches `security_local_set` (Tag 48 →
    /// `klv::st0102`).
    pub vmti: Option<Vec<u8>>,

    // Pass-through
    pub unknown: Vec<OwnedRawField>,
    pub field_errors: Vec<KlvFieldError>,

    /// Tags whose wire value was the INT_MIN sentinel for their signed linear
    /// mapping. INT_MIN is a spec-defined signal (not an error), so the
    /// corresponding typed field is left as `None` and the tag is recorded
    /// here instead of in `field_errors`.
    ///
    /// Use [`crate::klv::st0601::st0601_sentinel_meaning`] to look up the
    /// spec-defined meaning for each tag (Out of Range / Reserved / N/A).
    ///
    /// **Encode precedence:** if a tag appears in `sentinel_tags` AND its
    /// typed field is `Some(v)`, the value `v` is encoded — the sentinel
    /// applies only to absent (`None`) fields. This lets a caller build a
    /// sentinel-carrying record by setting the field to `None` and listing
    /// the tag here, while still allowing a non-sentinel override by
    /// setting the field to `Some(v)`.
    pub sentinel_tags: Vec<u32>,
}

impl Default for UasDatalinkLs {
    fn default() -> Self {
        Self {
            universal_label: UniversalLabel::ST_0601_LS,
            declared_version: UniversalLabel::ST_0601_LS.st0601_version_byte(),
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
            platform_pitch_full_deg: None,
            platform_roll_full_deg: None,
            platform_angle_of_attack_deg: None,
            sensor_lat_deg: None,
            sensor_lon_deg: None,
            sensor_alt_m: None,
            sensor_ellipsoid_height_m: None,
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
            frame_center_ellipsoid_height_m: None,
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
            vmti: None,
            unknown: Vec::new(),
            field_errors: Vec::new(),
            sentinel_tags: Vec::new(),
        }
    }
}

#[must_use]
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EncodeConfig {
    pub universal_label: UniversalLabel,
    /// Version byte to use for Tag 65 if the struct's `uas_ls_version` is None.
    pub version: u8,
}

impl Default for EncodeConfig {
    fn default() -> Self {
        Self {
            universal_label: UniversalLabel::ST_0601_LS,
            // Tag 65 ("UAS Datalink LS Version Number") encodes the
            // document revision the codebase conforms to: ST 0601.19 = 19.
            // Decoupled from `UL.version_byte()` because the canonical UL
            // per ST 0601.19 §6.2 has byte 13 = 0x00, which is not the
            // ST 0601 version number.
            version: 19,
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
        let (dl1, do1) = (
            self.corner_lat_offset_p1_deg?,
            self.corner_lon_offset_p1_deg?,
        );
        let (dl2, do2) = (
            self.corner_lat_offset_p2_deg?,
            self.corner_lon_offset_p2_deg?,
        );
        let (dl3, do3) = (
            self.corner_lat_offset_p3_deg?,
            self.corner_lon_offset_p3_deg?,
        );
        let (dl4, do4) = (
            self.corner_lat_offset_p4_deg?,
            self.corner_lon_offset_p4_deg?,
        );
        Some(Corners {
            p1: (lat0 + dl1, lon0 + do1),
            p2: (lat0 + dl2, lon0 + do2),
            p3: (lat0 + dl3, lon0 + do3),
            p4: (lat0 + dl4, lon0 + do4),
        })
    }
}
