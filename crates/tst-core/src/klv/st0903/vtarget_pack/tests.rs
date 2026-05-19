use super::decode::read_pack;
use super::encode::{encoded_len, write_pack};
use super::model::{PACK_TAGS, VTargetPack, VTargetPackError, pack_lookup};
use crate::klv::pack::OwnedRawField;

#[test]
fn pack_tags_table_has_unique_ids() {
    let mut ids: Vec<u8> = PACK_TAGS.iter().map(|t| t.id).collect();
    ids.sort();
    let len_before = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), len_before, "duplicate pack tag IDs");
}

#[test]
fn pack_tags_lookup_round_trips() {
    for tag in PACK_TAGS {
        assert_eq!(pack_lookup(tag.id), Some(tag));
    }
    assert_eq!(pack_lookup(0), None);
    assert_eq!(pack_lookup(255), None);
    // Deprecated tags per ST 0903.6 §10.2.2.22, §10.2.2.26,
    // §10.2.2.27 — intentionally absent from the table; lenient
    // decoders must treat any wire occurrence as an unknown tag
    // (preserved per ST 0107.5 §6).
    assert_eq!(pack_lookup(21), None);
    assert_eq!(pack_lookup(102), None);
    assert_eq!(pack_lookup(103), None);
}

#[test]
fn empty_pack_round_trips() {
    let pack = VTargetPack {
        target_id: 1,
        ..Default::default()
    };
    let mut bytes = Vec::new();
    let written = write_pack(&pack, &mut bytes).unwrap();
    assert_eq!(written, bytes.len());
    assert_eq!(written, encoded_len(&pack));
    let (decoded, consumed) = read_pack(&bytes).unwrap();
    assert_eq!(consumed, bytes.len());
    assert_eq!(decoded.target_id, 1);
    // Default fields should round-trip as None / empty.
    assert!(decoded.centroid_pixel.is_none());
    assert!(decoded.target_color.is_none());
    assert!(decoded.unknown.is_empty());
    assert!(decoded.field_errors.is_empty());
}

#[test]
fn populated_pack_round_trips() {
    let pack = VTargetPack {
        target_id: 42,
        centroid_pixel: Some(8_294_400),
        bbox_top_left_pixel: Some(8_293_000),
        bbox_bottom_right_pixel: Some(8_295_800),
        priority: Some(1),
        confidence_level: Some(95),
        history: Some(0),
        percentage_of_target_pixels: Some(60),
        target_color: Some([0xFF, 0x80, 0x40]),
        target_intensity: Some(220),
        centroid_lat_offset: Some(0.001234),
        centroid_lon_offset: Some(-0.005678),
        centroid_hae: Some(150.0),
        bbox_top_left_lat_offset: Some(0.000123),
        bbox_top_left_lon_offset: Some(-0.000456),
        bbox_bottom_right_lat_offset: Some(0.000789),
        bbox_bottom_right_lon_offset: Some(-0.001234),
        target_location: Some(vec![0xAA, 0xBB]),
        geospatial_contour_series: Some(vec![0xCC, 0xDD]),
        centroid_pix_row: Some(1080),
        centroid_pix_col: Some(1920),
        algorithm_id: Some(7),
        detection_status: Some(1),
        vmask: Some(vec![0xDE, 0xAD]),
        vtracker: Some(vec![0x42]),
        vchip: None,
        vchip_series: Some(vec![0x01, 0x02]),
        vobject_series: Some(vec![0x03, 0x04]),
        unknown: vec![],
        field_errors: vec![],
    };
    let bytes = {
        let mut b = Vec::new();
        write_pack(&pack, &mut b).unwrap();
        b
    };
    let (decoded, _consumed) = read_pack(&bytes).unwrap();
    assert_eq!(decoded.target_id, 42);
    assert_eq!(decoded.centroid_pixel, Some(8_294_400));
    assert_eq!(decoded.priority, Some(1));
    assert_eq!(decoded.target_color, Some([0xFF, 0x80, 0x40]));
    assert_eq!(decoded.vmask, Some(vec![0xDE, 0xAD]));
    assert_eq!(decoded.detection_status, Some(1));
    assert_eq!(decoded.algorithm_id, Some(7));
    assert!((decoded.centroid_lat_offset.unwrap() - 0.001234).abs() < 1e-5);
    assert!((decoded.centroid_lon_offset.unwrap() - (-0.005678)).abs() < 1e-5);
    assert!((decoded.bbox_top_left_lat_offset.unwrap() - 0.000123).abs() < 1e-5);
    assert!((decoded.bbox_bottom_right_lon_offset.unwrap() - (-0.001234)).abs() < 1e-5);
}

#[test]
fn target_id_multibyte_round_trips() {
    // Target ID = 200 fits in 2 BER-OID bytes (0x81 0x48).
    let pack = VTargetPack {
        target_id: 200,
        ..Default::default()
    };
    let mut bytes = Vec::new();
    write_pack(&pack, &mut bytes).unwrap();
    assert_eq!(bytes[0], 0x81);
    assert_eq!(bytes[1], 0x48);
    let (decoded, _) = read_pack(&bytes).unwrap();
    assert_eq!(decoded.target_id, 200);
}

#[test]
fn truncated_target_id_rejected() {
    // 0x81 alone signals "more bytes follow" but buffer is empty.
    let bytes = [0x81u8];
    let err = read_pack(&bytes).unwrap_err();
    assert!(matches!(err, VTargetPackError::TruncatedTargetId));
}

#[test]
fn truncated_field_value_rejected() {
    // Target ID = 1 (1 byte 0x01), then tag 6 (Target History)
    // declares length 2 but provides 1 byte.
    let bytes = [0x01, 6, 2, 0xFF];
    let err = read_pack(&bytes).unwrap_err();
    assert!(matches!(
        err,
        VTargetPackError::LengthOverrun { tag: 6, .. }
    ));
}

#[test]
fn unknown_tags_preserved() {
    // Build by hand: target_id=1, then unknown tag 200 with 3 bytes
    // [0xAA 0xBB 0xCC].
    let bytes = [0x01u8, 200, 3, 0xAA, 0xBB, 0xCC];
    let (decoded, _) = read_pack(&bytes).unwrap();
    assert_eq!(decoded.target_id, 1);
    assert_eq!(decoded.unknown.len(), 1);
    assert_eq!(decoded.unknown[0].tag, 200);
    assert_eq!(decoded.unknown[0].value, vec![0xAA, 0xBB, 0xCC]);
}

#[test]
fn deprecated_tag_preserved_as_unknown() {
    // Per ST 0107.5 §6, deprecated tag IDs (e.g., 21) should round-
    // trip as unknown bytes — the decoder doesn't reject them, just
    // treats them as opaque.
    let bytes = [0x01u8, 21, 2, 0xDE, 0xAD];
    let (decoded, _) = read_pack(&bytes).unwrap();
    assert_eq!(decoded.unknown.len(), 1);
    assert_eq!(decoded.unknown[0].tag, 21);
    assert_eq!(decoded.unknown[0].value, vec![0xDE, 0xAD]);
}

/// Locks in the canonical wire format of a known VTargetPack.
/// Catches accidental field-order changes in `write_pack` (which
/// round-trip tests miss because `read_pack` is order-agnostic) and
/// catches `encoded_len` drift relative to `write_pack`.
#[test]
fn write_pack_canonical_byte_layout() {
    let pack = VTargetPack {
        target_id: 7,
        centroid_pixel: Some(0x1234), // Tag 1, V6 → 2 bytes [0x12, 0x34]
        priority: Some(2),            // Tag 4, U8
        confidence_level: Some(95),   // Tag 5, U8
        target_color: Some([0xAA, 0xBB, 0xCC]), // Tag 8, 3-byte RGB
        detection_status: Some(1),    // Tag 23, U8
        vmask: Some(vec![0xDE, 0xAD]), // Tag 101
        ..Default::default()
    };

    let mut bytes = Vec::new();
    let written = write_pack(&pack, &mut bytes).unwrap();

    // Expected wire form (BER-OID Target ID + ascending-tag TLVs):
    let expected: Vec<u8> = vec![
        0x07, // BER-OID Target ID = 7
        // Tag 1, len 2, value [0x12, 0x34] (centroid_pixel)
        0x01, 0x02, 0x12, 0x34, // Tag 4, len 1, value [0x02] (priority)
        0x04, 0x01, 0x02, // Tag 5, len 1, value [0x5F] (confidence_level = 95 = 0x5F)
        0x05, 0x01, 0x5F, // Tag 8, len 3, value [0xAA, 0xBB, 0xCC] (target_color)
        0x08, 0x03, 0xAA, 0xBB, 0xCC, // Tag 23, len 1, value [0x01] (detection_status)
        0x17, 0x01, 0x01, // Tag 101, len 2, value [0xDE, 0xAD] (vmask)
        0x65, 0x02, 0xDE, 0xAD,
    ];
    assert_eq!(
        bytes, expected,
        "write_pack produced unexpected byte layout — \
        either field-order changed or a TLV got bogus bytes"
    );

    assert_eq!(written, bytes.len());
    assert_eq!(
        written,
        encoded_len(&pack),
        "write_pack length disagrees with encoded_len — drift between the two functions"
    );
}

#[test]
fn round_trip_with_unknown_preserved() {
    let mut pack = VTargetPack {
        target_id: 5,
        priority: Some(3),
        ..Default::default()
    };
    pack.unknown.push(OwnedRawField {
        tag: 200,
        value: vec![0xFF, 0xEE],
    });
    let mut bytes = Vec::new();
    write_pack(&pack, &mut bytes).unwrap();
    let (decoded, _) = read_pack(&bytes).unwrap();
    assert_eq!(decoded.target_id, 5);
    assert_eq!(decoded.priority, Some(3));
    assert_eq!(decoded.unknown.len(), 1);
    assert_eq!(decoded.unknown[0].tag, 200);
}
