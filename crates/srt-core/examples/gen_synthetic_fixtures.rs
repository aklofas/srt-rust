//! Generate synthetic ST 0601 fixture files into tests/fixtures/st0601/.
//! Run via: `cargo run --example gen_synthetic_fixtures`.
//! Idempotent — running it again produces byte-identical output.

#![allow(clippy::field_reassign_with_default)]

use std::fs;
use std::path::Path;

use srt_core::UniversalLabel;
use srt_core::klv::pack::OwnedRawField;
use srt_core::klv::st0601::{
    EncodeOptions, UasDatalinkLs, encode_to_vec, encode_with, encoded_len_with,
};

fn main() {
    let out_dir = Path::new("crates/srt-core/tests/fixtures/st0601");
    fs::create_dir_all(out_dir).unwrap();

    write(&out_dir.join("synthetic_minimal.klv"), &minimal());
    write(&out_dir.join("synthetic_full.klv"), &full());
    write(&out_dir.join("synthetic_funky_ul.klv"), &funky_ul());
    write(
        &out_dir.join("synthetic_field_errors.klv"),
        &field_errors_record(),
    );
    println!("Wrote 4 synthetic fixtures to {}", out_dir.display());
}

fn write(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    println!("wrote {} ({} bytes)", path.display(), bytes.len());
}

fn minimal() -> Vec<u8> {
    let mut r = UasDatalinkLs::default();
    r.timestamp_us = Some(1_700_000_000_000_000);
    encode_to_vec(&r).unwrap()
}

fn full() -> Vec<u8> {
    let mut r = UasDatalinkLs::default();
    r.timestamp_us = Some(1_700_123_456_789_000);
    r.mission_id = Some("M-001".to_owned());
    r.platform_tail_number = Some("N12345".to_owned());
    r.platform_designation = Some("DRONE-A".to_owned());
    r.image_source_sensor = Some("EO-NOSE".to_owned());
    r.image_coordinate_system = Some("WGS-84".to_owned());
    r.platform_call_sign = Some("ECHO-1".to_owned());
    r.platform_heading_deg = Some(123.45);
    r.platform_pitch_deg = Some(-5.0);
    r.platform_roll_deg = Some(10.0);
    r.platform_true_airspeed = Some(45.0);
    r.platform_indicated_airspeed = Some(40.0);
    r.sensor_lat_deg = Some(38.123456);
    r.sensor_lon_deg = Some(-121.654321);
    r.sensor_alt_m = Some(2500.0);
    r.sensor_hfov_deg = Some(45.0);
    r.sensor_vfov_deg = Some(30.0);
    r.sensor_rel_az_deg = Some(180.0);
    r.sensor_rel_el_deg = Some(-45.0);
    r.sensor_rel_roll_deg = Some(0.0);
    r.slant_range_m = Some(3000.0);
    r.target_width_m = Some(150.0);
    r.frame_center_lat_deg = Some(38.0);
    r.frame_center_lon_deg = Some(-121.5);
    r.frame_center_elev_m = Some(0.0);
    r.corner_lat_offset_p1_deg = Some(0.01);
    r.corner_lon_offset_p1_deg = Some(0.01);
    r.corner_lat_offset_p2_deg = Some(0.01);
    r.corner_lon_offset_p2_deg = Some(-0.01);
    r.corner_lat_offset_p3_deg = Some(-0.01);
    r.corner_lon_offset_p3_deg = Some(-0.01);
    r.corner_lat_offset_p4_deg = Some(-0.01);
    r.corner_lon_offset_p4_deg = Some(0.01);
    r.corner_lat_p1_deg = Some(38.001);
    r.corner_lon_p1_deg = Some(-121.499);
    r.corner_lat_p2_deg = Some(38.001);
    r.corner_lon_p2_deg = Some(-121.501);
    r.corner_lat_p3_deg = Some(37.999);
    r.corner_lon_p3_deg = Some(-121.501);
    r.corner_lat_p4_deg = Some(37.999);
    r.corner_lon_p4_deg = Some(-121.499);
    r.generic_flag_data = Some(0x09);
    r.security_local_set = Some(vec![0x01, 0x02, 0x03]);
    encode_to_vec(&r).unwrap()
}

fn funky_ul() -> Vec<u8> {
    let mut r = UasDatalinkLs::default();
    r.timestamp_us = Some(123);
    r.sensor_lat_deg = Some(45.0);
    let opts = EncodeOptions {
        universal_label: UniversalLabel::new([
            0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x09,
            0x00, 0x00,
        ]),
        version: 0x09,
    };
    let n = encoded_len_with(&r, &opts);
    let mut buf = vec![0u8; n];
    let written = encode_with(&r, &opts, &mut buf).unwrap();
    buf.truncate(written);
    buf
}

fn field_errors_record() -> Vec<u8> {
    let mut r = UasDatalinkLs::default();
    r.timestamp_us = Some(789);
    r.unknown.push(OwnedRawField {
        tag: 13, // lat — but with malformed length (3 bytes instead of 4)
        value: vec![0x00, 0x00, 0x00],
    });
    encode_to_vec(&r).unwrap()
}
