//! Sibling-layer integration: VMTI typed decode dispatched from a
//! Tag-74-bearing ST 0601 record.
//!
//! Verifies that the parent ST 0601 layer stays out of the way — Tag 74
//! is carried on the typed `vmti: Option<Vec<u8>>` pass-through field,
//! and the parent decoder does not recurse into the VMTI inner schema.
//! Consumers compose `klv::st0903::decode` themselves, exactly as the
//! `security_local_set` (Tag 48 → ST 0102) sibling-layer pattern does.

use tst_core::klv::{st0601, st0903};

#[test]
fn vmti_round_trips_through_st0601_tag_74() {
    // Build a synthetic VMTI LS. `checksum` intentionally not set —
    // Tag 74 embeds the body and ST 0903.6-120 forbids Tag 1 in the
    // embedded-VMTI form (it would be silently dropped anyway).
    let vmti = st0903::VmtiLs {
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

    // Build the parent ST 0601 record. Tag 74 = VMTI LS, carried on
    // the typed `vmti` field — the parent does not recurse.
    let parent = st0601::UasDatalinkLs {
        timestamp_us: Some(1_700_000_000_000_000),
        platform_designation: Some("Test-Platform".to_string()),
        vmti: Some(vmti_bytes.clone()),
        ..Default::default()
    };
    let parent_bytes = st0601::encode_to_vec(&parent).expect("st0601 encode succeeds");

    // Round-trip the parent.
    let decoded_parent = st0601::decode(&parent_bytes).expect("st0601 decode succeeds");
    assert_eq!(
        decoded_parent.platform_designation.as_deref(),
        Some("Test-Platform")
    );
    let inner = decoded_parent
        .vmti
        .as_deref()
        .expect("Tag 74 present after decode");
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
        vmti: Some(garbage.clone()),
        ..Default::default()
    };
    let parent_bytes = st0601::encode_to_vec(&parent).expect("st0601 encode succeeds");

    let decoded = st0601::decode(&parent_bytes).expect("st0601 decode succeeds");
    assert_eq!(
        decoded.vmti.as_deref(),
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
        vmti: Some(vmti_bytes),
        ..Default::default()
    };
    let parent_bytes = st0601::encode_to_vec(&parent).expect("st0601 encode succeeds");

    let decoded_parent = st0601::decode(&parent_bytes).expect("st0601 decode succeeds");
    let inner = decoded_parent.vmti.as_deref().expect("Tag 74 present");
    let decoded_vmti = st0903::decode_strict(inner).expect("strict decode succeeds");
    assert_eq!(decoded_vmti.version_number, Some(6));
    assert_eq!(decoded_vmti.num_targets_reported, Some(0));
}
