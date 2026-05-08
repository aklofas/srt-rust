//! Standalone-PID consumer pattern: VMTI as its own KLV record with
//! 16-byte UL prefix + BER outer length + LS body.
//!
//! Mirrors the docstring example in `klv::st0903::mod.rs`. Verifies
//! that consumers carrying VMTI on its own KLV PID (separate from any
//! ST 0601 stream) can dispatch via the `VMTI_LS_UL` constant without
//! the demuxer needing to be VMTI-aware.

use tst_core::klv::{length, st0903};

#[test]
fn standalone_ul_dispatch_pattern() {
    // Build a VMTI LS body.
    let vmti = st0903::VmtiLs {
        checksum: Some(0xDEAD),
        precision_time_stamp: Some(1_700_000_000_000_000),
        version_number: Some(6),
        num_targets_reported: Some(1),
        targets: vec![st0903::VTargetPack {
            target_id: 42,
            centroid_pixel: Some(1000),
            ..Default::default()
        }],
        ..Default::default()
    };
    let body = st0903::encode_to_vec(&vmti).unwrap();

    // Wrap in UL + BER outer length.
    let mut record = Vec::new();
    record.extend_from_slice(&st0903::VMTI_LS_UL);
    let mut len_buf = [0u8; 9];
    let len_n = length::write_ber(body.len(), &mut len_buf).unwrap();
    record.extend_from_slice(&len_buf[..len_n]);
    record.extend_from_slice(&body);

    // Consumer-side dispatch (the documented pattern from
    // klv::st0903's module rustdoc).
    assert!(
        record.starts_with(&st0903::VMTI_LS_UL),
        "record should start with the 16-byte VMTI_LS_UL"
    );
    let after_ul = &record[16..];
    let (declared_len, after_len) = length::read_ber(after_ul).unwrap();
    let inner = &after_len[..declared_len];

    // Decode the inner LS body via klv::st0903::decode.
    let decoded = st0903::decode(inner).unwrap();
    assert_eq!(decoded.checksum, Some(0xDEAD));
    assert_eq!(decoded.precision_time_stamp, Some(1_700_000_000_000_000));
    assert_eq!(decoded.targets.len(), 1);
    assert_eq!(decoded.targets[0].target_id, 42);
    assert_eq!(decoded.targets[0].centroid_pixel, Some(1000));
}

#[test]
fn vmti_ul_constant_matches_misb_st0903_6_section_6_1() {
    // Sanity check the UL bytes — the 16-byte ULs are easy to typo.
    // First 4 bytes are SMPTE 336M-2007 prefix (06 0E 2B 34); remaining
    // 12 bytes encode the MISB ST 0903 designator. Verify the prefix
    // and the well-known last byte (0x06 = VMTI item) per ST 0903.6.
    assert_eq!(
        &st0903::VMTI_LS_UL[..4],
        &[0x06, 0x0E, 0x2B, 0x34],
        "SMPTE 336M-2007 prefix"
    );
    assert_eq!(
        st0903::VMTI_LS_UL[12],
        0x06,
        "VMTI = item 0x06 in the org/dict per ST 0903.6 §6.1"
    );
}

#[test]
fn standalone_ul_with_strict_decode() {
    // Test that the standalone-PID pattern works with decode_strict
    // when the LS body satisfies strict requirements (Tags 4 + 6).
    let vmti = st0903::VmtiLs {
        version_number: Some(6),
        num_targets_reported: Some(0),
        ..Default::default()
    };
    let body = st0903::encode_to_vec(&vmti).unwrap();

    let mut record = Vec::new();
    record.extend_from_slice(&st0903::VMTI_LS_UL);
    let mut len_buf = [0u8; 9];
    let len_n = length::write_ber(body.len(), &mut len_buf).unwrap();
    record.extend_from_slice(&len_buf[..len_n]);
    record.extend_from_slice(&body);

    // Strict-mode dispatch via the standalone pattern.
    assert!(record.starts_with(&st0903::VMTI_LS_UL));
    let after_ul = &record[16..];
    let (declared_len, after_len) = length::read_ber(after_ul).unwrap();
    let inner = &after_len[..declared_len];
    let decoded = st0903::decode_strict(inner).unwrap();
    assert_eq!(decoded.version_number, Some(6));
    assert_eq!(decoded.num_targets_reported, Some(0));
}
