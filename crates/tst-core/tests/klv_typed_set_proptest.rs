//! Round-trip property tests for **typed** KLV Local Sets and Packs in
//! `tst_core::klv::st*`.
//!
//! ## Convention
//!
//! **Every typed set in `klv::*` MUST have a round-trip proptest in this
//! file.** When adding a new typed set (e.g. ST 0806 RVT), add a section
//! here using the TSDuck-style shape:
//!
//! ```text
//! 1. Define a `proptest::Strategy` that generates a typed record with
//!    field values sampled from spec-realistic ranges (one field at a
//!    time is fine — see `st0601_per_tag_roundtrip` below).
//! 2. Encode → decode → assert_eq!(original, decoded).
//! 3. For f64-bearing fields routed through IMAPB, allow a tolerance
//!    computed per `klv_proptest.rs::imapb_roundtrip` (scale_factor
//!    plus f64-precision floor).
//! ```
//!
//! The existing in-module example-based `round_trip_*` tests cover
//! fixed-value records; these proptests add **value-space exploration**
//! and catch range-boundary + f64-precision corners.
//!
//! ## Why this is its own file (not `klv_proptest.rs`)
//!
//! `klv_proptest.rs` covers the **substrate** (BER, BER-OID, IMAPB).
//! This file covers the **typed sets that sit on top**. Splitting keeps
//! each file under ~300 LoC and makes the "where do I add the proptest
//! for my new typed set?" answer mechanical: this file.
//!
//! ## Failure-mode discipline
//!
//! A property failure here means value-space exploration surfaced a
//! real bug in the encoder/decoder. Capture the regression seed
//! (proptest does this automatically in `.proptest-regressions`) and
//! file a follow-up plan. Do NOT mask with `prop_assume!` without
//! recording the bug — silent suppression defeats the point of the
//! property test.

use proptest::prelude::*;

use tst_core::klv::st0605::{self, PrecisionTimeStampPack, TimeStatus};

proptest! {
    /// ST 0605 PrecisionTimeStampPack round-trip: build a typed pack
    /// with random TimeStatus byte + random u64 microseconds, encode to
    /// the 26-byte canonical layout (UL + BER length + body), decode,
    /// assert_eq.
    ///
    /// `time_status_byte`: full u8 domain (0..=255). `TimeStatus` is a
    /// transparent wrapper; encode/decode are byte-passthrough. The
    /// reserved-bits validity check (`reserved_bits_valid`) is a
    /// downstream consumer concern, not an encode/decode invariant.
    ///
    /// `timestamp_us`: full u64 domain. POSIX microseconds since 1970
    /// can in principle hit any u64 value; the codec uses big-endian
    /// `to_be_bytes` / `from_be_bytes` so the property is byte-identity.
    #[test]
    fn st0605_precision_time_stamp_pack_roundtrip(
        time_status_byte in any::<u8>(),
        timestamp_us in any::<u64>(),
    ) {
        let original = PrecisionTimeStampPack {
            time_status: TimeStatus(time_status_byte),
            timestamp_us,
        };
        let bytes = st0605::encode(&original);
        // encode returns a fixed-size [u8; 26]; sanity-check the layout
        // boundary (UL + 1-byte BER length=9 + 9-byte body).
        prop_assert_eq!(bytes.len(), 26);
        let decoded = st0605::decode(&bytes).expect("ST 0605 decode of valid pack must succeed");
        prop_assert_eq!(decoded, original);
    }
}

// ----------------------------------------------------------------------------
// ST 0102 SecurityLs
// ----------------------------------------------------------------------------

use tst_core::klv::st0102::{
    self, ClassifyingCountryCodingMethod, ObjectCountryCodingMethod, SecurityClassification,
    SecurityLs,
};

/// One-tag-at-a-time strategy: returns a `SecurityLs` with default
/// (all-None) fields except for ONE tag set to a random value.
///
/// Why one-at-a-time: combinatorial explosion. SecurityLs has 17
/// optional fields; sampling all of them independently produces 2^17
/// distinct presence permutations before considering value spaces. The
/// existing in-module `round_trip_full_record` test (st0102/mod.rs:842)
/// already covers the "all populated" case with fixed values — this
/// proptest's job is value-space exploration per tag.
///
/// Why the enum sub-strategies restrict `Unknown(byte)` to non-named
/// byte ranges: `decode` maps named codepoints back to their named
/// variants, so constructing e.g. `SecurityClassification::Unknown(0x01)`
/// and encoding it would round-trip back as `Unclassified` (the named
/// variant at codepoint 0x01) — breaking `assert_eq`. Each enum's safe
/// Unknown range is the set of bytes `b` where `from_u8(b) == Unknown(b)`,
/// i.e. the bytes outside the spec's named codepoint set. Confirmed
/// against `crates/tst-core/src/klv/st0102/enums.rs` codepoint comments.
fn st0102_one_tag_record_strategy() -> impl Strategy<Value = SecurityLs> {
    // ASCII-only string for fields whose lenient-mode UTF-8 / UTF-16
    // decode would otherwise normalize away non-roundtrippable input.
    // Tag 13 is the explicit UTF-16 case; all other text fields are
    // straight UTF-8 but ASCII keeps the strategy small and noise-free.
    // Length 1..=64 to exercise the BER short-form length boundary (≤127).
    let ascii_string = "[ -~]{1,64}";

    prop_oneof![
        // Tag 1: SecurityClassification enum. Named codepoints per
        // klv/st0102/enums.rs: 0x01..=0x05 (Unclassified..=TopSecret).
        // Safe Unknown bytes: 0x00 plus 0x06..=0xFF.
        prop_oneof![
            Just(SecurityClassification::Unclassified),
            Just(SecurityClassification::Restricted),
            Just(SecurityClassification::Confidential),
            Just(SecurityClassification::Secret),
            Just(SecurityClassification::TopSecret),
            Just(SecurityClassification::Unknown(0x00)),
            (0x06u8..=0xFFu8).prop_map(SecurityClassification::Unknown),
        ]
        .prop_map(|v| SecurityLs {
            security_classification: Some(v),
            ..Default::default()
        }),
        // Tag 2: ClassifyingCountryCodingMethod enum. Named codepoints
        // per klv/st0102/enums.rs: 0x01..=0x10 (contiguous, includes
        // OmittedValue08/09 reserved slots which are still recognized
        // by `from_u8`). Safe Unknown bytes: 0x00 plus 0x11..=0xFF.
        // Strategy samples all 16 named variants plus Unknown(byte) over
        // the safe range — the named-variant encode path (`to_u8` match
        // arm by name) is a different code path from the Unknown encode
        // path (`Unknown(b) => b`), so each named arm needs coverage.
        prop_oneof![
            Just(ClassifyingCountryCodingMethod::Iso3166TwoLetter),
            Just(ClassifyingCountryCodingMethod::Iso3166ThreeLetter),
            Just(ClassifyingCountryCodingMethod::Fips104TwoLetter),
            Just(ClassifyingCountryCodingMethod::Fips104FourLetter),
            Just(ClassifyingCountryCodingMethod::Iso3166Numeric),
            Just(ClassifyingCountryCodingMethod::Stanag1059TwoLetter),
            Just(ClassifyingCountryCodingMethod::Stanag1059ThreeLetter),
            Just(ClassifyingCountryCodingMethod::OmittedValue08),
            Just(ClassifyingCountryCodingMethod::OmittedValue09),
            Just(ClassifyingCountryCodingMethod::Fips104Mixed),
            Just(ClassifyingCountryCodingMethod::Iso3166Mixed),
            Just(ClassifyingCountryCodingMethod::Stanag1059Mixed),
            Just(ClassifyingCountryCodingMethod::GencTwoLetter),
            Just(ClassifyingCountryCodingMethod::GencThreeLetter),
            Just(ClassifyingCountryCodingMethod::GencNumeric),
            Just(ClassifyingCountryCodingMethod::GencMixed),
            Just(ClassifyingCountryCodingMethod::Unknown(0x00)),
            (0x11u8..=0xFFu8).prop_map(ClassifyingCountryCodingMethod::Unknown),
        ]
        .prop_map(|v| SecurityLs {
            classifying_country_coding_method: Some(v),
            ..Default::default()
        }),
        // Tag 3: classifying_country (UTF-8).
        ascii_string.prop_map(|s| SecurityLs {
            classifying_country: Some(s),
            ..Default::default()
        }),
        // Tag 12: ObjectCountryCodingMethod enum. Note: this is a
        // distinct enum from Tag 2's even though some variant names
        // overlap — the numeric codepoints differ (e.g. Iso3166Numeric
        // is 0x03 here but 0x05 in Tag 2's enum). Named codepoints per
        // klv/st0102/enums.rs: 0x01..=0x0F contiguous, then a jump to
        // 0x40 for GencAdminSub. Safe Unknown bytes: 0x00, 0x10..=0x3F,
        // and 0x41..=0xFF (excluding the 0x40 island).
        // Strategy samples all 16 named variants plus Unknown(byte) over
        // the safe ranges — the named-variant encode path (`to_u8` match
        // arm by name) is a different code path from the Unknown encode
        // path (`Unknown(b) => b`), so each named arm needs coverage.
        prop_oneof![
            Just(ObjectCountryCodingMethod::Iso3166TwoLetter),
            Just(ObjectCountryCodingMethod::Iso3166ThreeLetter),
            Just(ObjectCountryCodingMethod::Iso3166Numeric),
            Just(ObjectCountryCodingMethod::Fips104TwoLetter),
            Just(ObjectCountryCodingMethod::Fips104FourLetter),
            Just(ObjectCountryCodingMethod::Stanag1059TwoLetter),
            Just(ObjectCountryCodingMethod::Stanag1059ThreeLetter),
            Just(ObjectCountryCodingMethod::OmittedValue08),
            Just(ObjectCountryCodingMethod::OmittedValue09),
            Just(ObjectCountryCodingMethod::OmittedValue0A),
            Just(ObjectCountryCodingMethod::OmittedValue0B),
            Just(ObjectCountryCodingMethod::OmittedValue0C),
            Just(ObjectCountryCodingMethod::GencTwoLetter),
            Just(ObjectCountryCodingMethod::GencThreeLetter),
            Just(ObjectCountryCodingMethod::GencNumeric),
            Just(ObjectCountryCodingMethod::GencAdminSub),
            Just(ObjectCountryCodingMethod::Unknown(0x00)),
            (0x10u8..=0x3Fu8).prop_map(ObjectCountryCodingMethod::Unknown),
            (0x41u8..=0xFFu8).prop_map(ObjectCountryCodingMethod::Unknown),
        ]
        .prop_map(|v| SecurityLs {
            object_country_coding_method: Some(v),
            ..Default::default()
        }),
        // Tag 13: object_country_codes (UTF-16 on the wire, BE-normalized).
        // Strategy uses ASCII so the value is byte-identical pre- and
        // post-normalization. UTF-16 BOM stripping + LE→BE normalization
        // is exercised by the in-module test `round_trip_utf16_normalizes_to_be`.
        ascii_string.prop_map(|s| SecurityLs {
            object_country_codes: Some(s),
            ..Default::default()
        }),
        // Tag 22: version u16 (2-byte BE).
        any::<u16>().prop_map(|v| SecurityLs {
            version: Some(v),
            ..Default::default()
        }),
        // Tags 4-11, 14, 23, 24: free-text UTF-8 fields.
        ascii_string.prop_map(|s| SecurityLs {
            sci_shi_info: Some(s),
            ..Default::default()
        }),
        ascii_string.prop_map(|s| SecurityLs {
            caveats: Some(s),
            ..Default::default()
        }),
        ascii_string.prop_map(|s| SecurityLs {
            releasing_instructions: Some(s),
            ..Default::default()
        }),
        ascii_string.prop_map(|s| SecurityLs {
            classified_by: Some(s),
            ..Default::default()
        }),
        ascii_string.prop_map(|s| SecurityLs {
            derived_from: Some(s),
            ..Default::default()
        }),
        ascii_string.prop_map(|s| SecurityLs {
            classification_reason: Some(s),
            ..Default::default()
        }),
        ascii_string.prop_map(|s| SecurityLs {
            declassification_date: Some(s),
            ..Default::default()
        }),
        ascii_string.prop_map(|s| SecurityLs {
            classification_marking_system: Some(s),
            ..Default::default()
        }),
        ascii_string.prop_map(|s| SecurityLs {
            classification_comments: Some(s),
            ..Default::default()
        }),
        ascii_string.prop_map(|s| SecurityLs {
            classifying_country_coding_method_version_date: Some(s),
            ..Default::default()
        }),
        ascii_string.prop_map(|s| SecurityLs {
            object_country_coding_method_version_date: Some(s),
            ..Default::default()
        }),
    ]
}

proptest! {
    /// ST 0102 SecurityLs per-tag round-trip via lenient `decode`.
    ///
    /// Uses lenient decode because the strategy produces records with
    /// only one optional field set, which `decode_strict` would reject
    /// for missing required tags. Lenient decode is the symmetric
    /// inverse of `encode` for any valid in-memory record.
    #[test]
    fn st0102_per_tag_roundtrip(record in st0102_one_tag_record_strategy()) {
        let bytes = st0102::encode_to_vec(&record).expect("ST 0102 encode of valid record must succeed");
        let decoded = st0102::decode(&bytes).expect("ST 0102 decode of self-encoded bytes must succeed");
        prop_assert_eq!(decoded, record);
    }
}
