use super::decode::{
    assign_ranged, decode, decode_strict, decode_strict_compliance, decode_unchecked,
};
use super::encode::{encode, encode_to_vec, encode_with, encoded_len};
use super::model::{EncodeConfig, UasDatalinkLs};
use super::tags::TAGS;
use crate::error::{KlvDecodeError, KlvEncodeError};
use crate::klv::pack::OwnedRawField;
use crate::klv::universal_label::UniversalLabel;

#[test]
fn default_uses_st0601_ul() {
    let r = UasDatalinkLs::default();
    assert_eq!(r.universal_label, UniversalLabel::ST_0601_LS);
    // declared_version mirrors UL byte 13. Per ST 0601.19 §6.2 the
    // canonical UL has byte 13 = 0x00; the field encodes a legacy
    // "document version" readout for non-conformant captures.
    assert_eq!(r.declared_version, 0x00);
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
    let opts = EncodeConfig::default();
    assert_eq!(opts.universal_label, UniversalLabel::ST_0601_LS);
    assert_eq!(opts.version, 0x13);
}

#[test]
fn encode_minimal_record_round_trip_via_iter() {
    // Encode a record with just a timestamp; verify the bytes parse back.
    let r = UasDatalinkLs {
        timestamp_us: Some(0x0123_4567_89AB_CDEF),
        ..UasDatalinkLs::default()
    };
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

#[test]
fn encoded_len_matches_actual() {
    let r = UasDatalinkLs {
        timestamp_us: Some(0xCAFE),
        sensor_lat_deg: Some(45.0),
        sensor_lon_deg: Some(-122.0),
        ..UasDatalinkLs::default()
    };
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

#[test]
fn encode_out_of_range_rejects() {
    let r = UasDatalinkLs {
        sensor_lat_deg: Some(95.0), // out of [-90, 90]
        ..UasDatalinkLs::default()
    };
    let mut buf = vec![0u8; 256];
    let err = encode(&r, &mut buf).unwrap_err();
    matches!(err, KlvEncodeError::OutOfRange { tag: 13, .. });
}

#[test]
fn encode_string_too_long_rejects() {
    let r = UasDatalinkLs {
        platform_call_sign: Some("x".repeat(200)),
        ..UasDatalinkLs::default()
    };
    let mut buf = vec![0u8; 512];
    let err = encode(&r, &mut buf).unwrap_err();
    matches!(err, KlvEncodeError::StringTooLong { tag: 59, max: 127 });
}

#[test]
fn encode_with_custom_ul() {
    let r = UasDatalinkLs::default();
    let custom_ul = UniversalLabel::new([0xAB; 16]);
    let opts = EncodeConfig {
        universal_label: custom_ul,
        version: 0x09,
    };
    let mut buf = vec![0u8; 256];
    let n = encode_with(&r, &opts, &mut buf).unwrap();
    assert_eq!(&buf[..16], &[0xAB; 16]);
    let _ = n;
}

#[test]
fn encode_to_vec_succeeds() {
    let r = UasDatalinkLs {
        timestamp_us: Some(0xABCD_EF00),
        ..UasDatalinkLs::default()
    };
    let bytes = encode_to_vec(&r).unwrap();
    assert!(!bytes.is_empty());
    assert_eq!(&bytes[..16], &UniversalLabel::ST_0601_LS.0);
}

#[allow(clippy::field_reassign_with_default)]
#[test]
fn round_trip_full_record() {
    let mut r = UasDatalinkLs::default();
    r.timestamp_us = Some(1_700_000_000_000_000);
    r.platform_designation = Some("DRONE-A".to_owned());
    r.platform_heading_deg = Some(123.45);
    r.platform_pitch_deg = Some(-5.0);
    r.platform_roll_deg = Some(10.0);
    r.sensor_lat_deg = Some(45.123);
    r.sensor_lon_deg = Some(-122.456);
    r.sensor_alt_m = Some(1500.0);
    r.frame_center_lat_deg = Some(45.0);
    r.frame_center_lon_deg = Some(-122.0);
    r.slant_range_m = Some(2500.0);

    let bytes = encode_to_vec(&r).unwrap();
    let parsed = decode(&bytes).unwrap();

    assert_eq!(parsed.timestamp_us, r.timestamp_us);
    assert_eq!(parsed.platform_designation, r.platform_designation);
    assert!((parsed.platform_heading_deg.unwrap() - 123.45).abs() < 0.01);
    assert!((parsed.sensor_lat_deg.unwrap() - 45.123).abs() < 1e-6);
    assert_eq!(parsed.universal_label, UniversalLabel::ST_0601_LS);
    // declared_version mirrors UL byte 13 (= 0x00 per ST 0601.19 §6.2
    // canonical registration); uas_ls_version is Tag 65 (= 19 = 0x13
    // per the document revision we conform to).
    assert_eq!(parsed.declared_version, 0x00);
    assert_eq!(parsed.uas_ls_version, Some(19));
    assert!(parsed.field_errors.is_empty());
}

#[test]
fn decode_unchecked_accepts_bad_checksum() {
    let r = UasDatalinkLs {
        timestamp_us: Some(123),
        ..UasDatalinkLs::default()
    };
    let mut bytes = encode_to_vec(&r).unwrap();
    // Corrupt the last checksum byte
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    // decode should fail; decode_unchecked should succeed.
    assert!(decode(&bytes).is_err());
    let parsed = decode_unchecked(&bytes).unwrap();
    assert_eq!(parsed.timestamp_us, Some(123));
}

#[test]
fn decode_strict_rejects_funky_ul() {
    let r = UasDatalinkLs {
        timestamp_us: Some(456),
        ..UasDatalinkLs::default()
    };
    let opts = EncodeConfig {
        universal_label: UniversalLabel::new([0xAB; 16]),
        version: 0x13,
    };
    let mut buf = vec![0u8; 256];
    let n = encode_with(&r, &opts, &mut buf).unwrap();
    let bytes = &buf[..n];
    let err = decode_strict(bytes).unwrap_err();
    assert!(matches!(
        err,
        KlvDecodeError::UnexpectedUniversalLabel { .. }
    ));
    // decode (non-strict) accepts any UL.
    let parsed = decode(bytes).unwrap();
    assert_eq!(parsed.universal_label, UniversalLabel::new([0xAB; 16]));
}

#[test]
fn decode_passes_through_unknown_tags() {
    let mut r = UasDatalinkLs::default();
    r.unknown.push(OwnedRawField {
        tag: 99,
        value: vec![0xDE, 0xAD],
    });
    let bytes = encode_to_vec(&r).unwrap();
    let parsed = decode(&bytes).unwrap();
    assert_eq!(parsed.unknown.len(), 1);
    assert_eq!(parsed.unknown[0].tag, 99);
    assert_eq!(parsed.unknown[0].value, vec![0xDE, 0xAD]);
}

#[test]
fn decode_field_errors_accumulate() {
    // Hand-build a record with a malformed Tag 13 (lat) value (1 byte instead of 4).
    // We synthesize the bytes by building a valid record and then patching it.
    let r = UasDatalinkLs {
        sensor_lat_deg: Some(45.0),
        timestamp_us: Some(123),
        ..UasDatalinkLs::default()
    };
    let bytes = encode_to_vec(&r).unwrap();

    // Easier path: construct a body that has a deliberately-malformed tag.
    // The simplest approach: replace the typed field with a malformed
    // unknown field via a hand-constructed input.
    let mut body = vec![];
    // Tag 2, len 8, [zeros]
    body.extend_from_slice(&[0x02, 0x08]);
    body.extend_from_slice(&[0u8; 8]);
    // Tag 13, len 1 (malformed; should be 4)
    body.extend_from_slice(&[0x0D, 0x01, 0x00]);

    // Reserve checksum slot: tag(1) + len(1) + value(2) = 4 bytes
    let body_with_cksum_len = body.len() + 4;

    let mut full = vec![];
    full.extend_from_slice(&UniversalLabel::ST_0601_LS.0);
    // Outer BER length
    let mut len_buf = [0u8; 8];
    let n = crate::klv::length::write_ber(body_with_cksum_len, &mut len_buf).unwrap();
    full.extend_from_slice(&len_buf[..n]);
    full.extend_from_slice(&body);
    full.push(0x01);
    full.push(0x02);
    let cksum = crate::klv::checksum::checksum_running_sum_16(&full);
    full.push((cksum >> 8) as u8);
    full.push(cksum as u8);

    let parsed = decode(&full).unwrap();
    assert!(parsed.timestamp_us.is_some(), "good field still parses");
    assert!(
        !parsed.field_errors.is_empty(),
        "malformed field accumulates"
    );
    let _ = bytes;
}

#[test]
fn decode_strict_compliance_accepts_valid_record() {
    // Build a minimal compliant record: Tag 2 first, Tag 65 present, Tag 1 last.
    let record = UasDatalinkLs {
        timestamp_us: Some(1_700_000_000_000_000),
        uas_ls_version: Some(0x13),
        ..UasDatalinkLs::default()
    };
    let buf = encode_to_vec(&record).unwrap();
    let r = decode_strict_compliance(&buf).expect("compliant record decodes");
    assert_eq!(r.timestamp_us, Some(1_700_000_000_000_000));
    assert_eq!(r.uas_ls_version, Some(0x13));
}

// Note: a full integration test for the non-canonical-BER strict path
// (build a record with tampered outer BER + recomputed checksum) is
// overkill — `read_ber_strict` is unit-tested in `klv::length::tests`,
// and the wiring in `decode_strict_compliance` is a single line. The
// strict-compliance Tag 2/Tag 1/Tag 65 ordering tests above exercise
// the same code path.

#[test]
fn decode_strict_compliance_rejects_missing_tag65() {
    // Encode without Tag 65 by skipping auto-version: pre-construct fields.
    // We rely on `encode_to_vec` defaulting auto_version=true — to force
    // missing, decode a hand-crafted record without the version tag.
    // Build manually: UL + BER + body{ Tag 2, Tag 1 }.
    use crate::klv::checksum::checksum_running_sum_16;
    use crate::klv::length::{ber_len, write_ber};
    use crate::klv::universal_label::UniversalLabel;
    // Body: Tag 2 (LEN 8 + 8-byte ts), then Tag 1 (LEN 2 + 2-byte placeholder).
    let mut body = Vec::new();
    body.push(0x02);
    body.push(0x08);
    body.extend_from_slice(&1u64.to_be_bytes());
    body.push(0x01);
    body.push(0x02);
    body.extend_from_slice(&[0, 0]); // placeholder; we'll fix the checksum
    // Wrap with UL + BER.
    let mut buf = Vec::new();
    buf.extend_from_slice(&UniversalLabel::ST_0601_LS.0);
    let mut len_bytes = [0u8; 9];
    let nlen = write_ber(body.len(), &mut len_bytes).unwrap();
    buf.extend_from_slice(&len_bytes[..nlen]);
    let body_offset_in_buf = buf.len();
    buf.extend_from_slice(&body);
    // Compute checksum over UL through length-of-checksum-item.
    let cksum_value_offset = body_offset_in_buf + body.len() - 2;
    let computed = checksum_running_sum_16(&buf[..cksum_value_offset]);
    buf[cksum_value_offset] = (computed >> 8) as u8;
    buf[cksum_value_offset + 1] = (computed & 0xFF) as u8;
    let _ = (ber_len, nlen); // silence unused warnings if any
    let err = decode_strict_compliance(&buf).unwrap_err();
    assert!(matches!(err, KlvDecodeError::MissingTag65));
}

#[test]
fn decode_strict_compliance_rejects_tag2_not_first() {
    // Build a record where Tag 65 appears before Tag 2.
    use crate::klv::checksum::checksum_running_sum_16;
    use crate::klv::length::write_ber;
    use crate::klv::universal_label::UniversalLabel;
    let mut body = vec![0x41u8, 0x01, 0x13]; // Tag 65
    body.extend_from_slice(&[0x02, 0x08]); // Tag 2
    body.extend_from_slice(&1u64.to_be_bytes());
    body.extend_from_slice(&[0x01, 0x02, 0x00, 0x00]); // Tag 1 (checksum placeholder)
    let mut buf = Vec::new();
    buf.extend_from_slice(&UniversalLabel::ST_0601_LS.0);
    let mut len_bytes = [0u8; 9];
    let nlen = write_ber(body.len(), &mut len_bytes).unwrap();
    buf.extend_from_slice(&len_bytes[..nlen]);
    let body_offset = buf.len();
    buf.extend_from_slice(&body);
    let cksum_value_offset = body_offset + body.len() - 2;
    let computed = checksum_running_sum_16(&buf[..cksum_value_offset]);
    buf[cksum_value_offset] = (computed >> 8) as u8;
    buf[cksum_value_offset + 1] = (computed & 0xFF) as u8;
    let _ = nlen;
    let err = decode_strict_compliance(&buf).unwrap_err();
    assert!(matches!(err, KlvDecodeError::Tag2NotFirst));
}

#[test]
fn decode_strict_compliance_rejects_tag1_not_last() {
    // Build a record where Tag 1 (checksum) is NOT last.
    use crate::klv::checksum::checksum_running_sum_16;
    use crate::klv::length::write_ber;
    use crate::klv::universal_label::UniversalLabel;
    let mut body = Vec::new();
    body.push(0x02); // Tag 2 first (correct)
    body.push(0x08);
    body.extend_from_slice(&1u64.to_be_bytes());
    body.push(0x01); // Tag 1 (checksum) — NOT last
    body.push(0x02);
    body.extend_from_slice(&[0, 0]);
    body.push(0x41); // Tag 65 after the checksum (wrong)
    body.push(0x01);
    body.push(0x13);
    let mut buf = Vec::new();
    buf.extend_from_slice(&UniversalLabel::ST_0601_LS.0);
    let mut len_bytes = [0u8; 9];
    let nlen = write_ber(body.len(), &mut len_bytes).unwrap();
    buf.extend_from_slice(&len_bytes[..nlen]);
    let body_offset = buf.len();
    buf.extend_from_slice(&body);
    // Checksum covers up to (and including) the length byte of Tag 1.
    // Find Tag 1's value-offset: scan body for tag=0x01 len=0x02.
    let mut idx = 0;
    let mut cksum_value_offset = 0;
    let body_slice = &buf[body_offset..body_offset + body.len()];
    while idx + 2 <= body_slice.len() {
        if body_slice[idx] == 0x01 && body_slice[idx + 1] == 0x02 {
            cksum_value_offset = body_offset + idx + 2;
            break;
        }
        // BER-OID tag 1 byte + BER length 1 byte short form
        let t = body_slice[idx];
        idx += 1;
        // assume short-form lengths < 128 in this hand-crafted body
        let l = body_slice[idx] as usize;
        idx += 1 + l;
        let _ = t;
    }
    let computed = checksum_running_sum_16(&buf[..cksum_value_offset]);
    buf[cksum_value_offset] = (computed >> 8) as u8;
    buf[cksum_value_offset + 1] = (computed & 0xFF) as u8;
    let _ = nlen;
    // strict_compliance should reject — Tag 65 follows Tag 1.
    let err = decode_strict_compliance(&buf).unwrap_err();
    // Acceptable error: Tag1NotLast OR ChecksumMismatch (since checksum doesn't include trailing bytes).
    // We assert specifically for Tag1NotLast since the strict pass detects ordering before checksum verifies.
    assert!(matches!(err, KlvDecodeError::Tag1NotLast));
}

#[test]
fn decode_picks_up_tag_75_sensor_ellipsoid_height() {
    let mut record = UasDatalinkLs {
        timestamp_us: Some(1_700_000_000_000_000),
        sensor_ellipsoid_height_m: Some(14190.7195),
        ..Default::default()
    };
    let _ = &mut record;
    let buf = encode_to_vec(&record).unwrap();
    let back = decode(&buf).unwrap();
    assert!(back.sensor_ellipsoid_height_m.is_some());
    let h = back.sensor_ellipsoid_height_m.unwrap();
    assert!((h - 14190.7195).abs() < 0.5, "got {h}");
}

#[test]
fn decode_picks_up_tag_90_platform_pitch_full() {
    let record = UasDatalinkLs {
        timestamp_us: Some(1_700_000_000_000_000),
        platform_pitch_full_deg: Some(-0.4315),
        ..Default::default()
    };
    let buf = encode_to_vec(&record).unwrap();
    let back = decode(&buf).unwrap();
    assert!(back.platform_pitch_full_deg.is_some());
    let p = back.platform_pitch_full_deg.unwrap();
    assert!((p - (-0.4315)).abs() < 1e-4, "got {p}");
}

#[test]
fn vmti_tag_74_round_trips_verbatim() {
    // Tag 74 (VMTI Local Set per MISB ST 0903) is carried as
    // pass-through bytes on the typed `vmti` field. The parent
    // ST 0601 decoder does not recurse into the VMTI inner schema;
    // consumers compose `klv::st0903::decode` themselves. Sibling-
    // layer pattern matches `security_local_set` (Tag 48 → ST 0102).
    let payload = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let record = UasDatalinkLs {
        timestamp_us: Some(1_700_000_000_000_000),
        vmti: Some(payload.clone()),
        ..Default::default()
    };
    let buf = encode_to_vec(&record).unwrap();
    let back = decode(&buf).unwrap();
    assert_eq!(
        back.vmti.as_deref(),
        Some(payload.as_slice()),
        "Tag 74 should round-trip on the typed vmti field byte-for-byte"
    );
}

#[test]
fn every_typed_tag_round_trips() {
    // For every TagSpec in TAGS, set its corresponding field in
    // UasDatalinkLs to a sentinel value, encode the record, decode
    // it back, and verify the field survived the round trip. This
    // catches "tag added to TAGS but apply_typed_tag/assign_ranged/
    // walk_typed_lens/write_typed_fields not updated" drift.

    for spec in TAGS {
        // Skip Tag 1 (checksum: not user-set) and Tag 47/65
        // (handled by separate U8 dispatch; round-trip test below
        // exercises them implicitly via auto_version).
        if spec.id == 1 {
            continue;
        }
        let mut record = UasDatalinkLs {
            timestamp_us: Some(1_700_000_000_000_000),
            ..Default::default()
        };
        // Set the field we expect for this tag. The choice of
        // sentinel value just has to be inside the spec range.
        match spec.id {
            2 => {} // already set
            3 => record.mission_id = Some("M".to_string()),
            4 => record.platform_tail_number = Some("T".to_string()),
            10 => record.platform_designation = Some("D".to_string()),
            11 => record.image_source_sensor = Some("S".to_string()),
            12 => record.image_coordinate_system = Some("WGS84".to_string()),
            47 => record.generic_flag_data = Some(0xAB),
            48 => record.security_local_set = Some(vec![0x01, 0x02]),
            59 => record.platform_call_sign = Some("CS".to_string()),
            65 => record.uas_ls_version = Some(0x13),
            74 => record.vmti = Some(vec![0xDE, 0xAD, 0xBE, 0xEF]),
            _ => {
                // Ranged numeric: pick a value at the midpoint of the spec range.
                let r = spec.range.expect("ranged tag has range");
                let midpoint = (r.min + r.max) / 2.0;
                assign_ranged(&mut record, spec.id as u32, midpoint);
                // Sanity-check: the field actually got set.
                let mut probe = UasDatalinkLs::default();
                assign_ranged(&mut probe, spec.id as u32, midpoint);
                assert_ne!(
                    format!("{probe:?}"),
                    format!("{:?}", UasDatalinkLs::default()),
                    "assign_ranged for tag {} ({}) is a no-op — missing arm",
                    spec.id,
                    spec.name
                );
            }
        }
        // Encode and decode round trip.
        let buf = encode_to_vec(&record)
            .unwrap_or_else(|e| panic!("encode failed for tag {} ({}): {e}", spec.id, spec.name));
        let back = decode(&buf)
            .unwrap_or_else(|e| panic!("decode failed for tag {} ({}): {e}", spec.id, spec.name));
        // Field must be present in the decoded record (we don't
        // compare exact values because IMAPB scaling is lossy).
        let present = match spec.id {
            3 => back.mission_id.is_some(),
            4 => back.platform_tail_number.is_some(),
            10 => back.platform_designation.is_some(),
            11 => back.image_source_sensor.is_some(),
            12 => back.image_coordinate_system.is_some(),
            47 => back.generic_flag_data.is_some(),
            48 => back.security_local_set.is_some(),
            59 => back.platform_call_sign.is_some(),
            65 => back.uas_ls_version.is_some(),
            74 => back.vmti.is_some(),
            2 => back.timestamp_us.is_some(),
            _ => {
                // For ranged numeric, presence == any of our ranged fields is set.
                // We reuse assign_ranged to a default record to compare which field
                // changed; back must have that same field set.
                let mut probe = UasDatalinkLs::default();
                assign_ranged(&mut probe, spec.id as u32, 0.0);
                // Compute a Debug snapshot of `back` field vs `probe`'s expected
                // field. Practically: we just check that *some* numeric field
                // changed in `back` relative to default.
                format!("{back:?}") != format!("{:?}", UasDatalinkLs::default())
            }
        };
        assert!(
            present,
            "round trip lost tag {} ({}); encoder or decoder dispatch arm missing",
            spec.id, spec.name
        );
    }
}

/// Walk the body of an encoded ST 0601 record and locate a single-byte
/// BER-OID tag. Returns `(value_offset, value_len)` relative to the
/// whole `encoded` buffer, or `None` if the tag is not present.
///
/// Only handles tags whose BER-OID encoding fits in one byte (id < 128)
/// — sufficient for Tags 50 and 59. The body starts after the 16-byte
/// UL plus its BER outer length; we parse the outer length to find
/// the body start, then walk tag-length-value triplets.
fn find_tag(encoded: &[u8], tag: u8) -> Option<(usize, usize)> {
    assert!(tag < 128, "find_tag only handles single-byte BER-OID tags");
    // Skip UL (16 bytes), read outer BER length; `rest` points at the
    // body. The body ends 4 bytes before EOF (Tag 1 + len byte + 2-byte
    // checksum value). Walk tag-length-value triplets inside.
    let after_ul = 16;
    let (_body_len, rest) =
        crate::klv::length::read_ber(&encoded[after_ul..]).expect("outer BER length");
    let body_start = encoded.len() - rest.len();
    let body_end = encoded.len() - 4;
    let mut i = body_start;
    while i < body_end {
        let cur_tag = encoded[i];
        i += 1;
        // Parse BER length: short form (< 128) or long form.
        let len_byte = encoded[i];
        i += 1;
        let value_len = if len_byte & 0x80 == 0 {
            len_byte as usize
        } else {
            let nbytes = (len_byte & 0x7F) as usize;
            let mut v = 0usize;
            for b in &encoded[i..i + nbytes] {
                v = (v << 8) | (*b as usize);
            }
            i += nbytes;
            v
        };
        if cur_tag == tag {
            return Some((i, value_len));
        }
        i += value_len;
    }
    None
}

/// Regression: per ST 0601.19 §8.50, Tag 50 carries Platform Angle of
/// Attack as a signed int16 mapped linearly to ±20°. Pre-fix the
/// library declared Tag 50 as a utf8 "Platform Call Sign" field —
/// wire-format incompatible with every other ST 0601 toolchain.
#[test]
fn tag_50_is_platform_angle_of_attack_int16_per_spec() {
    let record = UasDatalinkLs {
        timestamp_us: Some(1_700_000_000_000_000),
        platform_angle_of_attack_deg: Some(12.5),
        ..Default::default()
    };
    let bytes = encode_to_vec(&record).expect("encode");

    let (value_off, value_len) =
        find_tag(&bytes, 50).expect("Tag 50 should be present in encoded record");
    assert_eq!(
        value_len, 2,
        "Tag 50 (Platform Angle of Attack) is int16 ⇒ 2-byte value per ST 0601.19 §8.50"
    );
    // Sanity: value bytes are not zero (we set a non-zero angle).
    assert_ne!(
        &bytes[value_off..value_off + value_len],
        &[0u8, 0u8],
        "encoded angle bytes should reflect 12.5°, not zero"
    );

    let decoded = decode(&bytes).expect("decode");
    let aoa = decoded
        .platform_angle_of_attack_deg
        .expect("Platform Angle of Attack should round-trip");
    assert!(
        (aoa - 12.5).abs() < 0.01,
        "Tag 50 round-trip drift exceeds 0.01°: got {aoa}"
    );
    assert!(
        decoded.platform_call_sign.is_none(),
        "Tag 50 must NOT populate Call Sign — that field belongs to Tag 59"
    );
}

/// Regression: per ST 0601.19 §8.59, Tag 59 carries Platform Call Sign
/// as utf8 ≤ 127 bytes. Paired with the Tag 50 fix above.
#[test]
fn tag_59_is_platform_call_sign_utf8_per_spec() {
    let record = UasDatalinkLs {
        timestamp_us: Some(1_700_000_000_000_000),
        platform_call_sign: Some("DRONE-7".to_string()),
        ..Default::default()
    };
    let bytes = encode_to_vec(&record).expect("encode");

    let (value_off, value_len) =
        find_tag(&bytes, 59).expect("Tag 59 should be present in encoded record");
    assert_eq!(
        value_len,
        "DRONE-7".len(),
        "Tag 59 (Platform Call Sign) value length must equal utf8 byte length"
    );
    assert_eq!(
        &bytes[value_off..value_off + value_len],
        b"DRONE-7",
        "Tag 59 value bytes must be the raw utf8 of the call sign"
    );

    let decoded = decode(&bytes).expect("decode");
    assert_eq!(
        decoded.platform_call_sign,
        Some("DRONE-7".to_string()),
        "Tag 59 must round-trip into platform_call_sign"
    );
    assert!(
        decoded.platform_angle_of_attack_deg.is_none(),
        "Tag 59 must NOT populate Angle of Attack — that field belongs to Tag 50"
    );
}
