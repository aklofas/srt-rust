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
use crate::klv::checksum::checksum_running_sum_16;
use crate::klv::length::{ber_len, ber_oid_len, write_ber, write_ber_oid};
use crate::klv::pack::OwnedRawField;
use crate::klv::st0601::mapping::encode_fixed_range;
use crate::klv::st0601::tags::{Encoding, TAGS};
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

pub fn encode(record: &UasDatalinkLs, out: &mut [u8]) -> Result<usize, KlvEncodeError> {
    encode_with(record, &EncodeOptions::default(), out)
}

pub fn encode_with(
    record: &UasDatalinkLs,
    opts: &EncodeOptions,
    out: &mut [u8],
) -> Result<usize, KlvEncodeError> {
    // Build the inner body into a temporary Vec, then assemble UL + BER length + body + checksum.
    let mut body: Vec<u8> = Vec::with_capacity(256);
    write_typed_fields(record, opts, &mut body)?;
    write_unknown_fields(record, &mut body)?;

    // Reserve room for Tag 1 (checksum) — 4 bytes (tag=1, len-byte=1, value=2).
    let body_len_with_checksum = body.len() + 4;
    let outer_len_bytes = ber_len(body_len_with_checksum);
    let total = 16 + outer_len_bytes + body_len_with_checksum;

    if out.len() < total {
        return Err(KlvEncodeError::BufferTooSmall {
            needed: total,
            got: out.len(),
        });
    }

    // 1) UL
    out[..16].copy_from_slice(&opts.universal_label.0);
    // 2) Outer BER length
    let written = write_ber(body_len_with_checksum, &mut out[16..])?;
    let body_offset = 16 + written;
    // 3) Body
    out[body_offset..body_offset + body.len()].copy_from_slice(&body);
    // 4) Tag 1 (checksum) tag + len
    let cksum_tag_offset = body_offset + body.len();
    out[cksum_tag_offset] = 0x01; // tag 1
    out[cksum_tag_offset + 1] = 0x02; // len 2
    // 5) Compute checksum across [UL .. start of checksum value]
    let cksum_value_offset = cksum_tag_offset + 2;
    let cksum = checksum_running_sum_16(&out[..cksum_value_offset]);
    out[cksum_value_offset] = (cksum >> 8) as u8;
    out[cksum_value_offset + 1] = cksum as u8;
    Ok(total)
}

pub fn encode_to_vec(record: &UasDatalinkLs) -> Result<Vec<u8>, KlvEncodeError> {
    let n = encoded_len(record);
    let mut buf = vec![0u8; n];
    let written = encode(record, &mut buf)?;
    buf.truncate(written);
    Ok(buf)
}

pub fn encoded_len(record: &UasDatalinkLs) -> usize {
    encoded_len_with(record, &EncodeOptions::default())
}

pub fn encoded_len_with(record: &UasDatalinkLs, opts: &EncodeOptions) -> usize {
    let mut body_len = 0usize;
    each_typed_field(record, opts, |tag, value_len| {
        body_len += ber_oid_len(tag as u32) + ber_len(value_len) + value_len;
    });
    for f in &record.unknown {
        body_len += ber_oid_len(f.tag) + ber_len(f.value.len()) + f.value.len();
    }
    let body_len_with_checksum = body_len + 4; // tag 1 (1 byte) + len byte (1) + value (2 bytes)
    16 + ber_len(body_len_with_checksum) + body_len_with_checksum
}

/// Visit each typed field that will be emitted, calling `visit(tag_id, value_len)`.
/// Used by both `encoded_len_with` (for sizing) and `write_typed_fields` (for emission).
fn each_typed_field<F: FnMut(u8, usize)>(
    record: &UasDatalinkLs,
    _opts: &EncodeOptions,
    mut visit: F,
) {
    // Tag 65 auto-emit if not explicitly set.
    let auto_version = record.uas_ls_version.is_none();

    for spec in TAGS {
        if spec.id == 1 {
            continue; // checksum is appended after
        }
        let len = match spec.id {
            2 => record.timestamp_us.map(|_| 8),
            3 => record.mission_id.as_ref().map(|s| s.len()),
            4 => record.platform_tail_number.as_ref().map(|s| s.len()),
            5 => record.platform_heading_deg.map(|_| 2),
            6 => record.platform_pitch_deg.map(|_| 2),
            7 => record.platform_roll_deg.map(|_| 2),
            8 => record.platform_true_airspeed.map(|_| 1),
            9 => record.platform_indicated_airspeed.map(|_| 1),
            10 => record.platform_designation.as_ref().map(|s| s.len()),
            11 => record.image_source_sensor.as_ref().map(|s| s.len()),
            12 => record.image_coordinate_system.as_ref().map(|s| s.len()),
            13 => record.sensor_lat_deg.map(|_| 4),
            14 => record.sensor_lon_deg.map(|_| 4),
            15 => record.sensor_alt_m.map(|_| 2),
            16 => record.sensor_hfov_deg.map(|_| 2),
            17 => record.sensor_vfov_deg.map(|_| 2),
            18 => record.sensor_rel_az_deg.map(|_| 4),
            19 => record.sensor_rel_el_deg.map(|_| 4),
            20 => record.sensor_rel_roll_deg.map(|_| 4),
            21 => record.slant_range_m.map(|_| 4),
            22 => record.target_width_m.map(|_| 2),
            23 => record.frame_center_lat_deg.map(|_| 4),
            24 => record.frame_center_lon_deg.map(|_| 4),
            25 => record.frame_center_elev_m.map(|_| 2),
            26 => record.corner_lat_offset_p1_deg.map(|_| 2),
            27 => record.corner_lon_offset_p1_deg.map(|_| 2),
            28 => record.corner_lat_offset_p2_deg.map(|_| 2),
            29 => record.corner_lon_offset_p2_deg.map(|_| 2),
            30 => record.corner_lat_offset_p3_deg.map(|_| 2),
            31 => record.corner_lon_offset_p3_deg.map(|_| 2),
            32 => record.corner_lat_offset_p4_deg.map(|_| 2),
            33 => record.corner_lon_offset_p4_deg.map(|_| 2),
            47 => record.generic_flag_data.map(|_| 1),
            48 => record.security_local_set.as_ref().map(|v| v.len()),
            50 => record.platform_call_sign.as_ref().map(|s| s.len()),
            65 => record.uas_ls_version.map(|_| 1).or(if auto_version {
                Some(1)
            } else {
                None
            }),
            82 => record.corner_lat_p1_deg.map(|_| 4),
            83 => record.corner_lon_p1_deg.map(|_| 4),
            84 => record.corner_lat_p2_deg.map(|_| 4),
            85 => record.corner_lon_p2_deg.map(|_| 4),
            86 => record.corner_lat_p3_deg.map(|_| 4),
            87 => record.corner_lon_p3_deg.map(|_| 4),
            88 => record.corner_lat_p4_deg.map(|_| 4),
            89 => record.corner_lon_p4_deg.map(|_| 4),
            _ => None,
        };
        if let Some(len) = len {
            visit(spec.id, len);
        }
    }
}

fn write_typed_fields(
    record: &UasDatalinkLs,
    opts: &EncodeOptions,
    body: &mut Vec<u8>,
) -> Result<(), KlvEncodeError> {
    let auto_version = record.uas_ls_version.is_none();

    for spec in TAGS {
        if spec.id == 1 {
            continue;
        }
        let mut scratch = [0u8; 8];
        // Encode this tag's value into a small buffer so we know its length.
        let value: Option<Vec<u8>> = match spec.id {
            2 => record.timestamp_us.map(|t| t.to_be_bytes().to_vec()),
            3 => record
                .mission_id
                .as_ref()
                .map(|s| check_string(3, s, &spec.encoding).map(|_| s.as_bytes().to_vec()))
                .transpose()?,
            4 => record
                .platform_tail_number
                .as_ref()
                .map(|s| check_string(4, s, &spec.encoding).map(|_| s.as_bytes().to_vec()))
                .transpose()?,
            5 => encode_ranged(record.platform_heading_deg, spec, &mut scratch)?,
            6 => encode_ranged(record.platform_pitch_deg, spec, &mut scratch)?,
            7 => encode_ranged(record.platform_roll_deg, spec, &mut scratch)?,
            8 => encode_ranged(record.platform_true_airspeed, spec, &mut scratch)?,
            9 => encode_ranged(record.platform_indicated_airspeed, spec, &mut scratch)?,
            10 => record
                .platform_designation
                .as_ref()
                .map(|s| check_string(10, s, &spec.encoding).map(|_| s.as_bytes().to_vec()))
                .transpose()?,
            11 => record
                .image_source_sensor
                .as_ref()
                .map(|s| check_string(11, s, &spec.encoding).map(|_| s.as_bytes().to_vec()))
                .transpose()?,
            12 => record
                .image_coordinate_system
                .as_ref()
                .map(|s| check_string(12, s, &spec.encoding).map(|_| s.as_bytes().to_vec()))
                .transpose()?,
            13 => encode_ranged(record.sensor_lat_deg, spec, &mut scratch)?,
            14 => encode_ranged(record.sensor_lon_deg, spec, &mut scratch)?,
            15 => encode_ranged(record.sensor_alt_m, spec, &mut scratch)?,
            16 => encode_ranged(record.sensor_hfov_deg, spec, &mut scratch)?,
            17 => encode_ranged(record.sensor_vfov_deg, spec, &mut scratch)?,
            18 => encode_ranged(record.sensor_rel_az_deg, spec, &mut scratch)?,
            19 => encode_ranged(record.sensor_rel_el_deg, spec, &mut scratch)?,
            20 => encode_ranged(record.sensor_rel_roll_deg, spec, &mut scratch)?,
            21 => encode_ranged(record.slant_range_m, spec, &mut scratch)?,
            22 => encode_ranged(record.target_width_m, spec, &mut scratch)?,
            23 => encode_ranged(record.frame_center_lat_deg, spec, &mut scratch)?,
            24 => encode_ranged(record.frame_center_lon_deg, spec, &mut scratch)?,
            25 => encode_ranged(record.frame_center_elev_m, spec, &mut scratch)?,
            26 => encode_ranged(record.corner_lat_offset_p1_deg, spec, &mut scratch)?,
            27 => encode_ranged(record.corner_lon_offset_p1_deg, spec, &mut scratch)?,
            28 => encode_ranged(record.corner_lat_offset_p2_deg, spec, &mut scratch)?,
            29 => encode_ranged(record.corner_lon_offset_p2_deg, spec, &mut scratch)?,
            30 => encode_ranged(record.corner_lat_offset_p3_deg, spec, &mut scratch)?,
            31 => encode_ranged(record.corner_lon_offset_p3_deg, spec, &mut scratch)?,
            32 => encode_ranged(record.corner_lat_offset_p4_deg, spec, &mut scratch)?,
            33 => encode_ranged(record.corner_lon_offset_p4_deg, spec, &mut scratch)?,
            47 => record.generic_flag_data.map(|b| vec![b]),
            48 => record.security_local_set.clone(),
            50 => record
                .platform_call_sign
                .as_ref()
                .map(|s| check_string(50, s, &spec.encoding).map(|_| s.as_bytes().to_vec()))
                .transpose()?,
            65 => {
                if let Some(v) = record.uas_ls_version {
                    Some(vec![v])
                } else if auto_version {
                    Some(vec![opts.version])
                } else {
                    None
                }
            }
            82 => encode_ranged(record.corner_lat_p1_deg, spec, &mut scratch)?,
            83 => encode_ranged(record.corner_lon_p1_deg, spec, &mut scratch)?,
            84 => encode_ranged(record.corner_lat_p2_deg, spec, &mut scratch)?,
            85 => encode_ranged(record.corner_lon_p2_deg, spec, &mut scratch)?,
            86 => encode_ranged(record.corner_lat_p3_deg, spec, &mut scratch)?,
            87 => encode_ranged(record.corner_lon_p3_deg, spec, &mut scratch)?,
            88 => encode_ranged(record.corner_lat_p4_deg, spec, &mut scratch)?,
            89 => encode_ranged(record.corner_lon_p4_deg, spec, &mut scratch)?,
            _ => None,
        };
        if let Some(value) = value {
            let mut tag_buf = [0u8; 8];
            let n = write_ber_oid(spec.id as u32, &mut tag_buf)?;
            body.extend_from_slice(&tag_buf[..n]);
            let mut len_buf = [0u8; 16];
            let m = write_ber(value.len(), &mut len_buf)?;
            body.extend_from_slice(&len_buf[..m]);
            body.extend_from_slice(&value);
        }
    }
    Ok(())
}

fn write_unknown_fields(record: &UasDatalinkLs, body: &mut Vec<u8>) -> Result<(), KlvEncodeError> {
    for f in &record.unknown {
        let mut tag_buf = [0u8; 8];
        let n = write_ber_oid(f.tag, &mut tag_buf)?;
        body.extend_from_slice(&tag_buf[..n]);
        let mut len_buf = [0u8; 16];
        let m = write_ber(f.value.len(), &mut len_buf)?;
        body.extend_from_slice(&len_buf[..m]);
        body.extend_from_slice(&f.value);
    }
    Ok(())
}

fn encode_ranged(
    value: Option<f64>,
    spec: &crate::klv::st0601::tags::TagSpec,
    scratch: &mut [u8; 8],
) -> Result<Option<Vec<u8>>, KlvEncodeError> {
    let Some(v) = value else { return Ok(None) };
    let r = spec
        .range
        .as_ref()
        .expect("ranged tag must have LinearRange");
    encode_fixed_range(r, spec.id as u32, v, &mut scratch[..r.byte_length])?;
    Ok(Some(scratch[..r.byte_length].to_vec()))
}

fn check_string(tag: u32, s: &str, enc: &Encoding) -> Result<(), KlvEncodeError> {
    if let Encoding::Utf8 { max_bytes } = enc {
        if s.len() > *max_bytes {
            return Err(KlvEncodeError::StringTooLong {
                tag,
                max: *max_bytes,
            });
        }
    }
    Ok(())
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

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn encode_minimal_record_round_trip_via_iter() {
        // Encode a record with just a timestamp; verify the bytes parse back.
        let mut r = UasDatalinkLs::default();
        r.timestamp_us = Some(0x0123_4567_89AB_CDEF);
        let mut buf = vec![0u8; 256];
        let n = encode(&r, &mut buf).unwrap();
        let bytes = &buf[..n];

        // Verify UL prefix
        assert_eq!(&bytes[..16], &UniversalLabel::ST_0601_LS.0);

        // Parse outer BER length
        use crate::klv::length::read_ber;
        let (body_len, body) = read_ber(&bytes[16..]).unwrap();
        assert_eq!(body_len, body.len());
        assert!(body_len >= 13); // tag 2 (1) + len (1) + 8 + tag 65 (1) + len (1) + 1 (auto-version) + checksum (3)

        // Parse body
        use crate::klv::pack::Iter;
        let mut tags_seen: Vec<u32> = Vec::new();
        for r in Iter::local_set(body) {
            let f = r.unwrap();
            tags_seen.push(f.tag);
        }
        assert!(tags_seen.contains(&2), "tag 2 (timestamp) missing");
        assert!(tags_seen.contains(&65), "tag 65 (auto-version) missing");
        assert!(tags_seen.contains(&1), "tag 1 (checksum) missing");
    }

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn encoded_len_matches_actual() {
        let mut r = UasDatalinkLs::default();
        r.timestamp_us = Some(0xCAFE);
        r.sensor_lat_deg = Some(45.0);
        r.sensor_lon_deg = Some(-122.0);
        let predicted = encoded_len(&r);
        let mut buf = vec![0u8; predicted];
        let actual = encode(&r, &mut buf).unwrap();
        assert_eq!(predicted, actual);
    }

    #[test]
    fn encode_buffer_too_small() {
        let r = UasDatalinkLs::default();
        let mut buf = vec![0u8; 5];
        let err = encode(&r, &mut buf).unwrap_err();
        matches!(err, KlvEncodeError::BufferTooSmall { .. });
    }

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn encode_out_of_range_rejects() {
        let mut r = UasDatalinkLs::default();
        r.sensor_lat_deg = Some(95.0); // out of [-90, 90]
        let mut buf = vec![0u8; 256];
        let err = encode(&r, &mut buf).unwrap_err();
        matches!(err, KlvEncodeError::OutOfRange { tag: 13, .. });
    }

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn encode_string_too_long_rejects() {
        let mut r = UasDatalinkLs::default();
        r.platform_call_sign = Some("x".repeat(200));
        let mut buf = vec![0u8; 512];
        let err = encode(&r, &mut buf).unwrap_err();
        matches!(err, KlvEncodeError::StringTooLong { tag: 50, max: 127 });
    }

    #[test]
    fn encode_with_custom_ul() {
        let r = UasDatalinkLs::default();
        let custom_ul = UniversalLabel::new([0xAB; 16]);
        let opts = EncodeOptions {
            universal_label: custom_ul,
            version: 0x09,
        };
        let mut buf = vec![0u8; 256];
        let n = encode_with(&r, &opts, &mut buf).unwrap();
        assert_eq!(&buf[..16], &[0xAB; 16]);
        let _ = n;
    }

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn encode_to_vec_succeeds() {
        let mut r = UasDatalinkLs::default();
        r.timestamp_us = Some(0xABCD_EF00);
        let bytes = encode_to_vec(&r).unwrap();
        assert!(!bytes.is_empty());
        assert_eq!(&bytes[..16], &UniversalLabel::ST_0601_LS.0);
    }
}
