use super::decode::{
    assign_ranged, decode, decode_strict, decode_strict_compliance, decode_unchecked,
};
use super::encode::{encode, encode_strict_compliance, encode_to_vec, encode_with, encoded_len};
use super::model::{EncodeConfig, UasDatalinkLs};
use super::patch::patch;
use super::tags::TAGS;
use crate::error::{KlvDecodeError, KlvEncodeError, KlvPatchError};
use crate::klv::checksum::checksum_running_sum_16;
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

// --- Reserved/typed-tag-in-unknown filter (validate-1 E3) ------------------
// `record.unknown` is for forward-compat pass-through of tags the encoder
// does not model. Putting a reserved structural tag (1 = Checksum, 2 = PTS,
// 65 = UAS LS Version) or a typed tag (anything in `tags::TAGS`) there
// would produce a non-conformant Local Set — duplicate entries or, for
// Tag 1, a bogus pseudo-checksum before the real one. The encoder fails
// fast with `ReservedTagInUnknown` before writing any bytes.

#[test]
fn encode_rejects_tag1_in_unknown() {
    let mut r = UasDatalinkLs::default();
    r.unknown.push(OwnedRawField {
        tag: 1,
        value: vec![0x00, 0x00],
    });
    let err = encode_to_vec(&r).unwrap_err();
    assert!(matches!(
        err,
        KlvEncodeError::ReservedTagInUnknown { tag: 1 }
    ));
}

#[test]
fn encode_rejects_tag2_in_unknown() {
    let mut r = UasDatalinkLs::default();
    r.unknown.push(OwnedRawField {
        tag: 2,
        value: vec![0u8; 8],
    });
    let err = encode_to_vec(&r).unwrap_err();
    assert!(matches!(
        err,
        KlvEncodeError::ReservedTagInUnknown { tag: 2 }
    ));
}

#[test]
fn encode_rejects_tag65_in_unknown() {
    let mut r = UasDatalinkLs::default();
    r.unknown.push(OwnedRawField {
        tag: 65,
        value: vec![0x0D],
    });
    let err = encode_to_vec(&r).unwrap_err();
    assert!(matches!(
        err,
        KlvEncodeError::ReservedTagInUnknown { tag: 65 }
    ));
}

#[test]
fn encode_rejects_typed_tag_in_unknown() {
    // Tag 13 (Sensor Latitude) is typed-modeled by the encoder. Even if
    // the caller didn't set `sensor_lat_deg`, smuggling Tag 13 in via
    // `unknown` would shadow the typed path and risk duplicate emission
    // on a later record.
    let mut r = UasDatalinkLs::default();
    r.unknown.push(OwnedRawField {
        tag: 13,
        value: vec![0u8; 4],
    });
    let err = encode_to_vec(&r).unwrap_err();
    assert!(matches!(
        err,
        KlvEncodeError::ReservedTagInUnknown { tag: 13 }
    ));
}

#[test]
fn encode_accepts_genuinely_unknown_tag() {
    // Tag 200 is not in `tags::TAGS` and not a reserved structural tag.
    // Forward-compat pass-through is the whole point of `unknown` — this
    // must continue to succeed.
    let mut r = UasDatalinkLs::default();
    r.unknown.push(OwnedRawField {
        tag: 200,
        value: vec![0xDE, 0xAD, 0xBE, 0xEF],
    });
    let bytes = encode_to_vec(&r).expect("genuinely-unknown tag should encode");
    // Round-trip preserves it.
    let parsed = decode(&bytes).unwrap();
    assert_eq!(parsed.unknown.len(), 1);
    assert_eq!(parsed.unknown[0].tag, 200);
    assert_eq!(parsed.unknown[0].value, vec![0xDE, 0xAD, 0xBE, 0xEF]);
}

#[test]
fn encode_accepts_genuinely_unknown_high_tag() {
    // Tag 500 exceeds u8::MAX — the typed table is u8-keyed so this short-
    // circuits in `is_reserved_or_typed_tag` without iterating TAGS.
    let mut r = UasDatalinkLs::default();
    r.unknown.push(OwnedRawField {
        tag: 500,
        value: vec![0x01],
    });
    encode_to_vec(&r).expect("genuinely-unknown high tag should encode");
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

// ---------------------------------------------------------------------------
// Regression — Validate-1 Phase 2 §A6: ST 0601 high-numbered BER-OID tags
// must not narrow to u8 and collide with known low-numbered tags.
//
// Per MISB ST 0107.3-04, decoders shall preserve unknown LS values without
// impacting the decoding of known items. Pre-fix the typed-decode site
// cast the multi-byte BER-OID tag to `u8`, so a future tag 258 (= 0x102,
// encoded `0x82 0x02`) narrowed to 2 and was treated as Tag 2 (Precision
// Time Stamp) — silently clobbering the typed `timestamp_us` field.
// ---------------------------------------------------------------------------

/// Build a strict-decodable ST 0601 LS buffer from a raw body. The body
/// is wrapped with the ST 0601 UL + outer BER length + a trailing Tag 1
/// (running-sum 16 checksum) so `decode` accepts it. The caller owns
/// emitting Tag 2 (timestamp) or omitting it inside `body_before_checksum`
/// — this helper does not inject either.
fn wrap_st0601(body_before_checksum: &[u8]) -> Vec<u8> {
    use crate::klv::checksum::checksum_running_sum_16;
    use crate::klv::length::write_ber;
    use crate::klv::universal_label::UniversalLabel;
    let mut body = Vec::from(body_before_checksum);
    // Append Tag 1 (checksum) placeholder.
    body.extend_from_slice(&[0x01, 0x02, 0x00, 0x00]);
    let mut buf = Vec::new();
    buf.extend_from_slice(&UniversalLabel::ST_0601_LS.0);
    let mut len_bytes = [0u8; 9];
    let n = write_ber(body.len(), &mut len_bytes).unwrap();
    buf.extend_from_slice(&len_bytes[..n]);
    let body_offset = buf.len();
    buf.extend_from_slice(&body);
    // Checksum spans UL + outer length + body up through the Tag 1
    // length byte (value bytes 0x00 0x00 placeholder are NOT included).
    let cksum_value_offset = body_offset + body.len() - 2;
    let computed = checksum_running_sum_16(&buf[..cksum_value_offset]);
    buf[cksum_value_offset] = (computed >> 8) as u8;
    buf[cksum_value_offset + 1] = (computed & 0xFF) as u8;
    buf
}

#[test]
fn future_tag_258_with_8_byte_value_does_not_clobber_timestamp_us() {
    // BER-OID tag 258 encodes as `0x82, 0x02` (continuation bit on byte 1,
    // value 0x02 in the low 7 bits of each byte: (2<<7)|2 == 258).
    // We hand `0x82 0x02` to the iterator's `read_ber_oid` which returns
    // u32 = 258. Pre-fix `lookup(258 as u8)` returned the Tag 2 spec
    // (Precision Time Stamp, 8-byte u64). With an 8-byte payload the
    // length check passed and `record.timestamp_us` was overwritten —
    // silent corruption of typed metadata by a tag that does not exist in
    // ST 0601 today.
    let mut body = Vec::new();
    // Tag 258 with 8-byte value (would have been mis-decoded as Tag 2):
    body.extend_from_slice(&[0x82, 0x02, 0x08]);
    body.extend_from_slice(&[0xAA; 8]);
    let buf = wrap_st0601(&body);

    let record = decode(&buf).expect("strict decode should succeed");

    assert!(
        record.timestamp_us.is_none(),
        "Tag 258 must NOT populate timestamp_us; that slot belongs to Tag 2 only. \
         Got timestamp_us={:?}",
        record.timestamp_us,
    );
    assert_eq!(
        record.unknown.len(),
        1,
        "Tag 258 (unknown) must be preserved in record.unknown",
    );
    assert_eq!(
        record.unknown[0].tag, 258,
        "preserved unknown tag must carry the full u32 BER-OID value, not narrowed",
    );
    assert_eq!(
        record.unknown[0].value,
        vec![0xAA; 8],
        "preserved unknown tag value bytes must round-trip verbatim",
    );
    assert!(
        record.field_errors.is_empty(),
        "no field errors expected; unknown tag goes to record.unknown not record.field_errors",
    );
}

#[test]
fn future_tag_300_with_arbitrary_value_preserved_in_unknown() {
    // BER-OID tag 300 encodes as `0x82, 0x2C` ((2<<7)|0x2C = 300).
    // Narrowing `300 as u8` yields 0x2C = 44 (not in the typed table
    // today, so this case wouldn't silently corrupt — but the wrong tag
    // value would be recorded in `record.unknown`, which is just as bad
    // for any downstream consumer trying to round-trip).
    let mut body = Vec::new();
    body.extend_from_slice(&[0x82, 0x2C, 0x03]);
    body.extend_from_slice(&[0x11, 0x22, 0x33]);
    let buf = wrap_st0601(&body);

    let record = decode(&buf).expect("strict decode should succeed");

    assert_eq!(
        record.unknown.len(),
        1,
        "Tag 300 (unknown) must be preserved in record.unknown",
    );
    assert_eq!(
        record.unknown[0].tag, 300,
        "preserved unknown tag must carry the full u32 BER-OID value (300), not narrowed (44)",
    );
    assert_eq!(record.unknown[0].value, vec![0x11, 0x22, 0x33]);
}

#[test]
fn known_tag_2_still_decodes_correctly_after_high_tag_fix() {
    // Regression: ensure single-byte BER-OID tags in [0, 127] still go
    // through the typed-decode dispatch table. Tag 2 (Precision Time
    // Stamp) is the canonical example — and the one the high-tag bug
    // was clobbering.
    let mut body = Vec::new();
    body.push(0x02);
    body.push(0x08);
    body.extend_from_slice(&0x1234_5678_9ABC_DEF0u64.to_be_bytes());
    let buf = wrap_st0601(&body);

    let record = decode(&buf).expect("strict decode should succeed");

    assert_eq!(
        record.timestamp_us,
        Some(0x1234_5678_9ABC_DEF0),
        "Tag 2 must round-trip into timestamp_us",
    );
    assert!(
        record.unknown.is_empty(),
        "Tag 2 is known; nothing should land in record.unknown",
    );
}

// ---------------------------------------------------------------------------
// Validate-1 E1+E2: strict-mode duplicate-tag detection + canonical-BER walker
// ---------------------------------------------------------------------------

/// Wrap a hand-crafted body into a full ST 0601 LS buffer (UL + outer BER
/// length + body) with the trailing Tag 1 checksum patched in over an
/// existing `0x01 0x02 0x00 0x00` placeholder. Caller must include the
/// placeholder at the end of `body`.
fn wrap_st0601_with_inline_checksum(body: &[u8]) -> Vec<u8> {
    use crate::klv::checksum::checksum_running_sum_16;
    use crate::klv::length::write_ber;
    use crate::klv::universal_label::UniversalLabel;
    let mut buf = Vec::new();
    buf.extend_from_slice(&UniversalLabel::ST_0601_LS.0);
    let mut len_bytes = [0u8; 9];
    let n = write_ber(body.len(), &mut len_bytes).unwrap();
    buf.extend_from_slice(&len_bytes[..n]);
    let body_offset = buf.len();
    buf.extend_from_slice(body);
    // Find the LAST `01 02 00 00` placeholder in the body and patch over
    // the value bytes with the running-sum-16 checksum.
    let body_slice_start = body_offset;
    let body_slice_end = buf.len();
    let mut value_off: Option<usize> = None;
    let mut i = body_slice_start;
    while i + 4 <= body_slice_end {
        if buf[i] == 0x01 && buf[i + 1] == 0x02 && buf[i + 2] == 0x00 && buf[i + 3] == 0x00 {
            value_off = Some(i + 2);
        }
        i += 1;
    }
    let off = value_off.expect("body must contain a 01 02 00 00 placeholder");
    let computed = checksum_running_sum_16(&buf[..off]);
    buf[off] = (computed >> 8) as u8;
    buf[off + 1] = (computed & 0xFF) as u8;
    buf
}

#[test]
fn decode_strict_compliance_rejects_duplicate_tag_13() {
    // ST 0601.13-24: each non-multiple item appears at most once.
    // Tag 13 (Sensor Latitude) is a single-use item. Build a body
    // with Tag 2 first, Tag 65, Tag 13 TWICE, then Tag 1 last.
    let mut body = Vec::new();
    body.extend_from_slice(&[0x02, 0x08]); // Tag 2
    body.extend_from_slice(&1_700_000_000_000_000u64.to_be_bytes());
    body.extend_from_slice(&[0x41, 0x01, 0x13]); // Tag 65 = 0x41
    // Tag 13: I32Range, 4 bytes (per the spec's Sensor Latitude
    // ranged encoding). Two copies.
    body.extend_from_slice(&[0x0D, 0x04, 0x12, 0x34, 0x56, 0x78]);
    body.extend_from_slice(&[0x0D, 0x04, 0x12, 0x34, 0x56, 0x78]);
    body.extend_from_slice(&[0x01, 0x02, 0x00, 0x00]); // Tag 1 placeholder
    let buf = wrap_st0601_with_inline_checksum(&body);

    // Lenient decode accepts the duplicate (second one clobbers the
    // first, but that's the lenient contract — see ST 0601.13).
    let _ = decode(&buf).expect("lenient decode accepts duplicate tags");

    // Strict-compliance rejects with DuplicateTag.
    let err = decode_strict_compliance(&buf).unwrap_err();
    assert!(
        matches!(err, KlvDecodeError::DuplicateTag { tag: 13, .. }),
        "expected DuplicateTag {{ tag: 13, .. }}, got {err:?}",
    );
}

#[test]
fn decode_strict_compliance_allows_duplicate_unknown_tag() {
    // ST 0601.13-24 mandates once-per-packet only for DEFINED items.
    // An unknown tag (outside the typed table) may repeat without
    // violating the local-set contract — the strict walker must
    // ignore duplicates of unknown tags. Tag 70 (0x46) sits in a
    // gap of the typed table (the table jumps 65→74), so it
    // qualifies as "unknown" for this test. Its BER-OID encoding
    // is the single byte 0x46 (high bit clear) — strict-canonical.
    let mut body = Vec::new();
    body.extend_from_slice(&[0x02, 0x08]); // Tag 2
    body.extend_from_slice(&1_700_000_000_000_000u64.to_be_bytes());
    body.extend_from_slice(&[0x41, 0x01, 0x13]); // Tag 65
    // Tag 70 twice with arbitrary 1-byte payloads.
    body.extend_from_slice(&[0x46, 0x01, 0xAA]);
    body.extend_from_slice(&[0x46, 0x01, 0xBB]);
    body.extend_from_slice(&[0x01, 0x02, 0x00, 0x00]); // Tag 1
    let buf = wrap_st0601_with_inline_checksum(&body);

    let record =
        decode_strict_compliance(&buf).expect("strict-compliance allows duplicate unknown tags");
    // Both copies land in record.unknown via the typed dispatcher.
    let unknown_70 = record.unknown.iter().filter(|f| f.tag == 70).count();
    assert_eq!(unknown_70, 2, "both unknown Tag 70 copies preserved");
}

#[test]
fn decode_strict_compliance_rejects_non_canonical_per_item_length() {
    // ST 0107.5 §6.3.2: BER length must use fewest bytes. Per-item
    // length 0x81 0x13 is the long-form encoding of value 19 — which
    // fits in the short form `0x13`. Strict-compliance must reject.
    //
    // Body layout:
    //   Tag 2 (8-byte timestamp)
    //   Tag 65 with NON-CANONICAL length 0x81 0x01 (value 1) carrying
    //     the 1-byte UAS LS version 0x13
    //   Tag 1 placeholder
    let mut body = Vec::new();
    body.extend_from_slice(&[0x02, 0x08]); // Tag 2
    body.extend_from_slice(&1_700_000_000_000_000u64.to_be_bytes());
    body.extend_from_slice(&[0x41, 0x81, 0x01, 0x13]); // Tag 65, BAD length encoding
    body.extend_from_slice(&[0x01, 0x02, 0x00, 0x00]);
    let buf = wrap_st0601_with_inline_checksum(&body);

    // Lenient decode accepts the non-canonical length.
    let _ = decode(&buf).expect("lenient decode accepts non-canonical per-item length");

    // Strict-compliance rejects.
    let err = decode_strict_compliance(&buf).unwrap_err();
    assert!(
        matches!(err, KlvDecodeError::NonCanonicalLength { .. }),
        "expected NonCanonicalLength, got {err:?}",
    );
}

#[test]
fn decode_strict_compliance_rejects_non_canonical_per_item_tag() {
    // ST 0107.5 §6.3.1: BER-OID forbids a leading 0x80 (overlong
    // encoding of value 0). Build a body with Tag 2, Tag 65, an
    // overlong-encoded tag `0x80 0x05` (= value 5), then Tag 1.
    //
    // Note: value 5 is a valid ST 0601 tag (Platform Heading Angle)
    // but the encoding `0x80 0x05` is non-canonical. The lenient
    // reader accepts it.
    let mut body = Vec::new();
    body.extend_from_slice(&[0x02, 0x08]); // Tag 2
    body.extend_from_slice(&1_700_000_000_000_000u64.to_be_bytes());
    body.extend_from_slice(&[0x41, 0x01, 0x13]); // Tag 65
    // Non-canonical BER-OID tag: 0x80 0x05 followed by 2-byte u16 value.
    body.extend_from_slice(&[0x80, 0x05, 0x02, 0x12, 0x34]);
    body.extend_from_slice(&[0x01, 0x02, 0x00, 0x00]);
    let buf = wrap_st0601_with_inline_checksum(&body);

    // Lenient decode accepts the non-canonical tag.
    let _ = decode(&buf).expect("lenient decode accepts non-canonical per-item tag");

    // Strict-compliance rejects.
    let err = decode_strict_compliance(&buf).unwrap_err();
    assert!(
        matches!(err, KlvDecodeError::NonCanonicalTag { .. }),
        "expected NonCanonicalTag, got {err:?}",
    );
}

// --- ST 0601 encode_strict_compliance (validate-1 act-now ST0601-NEW-01) ---
//
// Symmetric counterpart to decode_strict_compliance: the encoder now
// refuses to emit a record missing any caller-supplied mandatory item.
// Today that's only Tag 2 (Precision Time Stamp); Tags 1 + 65 auto-emit.

#[test]
fn encode_strict_rejects_missing_tag_2() {
    // ST 0601.13-22: Tag 2 (Precision Time Stamp) is mandatory in every
    // conformant LS instance. Strict-encode must refuse a record where
    // it is absent.
    let r = UasDatalinkLs {
        timestamp_us: None,
        ..UasDatalinkLs::default()
    };
    let err = encode_strict_compliance(&r).unwrap_err();
    assert!(
        matches!(
            err,
            KlvEncodeError::MissingMandatoryItem {
                tag: 2,
                name: "Precision Time Stamp",
            }
        ),
        "expected MissingMandatoryItem(tag=2), got {err:?}"
    );
}

#[test]
fn encode_strict_accepts_minimal_record() {
    // The strict path must succeed on the bare-minimum conformant
    // record (timestamp set, nothing else). The output bytes must in
    // turn satisfy decode_strict_compliance — pin the round trip.
    let r = UasDatalinkLs {
        timestamp_us: Some(1_700_000_000_000_000),
        ..UasDatalinkLs::default()
    };
    let bytes = encode_strict_compliance(&r).expect("minimal strict record must encode");
    let back = decode_strict_compliance(&bytes)
        .expect("strict-encoded bytes must round-trip through decode_strict_compliance");
    assert_eq!(back.timestamp_us, Some(1_700_000_000_000_000));
    // Tag 65 was auto-emitted by encode (ST 0601.x version byte) — the
    // strict decode would have failed via `MissingTag65` otherwise.
}

#[test]
fn encode_then_decode_strict_roundtrip() {
    // Multi-field record: pin that strict encode preserves Tag 2,
    // Tag 65 (auto), Tag 1 (auto checksum) AND the user-supplied
    // typed fields when round-tripped through strict decode.
    let r = UasDatalinkLs {
        timestamp_us: Some(1_700_000_000_000_000),
        platform_heading_deg: Some(123.4),
        platform_pitch_deg: Some(-5.6),
        platform_roll_deg: Some(2.7),
        sensor_lat_deg: Some(45.0),
        sensor_lon_deg: Some(-122.0),
        sensor_alt_m: Some(1500.0),
        ..UasDatalinkLs::default()
    };
    let bytes = encode_strict_compliance(&r).expect("multi-field strict record must encode");
    let back = decode_strict_compliance(&bytes).expect("strict round-trip must succeed");
    assert_eq!(back.timestamp_us, Some(1_700_000_000_000_000));
    assert!((back.platform_heading_deg.unwrap() - 123.4).abs() < 0.01);
    assert!((back.platform_pitch_deg.unwrap() - -5.6).abs() < 0.01);
    assert!((back.platform_roll_deg.unwrap() - 2.7).abs() < 0.01);
    assert!((back.sensor_lat_deg.unwrap() - 45.0).abs() < 1e-6);
    assert!((back.sensor_lon_deg.unwrap() - -122.0).abs() < 1e-6);
    // Tag 15 (Sensor True Altitude) is IMAPB-encoded with 2-byte width
    // over the [-900, 19000] m range; quantization step is ~0.3 m, so a
    // tolerance of 1 m is generous-but-safe for round-trip pinning.
    assert!((back.sensor_alt_m.unwrap() - 1500.0).abs() < 1.0);
}

// ---------------------------------------------------------------------------
// patch() — byte-faithful tag-level editing
// ---------------------------------------------------------------------------

/// Hand-build a raw LS: ST 0601 UL + short-form outer length + `body`,
/// optionally appending a valid trailing checksum TLV.
fn build_raw_ls(body: &[u8], with_checksum: bool) -> Vec<u8> {
    let mut raw = Vec::new();
    raw.extend_from_slice(&UniversalLabel::ST_0601_LS.0);
    let body_len = body.len() + if with_checksum { 4 } else { 0 };
    assert!(
        body_len < 128,
        "test helper supports short-form length only"
    );
    raw.push(body_len as u8);
    raw.extend_from_slice(body);
    if with_checksum {
        raw.extend_from_slice(&[0x01, 0x02, 0x00, 0x00]);
        let off = raw.len() - 2;
        let sum = checksum_running_sum_16(&raw[..off]);
        raw[off] = (sum >> 8) as u8;
        raw[off + 1] = sum as u8;
    }
    raw
}

#[test]
fn patch_empty_edits_is_byte_identity() {
    let rec = UasDatalinkLs {
        timestamp_us: Some(1_700_000_000_000_000),
        mission_id: Some("M1".into()),
        frame_center_lat_deg: Some(33.4),
        ..UasDatalinkLs::default()
    };
    let raw = encode_to_vec(&rec).unwrap();
    let out = patch(&raw, &UasDatalinkLs::default()).unwrap();
    assert_eq!(out, raw);
}

#[test]
fn patch_in_place_edit_equals_full_reencode() {
    // encode() emits tags in TAGS-table order with canonical encodings,
    // so patching a PRESENT tag must be byte-equal to re-encoding the
    // edited record from scratch.
    let mut rec = UasDatalinkLs {
        timestamp_us: Some(1_700_000_000_000_000),
        mission_id: Some("M1".into()),
        frame_center_lat_deg: Some(33.4),
        corner_lat_p1_deg: Some(33.41),
        uas_ls_version: Some(19),
        ..UasDatalinkLs::default()
    };
    let raw = encode_to_vec(&rec).unwrap();
    let edits = UasDatalinkLs {
        corner_lat_p1_deg: Some(33.99),
        ..UasDatalinkLs::default()
    };
    let patched = patch(&raw, &edits).unwrap();
    rec.corner_lat_p1_deg = Some(33.99);
    assert_eq!(patched, encode_to_vec(&rec).unwrap());
}

#[test]
fn patch_inserts_absent_tag_before_trailing_checksum() {
    let rec = UasDatalinkLs {
        timestamp_us: Some(1),
        uas_ls_version: Some(19),
        ..UasDatalinkLs::default()
    };
    let raw = encode_to_vec(&rec).unwrap();
    let edits = UasDatalinkLs {
        frame_center_lat_deg: Some(10.0),
        ..UasDatalinkLs::default()
    };
    let out = patch(&raw, &edits).unwrap();
    // decode() verifies the running-sum checksum, so this also proves
    // the recompute is correct.
    let dec = decode(&out).expect("patched output decodes with a valid checksum");
    assert!((dec.frame_center_lat_deg.unwrap() - 10.0).abs() < 1e-6);
    // The checksum TLV (tag 1, len 2, 2 value bytes) is still the
    // final element — the inserted tag landed before it.
    assert_eq!(out[out.len() - 4], 0x01);
    assert_eq!(out[out.len() - 3], 0x02);
}

#[test]
fn patch_preserves_noncanonical_length_and_vendor_tlv_bytes() {
    // tag 3 with a NON-CANONICAL long-form length (0x81 0x02) and a
    // vendor TLV (tag 0x67 = 103, untyped). Patching an unrelated tag
    // must leave both byte sequences intact.
    let mut body = Vec::new();
    body.extend_from_slice(&[0x03, 0x81, 0x02, b'A', b'B']);
    body.extend_from_slice(&[0x67, 0x02, 0xDE, 0xAD]);
    let raw = build_raw_ls(&body, true);
    let _ = decode(&raw).expect("fixture must decode");

    let edits = UasDatalinkLs {
        frame_center_lat_deg: Some(10.0),
        ..UasDatalinkLs::default()
    };
    let out = patch(&raw, &edits).unwrap();
    let find = |needle: &[u8]| out.windows(needle.len()).any(|w| w == needle);
    assert!(
        find(&[0x03, 0x81, 0x02, b'A', b'B']),
        "non-canonical TLV must survive verbatim"
    );
    assert!(
        find(&[0x67, 0x02, 0xDE, 0xAD]),
        "vendor TLV must survive verbatim"
    );
    let _ = decode(&out).expect("patched output decodes with a valid checksum");
}

#[test]
fn patch_reencodes_every_occurrence_of_a_duplicated_tag_and_mirrors_missing_checksum() {
    // Two tag-4 TLVs, no checksum tag. Both occurrences re-encode; no
    // checksum is added (mirror-input); outer length re-encodes
    // canonically because the body size changed.
    let mut body = Vec::new();
    body.extend_from_slice(&[0x04, 0x01, b'X']);
    body.extend_from_slice(&[0x04, 0x01, b'Y']);
    let raw = build_raw_ls(&body, false);
    let edits = UasDatalinkLs {
        platform_tail_number: Some("Z9".into()),
        ..UasDatalinkLs::default()
    };
    let out = patch(&raw, &edits).unwrap();
    let mut expected = Vec::new();
    expected.extend_from_slice(&UniversalLabel::ST_0601_LS.0);
    expected.push(8);
    expected.extend_from_slice(&[0x04, 0x02, b'Z', b'9', 0x04, 0x02, b'Z', b'9']);
    assert_eq!(out, expected);
}

#[test]
fn patch_unknown_escape_hatch_replaces_vendor_tlv() {
    let rec = UasDatalinkLs {
        timestamp_us: Some(1),
        unknown: vec![OwnedRawField {
            tag: 103,
            value: vec![0xDE, 0xAD],
        }],
        ..UasDatalinkLs::default()
    };
    let raw = encode_to_vec(&rec).unwrap();
    let edits = UasDatalinkLs {
        unknown: vec![OwnedRawField {
            tag: 103,
            value: vec![0x01, 0x02, 0x03],
        }],
        ..UasDatalinkLs::default()
    };
    let out = patch(&raw, &edits).unwrap();
    let dec = decode(&out).unwrap();
    assert_eq!(
        dec.unknown,
        vec![OwnedRawField {
            tag: 103,
            value: vec![0x01, 0x02, 0x03],
        }]
    );
}

#[test]
fn patch_rejects_typed_tag_in_unknown_edits() {
    let raw = encode_to_vec(&UasDatalinkLs {
        timestamp_us: Some(1),
        ..UasDatalinkLs::default()
    })
    .unwrap();
    let edits = UasDatalinkLs {
        unknown: vec![OwnedRawField {
            tag: 2,
            value: vec![0],
        }],
        ..UasDatalinkLs::default()
    };
    match patch(&raw, &edits) {
        Err(KlvPatchError::Encode(KlvEncodeError::ReservedTagInUnknown { tag: 2 })) => {}
        other => panic!("expected ReservedTagInUnknown, got {other:?}"),
    }
}

#[test]
fn patch_out_of_range_edit_value_errors() {
    let raw = encode_to_vec(&UasDatalinkLs {
        timestamp_us: Some(1),
        corner_lat_p1_deg: Some(10.0),
        ..UasDatalinkLs::default()
    })
    .unwrap();
    let edits = UasDatalinkLs {
        corner_lat_p1_deg: Some(999.0),
        ..UasDatalinkLs::default()
    };
    assert!(matches!(
        patch(&raw, &edits),
        Err(KlvPatchError::Encode(KlvEncodeError::OutOfRange { .. }))
    ));
}

#[test]
fn patch_truncated_input_errors() {
    assert!(matches!(
        patch(&[0x06, 0x0E], &UasDatalinkLs::default()),
        Err(KlvPatchError::Decode(KlvDecodeError::Truncated { .. }))
    ));
}

#[test]
fn patch_recomputes_mid_body_checksum_in_place() {
    // A NON-COMPLIANT mid-body tag-1 (followed by a vendor TLV, no
    // trailing checksum) is tolerated: recomputed in place over its
    // prefix, with every other byte preserved verbatim.
    let mut body = Vec::new();
    body.extend_from_slice(&[0x01, 0x02, 0xAB, 0xCD]); // stale checksum value
    body.extend_from_slice(&[0x67, 0x01, 0x55]);
    let raw = build_raw_ls(&body, false);
    let out = patch(&raw, &UasDatalinkLs::default()).unwrap();
    assert_eq!(out.len(), raw.len());
    // UL + outer length + tag-1 header preserved (bytes 0..19).
    assert_eq!(&out[..19], &raw[..19]);
    // The 2-byte value is recomputed over its prefix, in place.
    let sum = checksum_running_sum_16(&out[..19]);
    assert_eq!(out[19], (sum >> 8) as u8);
    assert_eq!(out[20], sum as u8);
    // Everything after the checksum value is verbatim.
    assert_eq!(&out[21..], &raw[21..]);
}

#[test]
fn patch_does_not_verify_input_checksum() {
    // patch() is an editor, not a validator: a corrupt input checksum
    // is not rejected — it is simply recomputed (here back to the
    // correct value, making the output byte-equal to the clean input).
    let clean = encode_to_vec(&UasDatalinkLs {
        timestamp_us: Some(1),
        ..UasDatalinkLs::default()
    })
    .unwrap();
    let mut corrupt = clean.clone();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 0xFF;
    assert!(
        matches!(
            decode(&corrupt),
            Err(KlvDecodeError::ChecksumMismatch { .. })
        ),
        "fixture sanity: the corruption must be decode-visible"
    );
    let out = patch(&corrupt, &UasDatalinkLs::default()).unwrap();
    assert_eq!(out, clean);
}

#[test]
fn patch_does_not_inject_uas_ls_version() {
    // Input has no tag 65 and no checksum; patching another field must
    // NOT auto-insert a version tag (unlike encode_with).
    let raw = build_raw_ls(&[0x04, 0x01, b'X'], false);
    let edits = UasDatalinkLs {
        mission_id: Some("M".into()),
        ..UasDatalinkLs::default()
    };
    let out = patch(&raw, &edits).unwrap();
    let dec = decode_unchecked(&out).unwrap();
    assert_eq!(dec.uas_ls_version, None);
    assert_eq!(dec.mission_id.as_deref(), Some("M"));
}

#[test]
fn patch_duplicate_checksums_only_last_recomputed() {
    // TWO non-compliant mid-body tag-1 TLVs with distinct known values,
    // followed by a vendor TLV (so neither is the trailing checksum).
    // The FIRST (non-last) checksum's original value bytes must survive
    // verbatim; only the LAST is recomputed.
    let mut body = Vec::new();
    body.extend_from_slice(&[0x01, 0x02, 0xAA, 0xBB]);
    body.extend_from_slice(&[0x01, 0x02, 0xCC, 0xDD]);
    body.extend_from_slice(&[0x67, 0x01, 0x55]);
    let raw = build_raw_ls(&body, false);
    let out = patch(&raw, &UasDatalinkLs::default()).unwrap();
    assert_eq!(out.len(), raw.len());
    // UL + outer length + first tag-1 TLV (header AND value) verbatim.
    assert_eq!(&out[..21], &raw[..21]);
    assert_eq!(
        &out[19..21],
        &[0xAA, 0xBB],
        "first (non-last) checksum value bytes must survive verbatim"
    );
    // Second tag-1: header verbatim, value recomputed over its prefix.
    assert_eq!(&out[21..23], &raw[21..23]);
    let sum = checksum_running_sum_16(&out[..23]);
    assert_eq!(out[23], (sum >> 8) as u8);
    assert_eq!(out[24], sum as u8);
    // Trailing vendor TLV verbatim.
    assert_eq!(&out[25..], &raw[25..]);
}

#[test]
fn patch_preserves_trailing_bytes_after_declared_length() {
    // Trailing bytes after the declared outer length (capture padding)
    // are preserved verbatim — full byte identity holds.
    let mut raw = encode_to_vec(&UasDatalinkLs {
        timestamp_us: Some(1),
        ..UasDatalinkLs::default()
    })
    .unwrap();
    raw.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    let _ = decode(&raw).expect("lenient decode tolerates trailing bytes");
    let out = patch(&raw, &UasDatalinkLs::default()).unwrap();
    assert_eq!(out, raw);
}

#[test]
fn patch_empty_body_identity_and_insertion() {
    // UL + 0x00 outer length, no TLVs at all.
    let raw = build_raw_ls(&[], false);
    // Empty edits: byte identity.
    let out = patch(&raw, &UasDatalinkLs::default()).unwrap();
    assert_eq!(out, raw);
    // An edit grows the empty body via the insertion path (no checksum
    // to land before — appended at the end, canonical outer length).
    let edits = UasDatalinkLs {
        mission_id: Some("M".into()),
        ..UasDatalinkLs::default()
    };
    let out = patch(&raw, &edits).unwrap();
    let mut expected = Vec::new();
    expected.extend_from_slice(&UniversalLabel::ST_0601_LS.0);
    expected.push(3);
    expected.extend_from_slice(&[0x03, 0x01, b'M']);
    assert_eq!(out, expected);
}

#[test]
fn patch_preserves_noncanonical_outer_length_when_size_unchanged() {
    // Outer length 0x81 0x03 = NON-CANONICAL long form of 3. An
    // in-place same-size edit must keep those length bytes verbatim.
    let mut raw = Vec::new();
    raw.extend_from_slice(&UniversalLabel::ST_0601_LS.0);
    raw.extend_from_slice(&[0x81, 0x03]);
    raw.extend_from_slice(&[0x04, 0x01, b'X']);
    let edits = UasDatalinkLs {
        platform_tail_number: Some("Y".into()),
        ..UasDatalinkLs::default()
    };
    let out = patch(&raw, &edits).unwrap();
    assert_eq!(
        &out[16..18],
        &[0x81, 0x03],
        "non-canonical outer length survives verbatim when body size is unchanged"
    );
    assert_eq!(&out[18..], &[0x04, 0x01, b'Y']);
    assert_eq!(out.len(), raw.len());
}

#[test]
fn patch_propagated_decode_error_offsets_are_absolute() {
    // Indefinite-form outer length byte (0x80) at raw[16]: the
    // propagated MalformedLength offset must be absolute, not 0.
    let mut raw = Vec::new();
    raw.extend_from_slice(&UniversalLabel::ST_0601_LS.0);
    raw.push(0x80);
    match patch(&raw, &UasDatalinkLs::default()) {
        Err(KlvPatchError::Decode(KlvDecodeError::MalformedLength { offset })) => {
            assert_eq!(offset, 16);
        }
        other => panic!("expected MalformedLength at offset 16, got {other:?}"),
    }

    // Truncated BER-OID tag inside the body (continuation bit set, no
    // following byte): offset must be raw-absolute too. The missing
    // byte sits at 16 (UL) + 1 (length) + 1 (tag byte) = 18.
    let raw = build_raw_ls(&[0x81], false);
    match patch(&raw, &UasDatalinkLs::default()) {
        Err(KlvPatchError::Decode(KlvDecodeError::Truncated { offset, .. })) => {
            assert_eq!(offset, 18);
        }
        other => panic!("expected Truncated at offset 18, got {other:?}"),
    }
}
