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
    /// Item 5: Platform Heading Angle — encode range [0, 360] deg (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub platform_heading_deg: Option<f64>,
    /// Item 6: Platform Pitch Angle — encode range [-20, 20] deg (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`]; for
    /// the full ±90° range use [`Self::platform_pitch_full_deg`] (Item 90).
    pub platform_pitch_deg: Option<f64>,
    /// Item 7: Platform Roll Angle — encode range [-50, 50] deg (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`]; for
    /// the full ±90° range use [`Self::platform_roll_full_deg`] (Item 91).
    pub platform_roll_deg: Option<f64>,
    /// Item 8: Platform True Airspeed — encode range [0, 255] m/s (uint8).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub platform_true_airspeed: Option<f64>,
    /// Item 9: Platform Indicated Airspeed — encode range [0, 255] m/s (uint8).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub platform_indicated_airspeed: Option<f64>,
    /// Item 90: Platform Pitch Angle (Full) — encode range [-90, 90] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    /// Full-range twin of [`Self::platform_pitch_deg`] (Item 6, ±20°).
    pub platform_pitch_full_deg: Option<f64>,
    /// Item 91: Platform Roll Angle (Full) — encode range [-90, 90] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    /// Full-range twin of [`Self::platform_roll_deg`] (Item 7, ±50°).
    pub platform_roll_full_deg: Option<f64>,
    /// Item 94: MIIS Core Identifier (MISB ST 1204 binary format).
    /// Raw bytes — decode/inspect via [`crate::klv::st1204`].
    pub miis_core_id: Option<Vec<u8>>,
    /// Item 50: Platform Angle of Attack — encode range [-20, 20] deg (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub platform_angle_of_attack_deg: Option<f64>,

    // Sensor pose & position
    /// Item 13: Sensor Latitude — encode range [-90, 90] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub sensor_lat_deg: Option<f64>,
    /// Item 14: Sensor Longitude — encode range [-180, 180] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub sensor_lon_deg: Option<f64>,
    /// Item 15: Sensor True Altitude — encode range [-900, 19000] m (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub sensor_alt_m: Option<f64>,
    /// Item 75: Sensor Ellipsoid Height (WGS84) — encode range [-900, 19000] m (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub sensor_ellipsoid_height_m: Option<f64>,
    /// Item 16: Sensor Horizontal FOV — encode range [0, 180] deg (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub sensor_hfov_deg: Option<f64>,
    /// Item 17: Sensor Vertical FOV — encode range [0, 180] deg (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub sensor_vfov_deg: Option<f64>,
    /// Item 18: Sensor Relative Azimuth — encode range [0, 360] deg (uint32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub sensor_rel_az_deg: Option<f64>,
    /// Item 19: Sensor Relative Elevation — encode range [-180, 180] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub sensor_rel_el_deg: Option<f64>,
    /// Item 20: Sensor Relative Roll — encode range [0, 360] deg (uint32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub sensor_rel_roll_deg: Option<f64>,

    // Ranging & frame center
    /// Item 21: Slant Range — encode range [0, 5000000] m (uint32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub slant_range_m: Option<f64>,
    /// Item 22: Target Width — encode range [0, 10000] m (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub target_width_m: Option<f64>,
    /// Item 23: Frame Center Latitude — encode range [-90, 90] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub frame_center_lat_deg: Option<f64>,
    /// Item 24: Frame Center Longitude — encode range [-180, 180] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub frame_center_lon_deg: Option<f64>,
    /// Item 25: Frame Center Elevation — encode range [-900, 19000] m (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub frame_center_elev_m: Option<f64>,
    /// Item 78: Frame Center Height Above Ellipsoid (WGS84) — encode range
    /// [-900, 19000] m (uint16). Values outside raise
    /// [`crate::error::KlvEncodeError::OutOfRange`].
    pub frame_center_ellipsoid_height_m: Option<f64>,

    // Image corners — offsets from frame center (tags 26-33)
    /// Item 26: Offset Corner Latitude Point 1 — encode range [-0.075, 0.075] deg (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`]; for the full
    /// latitude range use [`Self::corner_lat_p1_deg`] (Item 82, ±90°).
    pub corner_lat_offset_p1_deg: Option<f64>,
    /// Item 27: Offset Corner Longitude Point 1 — encode range [-0.075, 0.075] deg (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`]; for the full
    /// longitude range use [`Self::corner_lon_p1_deg`] (Item 83, ±180°).
    pub corner_lon_offset_p1_deg: Option<f64>,
    /// Item 28: Offset Corner Latitude Point 2 — encode range [-0.075, 0.075] deg (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`]; for the full
    /// latitude range use [`Self::corner_lat_p2_deg`] (Item 84, ±90°).
    pub corner_lat_offset_p2_deg: Option<f64>,
    /// Item 29: Offset Corner Longitude Point 2 — encode range [-0.075, 0.075] deg (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`]; for the full
    /// longitude range use [`Self::corner_lon_p2_deg`] (Item 85, ±180°).
    pub corner_lon_offset_p2_deg: Option<f64>,
    /// Item 30: Offset Corner Latitude Point 3 — encode range [-0.075, 0.075] deg (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`]; for the full
    /// latitude range use [`Self::corner_lat_p3_deg`] (Item 86, ±90°).
    pub corner_lat_offset_p3_deg: Option<f64>,
    /// Item 31: Offset Corner Longitude Point 3 — encode range [-0.075, 0.075] deg (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`]; for the full
    /// longitude range use [`Self::corner_lon_p3_deg`] (Item 87, ±180°).
    pub corner_lon_offset_p3_deg: Option<f64>,
    /// Item 32: Offset Corner Latitude Point 4 — encode range [-0.075, 0.075] deg (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`]; for the full
    /// latitude range use [`Self::corner_lat_p4_deg`] (Item 88, ±90°).
    pub corner_lat_offset_p4_deg: Option<f64>,
    /// Item 33: Offset Corner Longitude Point 4 — encode range [-0.075, 0.075] deg (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`]; for the full
    /// longitude range use [`Self::corner_lon_p4_deg`] (Item 89, ±180°).
    pub corner_lon_offset_p4_deg: Option<f64>,

    // Image corners — full lat/lon (tags 82-89, ST 0601.13+)
    /// Item 82: Corner Latitude Point 1 (Full) — encode range [-90, 90] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    /// Full-range twin of [`Self::corner_lat_offset_p1_deg`] (Item 26, ±0.075°).
    pub corner_lat_p1_deg: Option<f64>,
    /// Item 83: Corner Longitude Point 1 (Full) — encode range [-180, 180] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    /// Full-range twin of [`Self::corner_lon_offset_p1_deg`] (Item 27, ±0.075°).
    pub corner_lon_p1_deg: Option<f64>,
    /// Item 84: Corner Latitude Point 2 (Full) — encode range [-90, 90] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    /// Full-range twin of [`Self::corner_lat_offset_p2_deg`] (Item 28, ±0.075°).
    pub corner_lat_p2_deg: Option<f64>,
    /// Item 85: Corner Longitude Point 2 (Full) — encode range [-180, 180] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    /// Full-range twin of [`Self::corner_lon_offset_p2_deg`] (Item 29, ±0.075°).
    pub corner_lon_p2_deg: Option<f64>,
    /// Item 86: Corner Latitude Point 3 (Full) — encode range [-90, 90] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    /// Full-range twin of [`Self::corner_lat_offset_p3_deg`] (Item 30, ±0.075°).
    pub corner_lat_p3_deg: Option<f64>,
    /// Item 87: Corner Longitude Point 3 (Full) — encode range [-180, 180] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    /// Full-range twin of [`Self::corner_lon_offset_p3_deg`] (Item 31, ±0.075°).
    pub corner_lon_p3_deg: Option<f64>,
    /// Item 88: Corner Latitude Point 4 (Full) — encode range [-90, 90] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    /// Full-range twin of [`Self::corner_lat_offset_p4_deg`] (Item 32, ±0.075°).
    pub corner_lat_p4_deg: Option<f64>,
    /// Item 89: Corner Longitude Point 4 (Full) — encode range [-180, 180] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    /// Full-range twin of [`Self::corner_lon_offset_p4_deg`] (Item 33, ±0.075°).
    pub corner_lon_p4_deg: Option<f64>,

    // Target location & tracking (tags 40-46)
    /// Item 40: Target Location Latitude — encode range [-90, 90] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub target_location_lat_deg: Option<f64>,
    /// Item 41: Target Location Longitude — encode range [-180, 180] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub target_location_lon_deg: Option<f64>,
    /// Item 42: Target Location Elevation — encode range [-900, 19000] m (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub target_location_elev_m: Option<f64>,
    /// Item 43: Target Track Gate Width — encode range [0, 510] px (uint8).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub target_track_gate_width_px: Option<f64>,
    /// Item 44: Target Track Gate Height — encode range [0, 510] px (uint8).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub target_track_gate_height_px: Option<f64>,
    /// Item 45: Target Error CE90 — encode range [0, 4095] m (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub target_error_ce90_m: Option<f64>,
    /// Item 46: Target Error LE90 — encode range [0, 4095] m (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub target_error_le90_m: Option<f64>,

    // Weather / atmospheric (tags 35-38, 49, 53-55)
    /// Item 35: Wind Direction — encode range [0, 360] deg (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub wind_direction_deg: Option<f64>,
    /// Item 36: Wind Speed — encode range [0, 100] m/s (uint8).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub wind_speed: Option<f64>,
    /// Item 37: Static Pressure — encode range [0, 5000] mbar (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub static_pressure_mbar: Option<f64>,
    /// Item 38: Density Altitude — encode range [-900, 19000] m (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub density_altitude_m: Option<f64>,
    /// Item 49: Differential Pressure — encode range [0, 5000] mbar (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub differential_pressure_mbar: Option<f64>,
    /// Item 53: Airfield Barometric Pressure — encode range [0, 5000] mbar (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub airfield_barometric_pressure_mbar: Option<f64>,
    /// Item 54: Airfield Elevation — encode range [-900, 19000] m (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub airfield_elevation_m: Option<f64>,
    /// Item 55: Relative Humidity — encode range [0, 100] % (uint8).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub relative_humidity_pct: Option<f64>,

    // Extended platform state (tags 51, 52, 56-58, 64, 92, 93)
    /// Item 51: Platform Vertical Speed — encode range [-180, 180] m/s (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub platform_vertical_speed: Option<f64>,
    /// Item 52: Platform Sideslip Angle — encode range [-20, 20] deg (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`]; for
    /// the full ±180° range use [`Self::platform_sideslip_full_deg`] (Item 93).
    pub platform_sideslip_deg: Option<f64>,
    /// Item 56: Platform Ground Speed — encode range [0, 255] m/s (uint8).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub platform_ground_speed: Option<f64>,
    /// Item 57: Ground Range — encode range [0, 5000000] m (uint32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub ground_range_m: Option<f64>,
    /// Item 58: Platform Fuel Remaining — encode range [0, 10000] kg (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub platform_fuel_remaining_kg: Option<f64>,
    /// Item 64: Platform Magnetic Heading — encode range [0, 360] deg (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub platform_magnetic_heading_deg: Option<f64>,
    /// Item 92: Platform Angle of Attack (Full) — encode range [-90, 90] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub platform_angle_of_attack_full_deg: Option<f64>,
    /// Item 93: Platform Sideslip Angle (Full) — encode range [-180, 180] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    /// Full-range twin of [`Self::platform_sideslip_deg`] (Item 52, ±20°).
    pub platform_sideslip_full_deg: Option<f64>,

    // Alternate platform (tags 67-69, 71, 76)
    /// Item 67: Alternate Platform Latitude — encode range [-90, 90] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub alternate_platform_lat_deg: Option<f64>,
    /// Item 68: Alternate Platform Longitude — encode range [-180, 180] deg (int32).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub alternate_platform_lon_deg: Option<f64>,
    /// Item 69: Alternate Platform Altitude — encode range [-900, 19000] m (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub alternate_platform_alt_m: Option<f64>,
    /// Item 71: Alternate Platform Heading — encode range [0, 360] deg (uint16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub alternate_platform_heading_deg: Option<f64>,
    /// Item 76: Alternate Platform Ellipsoid Height (WGS84) — encode range
    /// [-900, 19000] m (uint16). Values outside raise
    /// [`crate::error::KlvEncodeError::OutOfRange`].
    pub alternate_platform_ellipsoid_height_m: Option<f64>,

    // Sensor velocity (tags 79-80)
    /// Item 79: Sensor North Velocity — encode range [-327, 327] m/s (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub sensor_north_velocity: Option<f64>,
    /// Item 80: Sensor East Velocity — encode range [-327, 327] m/s (int16).
    /// Values outside raise [`crate::error::KlvEncodeError::OutOfRange`].
    pub sensor_east_velocity: Option<f64>,

    // Misc
    /// Item 47: Generic Flag Data — bitfield per ST 0601.19 Table 3:
    /// bit 0 laser range on, bit 1 auto-track on, bit 2 IR polarity
    /// (set = black hot), bit 3 icing detected, bit 4 slant range
    /// measured (vs. calculated), bit 5 image invalid, bits 6-7 reserved.
    pub generic_flag_data: Option<u8>,
    pub security_local_set: Option<Vec<u8>>,
    /// Tag 74 — VMTI Local Set (MISB ST 0903). Pass-through bytes;
    /// consumers needing typed access call `klv::st0903::decode` on
    /// `vmti.as_deref()?`. See `klv::st0903` for the typed layer.
    /// Sibling-layer pattern matches `security_local_set` (Tag 48 →
    /// `klv::st0102`).
    pub vmti: Option<Vec<u8>>,

    // Raw scalar & string items (tags 39, 60-62, 70, 72, 106-108, 129, 135)
    pub outside_air_temp_c: Option<i8>,
    /// Item 60: Weapon Load — bit-packed nibbles (high→low): store
    /// station, store substation, weapon type, weapon variant. Kept as a
    /// raw opaque `u16`, not a sub-struct — callers needing individual
    /// nibbles shift and mask this value themselves.
    pub weapon_load: Option<u16>,
    pub weapon_fired: Option<u8>,
    pub laser_prf_code: Option<u16>,
    pub alternate_platform_name: Option<String>,
    pub event_start_time_us: Option<u64>,
    pub stream_designator: Option<String>,
    pub operational_base: Option<String>,
    pub broadcast_source: Option<String>,
    pub target_id: Option<String>,
    pub communications_method: Option<String>,

    // Coded enums (tags 34, 63, 77)
    /// Item 34: Icing Detected — icing-detector state. See [`IcingDetected`].
    pub icing_detected: Option<IcingDetected>,
    /// Item 63: Sensor Field of View Name — named FOV preset. See [`SensorFovName`].
    pub sensor_fov_name: Option<SensorFovName>,
    /// Item 77: Operational Mode — operating mode of the portrayed event.
    /// See [`OperationalMode`].
    pub operational_mode: Option<OperationalMode>,

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
    /// **Encode semantics:** encoding a record that carries sentinel tags
    /// re-emits the INT_MIN bytes for each sentinel tag whose typed field is
    /// `None`. If the typed field is `Some(v)`, the value `v` is encoded
    /// instead — a non-None field always wins over a `sentinel_tags` entry.
    /// The encoder also auto-injects Tag 65 (version) and the trailing
    /// checksum, so the output byte sequence is not guaranteed to match the
    /// original wire input byte-for-byte; the sentinel *value* round-trips,
    /// but the emission order is: typed fields in ascending tag order, then
    /// `sentinel_tags` entries, then `unknown` fields in caller order.
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
            miis_core_id: None,
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
            target_location_lat_deg: None,
            target_location_lon_deg: None,
            target_location_elev_m: None,
            target_track_gate_width_px: None,
            target_track_gate_height_px: None,
            target_error_ce90_m: None,
            target_error_le90_m: None,
            wind_direction_deg: None,
            wind_speed: None,
            static_pressure_mbar: None,
            density_altitude_m: None,
            differential_pressure_mbar: None,
            airfield_barometric_pressure_mbar: None,
            airfield_elevation_m: None,
            relative_humidity_pct: None,
            platform_vertical_speed: None,
            platform_sideslip_deg: None,
            platform_ground_speed: None,
            ground_range_m: None,
            platform_fuel_remaining_kg: None,
            platform_magnetic_heading_deg: None,
            platform_angle_of_attack_full_deg: None,
            platform_sideslip_full_deg: None,
            alternate_platform_lat_deg: None,
            alternate_platform_lon_deg: None,
            alternate_platform_alt_m: None,
            alternate_platform_heading_deg: None,
            alternate_platform_ellipsoid_height_m: None,
            sensor_north_velocity: None,
            sensor_east_velocity: None,
            generic_flag_data: None,
            security_local_set: None,
            vmti: None,
            outside_air_temp_c: None,
            weapon_load: None,
            weapon_fired: None,
            laser_prf_code: None,
            alternate_platform_name: None,
            event_start_time_us: None,
            stream_designator: None,
            operational_base: None,
            broadcast_source: None,
            target_id: None,
            communications_method: None,
            icing_detected: None,
            sensor_fov_name: None,
            operational_mode: None,
            unknown: Vec::new(),
            field_errors: Vec::new(),
            sentinel_tags: Vec::new(),
        }
    }
}

// ============================================================================
// Coded value enums (tags 34, 63, 77)
// ============================================================================

/// Item 34: Icing Detected (ST 0601.19 §8.34) — flag for icing detected at
/// the aircraft location, sensed by a vibrating-probe ice detector.
///
/// | Code | Meaning |
/// |---:|---|
/// | 0 | `DetectorOff` |
/// | 1 | `NoIcingDetected` |
/// | 2 | `IcingDetected` |
/// | other | `Other(code)` — wire-unknown, round-trips byte-exact |
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IcingDetected {
    DetectorOff,
    NoIcingDetected,
    IcingDetected,
    /// Wire-unknown codepoint; round-trips byte-exact through encode.
    Other(u8),
}

impl IcingDetected {
    pub(crate) fn from_wire(b: u8) -> Self {
        match b {
            0 => Self::DetectorOff,
            1 => Self::NoIcingDetected,
            2 => Self::IcingDetected,
            other => Self::Other(other),
        }
    }

    pub(crate) fn to_wire(self) -> u8 {
        match self {
            Self::DetectorOff => 0,
            Self::NoIcingDetected => 1,
            Self::IcingDetected => 2,
            Self::Other(b) => b,
        }
    }
}

/// Item 63: Sensor Field of View Name (ST 0601.19 §8.63) — indicates the
/// Motion Imagery sensor's current lens type / FOV preset.
///
/// | Code | Meaning |
/// |---:|---|
/// | 0 | `Ultranarrow` |
/// | 1 | `Narrow` |
/// | 2 | `Medium` |
/// | 3 | `Wide` |
/// | 4 | `Ultrawide` |
/// | 5 | `NarrowMedium` |
/// | 6 | `TwoXUltranarrow` |
/// | 7 | `FourXUltranarrow` |
/// | 8 | `ContinuousZoom` |
/// | other | `Other(code)` — wire-unknown, round-trips byte-exact |
///
/// **Spec discrepancy:** the item's own definition table (§8.63) caps the
/// KLV range at `[0, 7]`, but the Details subsection's worked table — ST
/// 0601.19 §8.63.1 Table 4 — lists a 9th codepoint, `8` = "Continuous
/// Zoom". Modeled per Table 4 since it is the more complete of the two
/// spec tables and real-world encoders emit it.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SensorFovName {
    Ultranarrow,
    Narrow,
    Medium,
    Wide,
    Ultrawide,
    NarrowMedium,
    TwoXUltranarrow,
    FourXUltranarrow,
    ContinuousZoom,
    /// Wire-unknown codepoint; round-trips byte-exact through encode.
    Other(u8),
}

impl SensorFovName {
    pub(crate) fn from_wire(b: u8) -> Self {
        match b {
            0 => Self::Ultranarrow,
            1 => Self::Narrow,
            2 => Self::Medium,
            3 => Self::Wide,
            4 => Self::Ultrawide,
            5 => Self::NarrowMedium,
            6 => Self::TwoXUltranarrow,
            7 => Self::FourXUltranarrow,
            8 => Self::ContinuousZoom,
            other => Self::Other(other),
        }
    }

    pub(crate) fn to_wire(self) -> u8 {
        match self {
            Self::Ultranarrow => 0,
            Self::Narrow => 1,
            Self::Medium => 2,
            Self::Wide => 3,
            Self::Ultrawide => 4,
            Self::NarrowMedium => 5,
            Self::TwoXUltranarrow => 6,
            Self::FourXUltranarrow => 7,
            Self::ContinuousZoom => 8,
            Self::Other(b) => b,
        }
    }
}

/// Item 77: Operational Mode (ST 0601.19 §8.77) — indicates the mode of
/// operations of the event portrayed in the Motion Imagery, per the
/// §8.77.1 Table 5 enumeration.
///
/// | Code | Meaning |
/// |---:|---|
/// | 0 | `OtherMode` |
/// | 1 | `Operational` |
/// | 2 | `Training` |
/// | 3 | `Exercise` |
/// | 4 | `Maintenance` |
/// | 5 | `Test` |
/// | other | `Other(code)` — wire-unknown, round-trips byte-exact |
///
/// Spec code `0` is named "Other" in Table 5; this crate names it
/// `OtherMode` to avoid colliding with the catch-all `Other(code)`
/// fallback arm above.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationalMode {
    OtherMode,
    Operational,
    Training,
    Exercise,
    Maintenance,
    Test,
    /// Wire-unknown codepoint; round-trips byte-exact through encode.
    Other(u8),
}

impl OperationalMode {
    pub(crate) fn from_wire(b: u8) -> Self {
        match b {
            0 => Self::OtherMode,
            1 => Self::Operational,
            2 => Self::Training,
            3 => Self::Exercise,
            4 => Self::Maintenance,
            5 => Self::Test,
            other => Self::Other(other),
        }
    }

    pub(crate) fn to_wire(self) -> u8 {
        match self {
            Self::OtherMode => 0,
            Self::Operational => 1,
            Self::Training => 2,
            Self::Exercise => 3,
            Self::Maintenance => 4,
            Self::Test => 5,
            Self::Other(b) => b,
        }
    }
}

/// What the `encode*` entry points do when a ranged value falls outside
/// its ST 0601 mapped range.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutOfRangePolicy {
    /// Reject the record with [`crate::error::KlvEncodeError::OutOfRange`] (default).
    #[default]
    Error,
    /// Emit the tag's spec-defined "Out of Range" special value instead of
    /// erroring — ST 0601.19 §7.5 / requirement ST 0601.13-27 (`0x8000` /
    /// `0x80000000` for 2-/4-byte signed mappings). Applies to the tags
    /// whose INT_MIN sentinel means Out of Range (Tags 6, 7, 50, 51, 52,
    /// 79, 80, 90–93 — see
    /// [`st0601_sentinel_meaning`][crate::klv::st0601::st0601_sentinel_meaning]),
    /// all of which are modeled as encodable [`UasDatalinkLs`] fields. Any
    /// non-finite input still returns
    /// [`crate::error::KlvEncodeError::OutOfRange`].
    Indicator,
}

#[must_use]
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EncodeConfig {
    pub universal_label: UniversalLabel,
    /// Version byte to use for Tag 65 if the struct's `uas_ls_version` is None.
    pub version: u8,
    /// Policy for ranged values that fall outside their ST 0601 mapped range.
    pub out_of_range_policy: OutOfRangePolicy,
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
            out_of_range_policy: OutOfRangePolicy::Error,
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
