//! Sibling-layer integration: VMTI typed decode dispatched from a
//! Tag-74-bearing ST 0601 record.
//!
//! Verifies that the parent ST 0601 layer stays out of the way — Tag 74
//! is not in `UasDatalinkLs`'s typed schema, so it round-trips verbatim
//! through `unknown: Vec<OwnedRawField>` — and that consumers can
//! compose the two layers exactly as the doc example shows.
//!
//! Note on field placement: today's `UasDatalinkLs` has no dedicated
//! `vmti: Option<Vec<u8>>` field for Tag 74. Tag 74 takes the
//! pass-through path that any non-typed tag uses: the encoder writes
//! the BER-OID tag + BER length + value verbatim from the `unknown`
//! collection, and the decoder pushes anything not in its tag table
//! straight back into `unknown`. The sibling-layer dispatch principle
//! is unchanged — the parent does not recurse, the consumer composes.

use tst_core::klv::{OwnedRawField, st0601, st0903};

const VMTI_TAG_IN_ST0601: u32 = 74;

/// Find the inner VMTI bytes carried under ST 0601 Tag 74.
fn find_vmti_in_unknown(record: &st0601::UasDatalinkLs) -> Option<&[u8]> {
    record
        .unknown
        .iter()
        .find(|f| f.tag == VMTI_TAG_IN_ST0601)
        .map(|f| f.value.as_slice())
}

#[test]
fn vmti_round_trips_through_st0601_tag_74() {
    // Build a synthetic VMTI LS.
    let vmti = st0903::VmtiLs {
        checksum: Some(0),
        precision_time_stamp: Some(1_700_000_000_000_000),
        version_number: Some(6),
        num_targets_reported: Some(2),
        frame_width: Some(3840),
        frame_height: Some(2160),
        horizontal_fov: Some(45.0),
        vertical_fov: Some(30.0),
        targets: vec![
            st0903::VTargetPack {
                target_id: 1,
                centroid_pixel: Some(8_294_400),
                priority: Some(1),
                confidence_level: Some(95),
                target_color: Some([0xFF, 0x00, 0x00]),
                ..Default::default()
            },
            st0903::VTargetPack {
                target_id: 2,
                centroid_pixel: Some(4_147_200),
                priority: Some(2),
                confidence_level: Some(80),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let vmti_bytes = st0903::encode_to_vec(&vmti).expect("st0903 encode succeeds");

    // Build the parent ST 0601 record. Tag 74 = VMTI LS, carried as a
    // pass-through unknown field (the ST 0601 typed schema doesn't model it).
    let parent = st0601::UasDatalinkLs {
        timestamp_us: Some(1_700_000_000_000_000),
        platform_designation: Some("Test-Platform".to_string()),
        unknown: vec![OwnedRawField {
            tag: VMTI_TAG_IN_ST0601,
            value: vmti_bytes.clone(),
        }],
        ..Default::default()
    };
    let parent_bytes = st0601::encode_to_vec(&parent).expect("st0601 encode succeeds");

    // Round-trip the parent.
    let decoded_parent = st0601::decode(&parent_bytes).expect("st0601 decode succeeds");
    assert_eq!(
        decoded_parent.platform_designation.as_deref(),
        Some("Test-Platform")
    );
    let inner = find_vmti_in_unknown(&decoded_parent).expect("Tag 74 present after decode");
    assert_eq!(
        inner,
        vmti_bytes.as_slice(),
        "ST 0601 Tag 74 round-trip should preserve bytes verbatim"
    );

    // Dispatch to klv::st0903::decode.
    let decoded_vmti = st0903::decode(inner).expect("st0903 decode succeeds");
    assert_eq!(decoded_vmti.frame_width, Some(3840));
    assert_eq!(decoded_vmti.frame_height, Some(2160));
    assert_eq!(decoded_vmti.targets.len(), 2);
    assert_eq!(decoded_vmti.targets[0].target_id, 1);
    assert_eq!(decoded_vmti.targets[0].confidence_level, Some(95));
    assert_eq!(
        decoded_vmti.targets[0].target_color,
        Some([0xFF, 0x00, 0x00])
    );
    assert_eq!(decoded_vmti.targets[1].target_id, 2);
    assert_eq!(decoded_vmti.targets[1].confidence_level, Some(80));
}

#[test]
fn vmti_pass_through_preserves_bytes_verbatim() {
    // Even malformed VMTI in Tag 74 should pass through ST 0601
    // unchanged — the parent decoder doesn't recurse.
    let garbage = vec![0xDEu8, 0xAD, 0xBE, 0xEF];
    let parent = st0601::UasDatalinkLs {
        timestamp_us: Some(1_700_000_000_000_000),
        unknown: vec![OwnedRawField {
            tag: VMTI_TAG_IN_ST0601,
            value: garbage.clone(),
        }],
        ..Default::default()
    };
    let parent_bytes = st0601::encode_to_vec(&parent).expect("st0601 encode succeeds");

    let decoded = st0601::decode(&parent_bytes).expect("st0601 decode succeeds");
    assert_eq!(
        find_vmti_in_unknown(&decoded),
        Some(garbage.as_slice()),
        "ST 0601 should not recurse into Tag 74"
    );

    // Lenient st0903::decode on garbage doesn't panic; accumulates
    // field_errors. Don't assert on exact field_errors content — just
    // that decode is panic-free.
    let _ = st0903::decode(&garbage);
}

#[test]
fn vmti_round_trip_via_strict_decode() {
    // Test that decode_strict also works through the sibling pattern.
    // Build a minimal VMTI LS with the strict-required tags satisfied.
    let vmti = st0903::VmtiLs {
        version_number: Some(6),
        num_targets_reported: Some(0),
        ..Default::default()
    };
    let vmti_bytes = st0903::encode_to_vec(&vmti).expect("st0903 encode succeeds");

    let parent = st0601::UasDatalinkLs {
        timestamp_us: Some(1_700_000_000_000_000),
        unknown: vec![OwnedRawField {
            tag: VMTI_TAG_IN_ST0601,
            value: vmti_bytes,
        }],
        ..Default::default()
    };
    let parent_bytes = st0601::encode_to_vec(&parent).expect("st0601 encode succeeds");

    let decoded_parent = st0601::decode(&parent_bytes).expect("st0601 decode succeeds");
    let inner = find_vmti_in_unknown(&decoded_parent).expect("Tag 74 present");
    let decoded_vmti = st0903::decode_strict(inner).expect("strict decode succeeds");
    assert_eq!(decoded_vmti.version_number, Some(6));
    assert_eq!(decoded_vmti.num_targets_reported, Some(0));
}
