//! Sibling-layer composition test: ST 0102 inside ST 0601 Tag 48.
//!
//! Verifies that:
//! 1. A typed `SecurityLs` encodes to bytes via `klv::st0102::encode_to_vec`.
//! 2. Those bytes wrap into `UasDatalinkLs.security_local_set` as
//!    `Option<Vec<u8>>` (the existing pass-through field).
//! 3. The parent ST 0601 record encodes + decodes losslessly.
//! 4. The decoded inner bytes feed back into `klv::st0102::decode`
//!    yielding a `SecurityLs` equal to the original.

use tst_core::klv::UniversalLabel;
use tst_core::klv::st0102::{
    self, ClassifyingCountryCodingMethod, ObjectCountryCodingMethod, SecurityClassification,
    SecurityLs,
};
use tst_core::klv::st0601::{self, UasDatalinkLs};

fn build_security_ls() -> SecurityLs {
    SecurityLs {
        security_classification: Some(SecurityClassification::Secret),
        classifying_country_coding_method: Some(ClassifyingCountryCodingMethod::Iso3166ThreeLetter),
        classifying_country: Some("//USA".to_string()),
        sci_shi_info: Some("HCS-O".to_string()),
        caveats: Some("FOUO".to_string()),
        releasing_instructions: Some("USA CAN GBR".to_string()),
        object_country_coding_method: Some(ObjectCountryCodingMethod::Iso3166ThreeLetter),
        object_country_codes: Some("USA".to_string()),
        version: Some(12),
        ..Default::default()
    }
}

fn build_parent_uas_ls(security_bytes: Vec<u8>) -> UasDatalinkLs {
    UasDatalinkLs {
        universal_label: UniversalLabel::default(),
        declared_version: 19,
        timestamp_us: Some(1_700_000_000_000_000),
        platform_designation: Some("PRED-UAV".to_string()),
        security_local_set: Some(security_bytes),
        uas_ls_version: Some(19),
        ..Default::default()
    }
}

#[test]
fn st0102_round_trips_through_st0601_tag_48() {
    let security = build_security_ls();
    let security_bytes = st0102::encode_to_vec(&security).expect("st0102 encode succeeds");

    let parent = build_parent_uas_ls(security_bytes.clone());
    let parent_bytes = st0601::encode_to_vec(&parent).expect("st0601 encode succeeds");

    let decoded_parent = st0601::decode(&parent_bytes).expect("st0601 decode succeeds");

    let inner_bytes = decoded_parent
        .security_local_set
        .as_deref()
        .expect("Tag 48 round-trips through ST 0601 layer");
    assert_eq!(inner_bytes, security_bytes.as_slice());

    let decoded_security = st0102::decode(inner_bytes).expect("st0102 decode succeeds");
    assert_eq!(decoded_security, security);
}

#[test]
fn st0102_strict_round_trips_through_st0601_tag_48() {
    // Same composition, but use decode_strict on the inner layer to
    // verify the typed surface meets ST 0102.12 §6.7 minimum
    // requirements when emitted by encode_to_vec.
    let security = build_security_ls();
    let security_bytes = st0102::encode_to_vec(&security).unwrap();

    let parent = build_parent_uas_ls(security_bytes);
    let parent_bytes = st0601::encode_to_vec(&parent).unwrap();
    let decoded_parent = st0601::decode(&parent_bytes).unwrap();

    let inner = decoded_parent.security_local_set.as_deref().unwrap();
    let decoded_security = st0102::decode_strict(inner).expect("strict decode succeeds");
    assert_eq!(decoded_security, security);
}
