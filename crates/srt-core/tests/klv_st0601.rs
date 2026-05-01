//! Integration tests for klv::st0601 — round trips from the public API surface.

use srt_core::UniversalLabel;
use srt_core::klv::st0601::{
    decode, decode_strict, decode_unchecked, encode, encode_to_vec, encode_with, encoded_len,
    EncodeOptions, UasDatalinkLs,
};

#[allow(clippy::field_reassign_with_default)]
#[test]
fn full_record_round_trip() {
    let mut r = UasDatalinkLs::default();
    r.timestamp_us = Some(1_700_000_123_456_789);
    r.platform_tail_number = Some("N12345".to_owned());
    r.platform_designation = Some("CAYUSE-1".to_owned());
    r.platform_heading_deg = Some(270.5);
    r.platform_pitch_deg = Some(-3.5);
    r.platform_roll_deg = Some(12.0);
    r.sensor_lat_deg = Some(38.123456);
    r.sensor_lon_deg = Some(-121.654321);
    r.sensor_alt_m = Some(2500.0);
    r.sensor_hfov_deg = Some(45.0);
    r.sensor_vfov_deg = Some(30.0);
    r.frame_center_lat_deg = Some(38.0);
    r.frame_center_lon_deg = Some(-121.5);
    r.frame_center_elev_m = Some(0.0);
    r.slant_range_m = Some(3000.0);
    r.target_width_m = Some(150.0);

    let bytes = encode_to_vec(&r).unwrap();
    let parsed = decode(&bytes).unwrap();

    // Spot-check a handful of typed fields.
    assert_eq!(parsed.platform_tail_number.as_deref(), Some("N12345"));
    assert_eq!(parsed.platform_designation.as_deref(), Some("CAYUSE-1"));
    assert!(parsed.field_errors.is_empty());
    assert!(parsed.unknown.is_empty());
    let pos = parsed.sensor_position().unwrap();
    assert!((pos.lat_deg - 38.123456).abs() < 1e-6);
    assert!((pos.alt_m - 2500.0).abs() < 1.0);
}

#[allow(clippy::field_reassign_with_default)]
#[test]
fn encoded_len_predicts_actual_size() {
    let mut r = UasDatalinkLs::default();
    r.timestamp_us = Some(0xDEAD_BEEF);
    r.platform_call_sign = Some("ECHO-1".to_owned());
    r.sensor_lat_deg = Some(45.0);
    let predicted = encoded_len(&r);
    let mut buf = vec![0u8; predicted];
    let n = encode(&r, &mut buf).unwrap();
    assert_eq!(predicted, n);
}

#[test]
fn decode_strict_round_trip() {
    let r = UasDatalinkLs::default();
    let bytes = encode_to_vec(&r).unwrap();
    let parsed = decode_strict(&bytes).unwrap();
    assert_eq!(parsed.universal_label, UniversalLabel::ST_0601_LS);
}

#[test]
fn decode_strict_rejects_arbitrary_ul() {
    let r = UasDatalinkLs::default();
    let opts = EncodeOptions {
        universal_label: UniversalLabel::new([0xFF; 16]),
        version: 0x13,
    };
    let bytes = {
        let n = encoded_len(&r) + 16; // upper bound
        let mut buf = vec![0u8; n];
        let written = encode_with(&r, &opts, &mut buf).unwrap();
        buf.truncate(written);
        buf
    };
    assert!(decode_strict(&bytes).is_err());
    assert!(decode(&bytes).is_ok()); // permissive accepts it
}

#[allow(clippy::field_reassign_with_default)]
#[test]
fn corner_full_form_round_trip() {
    let mut r = UasDatalinkLs::default();
    r.corner_lat_p1_deg = Some(45.1);
    r.corner_lon_p1_deg = Some(-122.1);
    r.corner_lat_p2_deg = Some(45.1);
    r.corner_lon_p2_deg = Some(-121.9);
    r.corner_lat_p3_deg = Some(44.9);
    r.corner_lon_p3_deg = Some(-121.9);
    r.corner_lat_p4_deg = Some(44.9);
    r.corner_lon_p4_deg = Some(-122.1);
    let bytes = encode_to_vec(&r).unwrap();
    let parsed = decode(&bytes).unwrap();
    let c = parsed.corners().unwrap();
    assert!((c.p1.0 - 45.1).abs() < 1e-6);
    assert!((c.p3.1 - -121.9).abs() < 1e-6);
}

#[allow(clippy::field_reassign_with_default)]
#[test]
fn unchecked_skips_checksum() {
    let mut r = UasDatalinkLs::default();
    r.timestamp_us = Some(42);
    let mut bytes = encode_to_vec(&r).unwrap();
    *bytes.last_mut().unwrap() ^= 0xFF;
    assert!(decode(&bytes).is_err());
    let parsed = decode_unchecked(&bytes).unwrap();
    assert_eq!(parsed.timestamp_us, Some(42));
}
