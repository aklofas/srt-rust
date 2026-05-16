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

// ----------------------------------------------------------------------------
// ST 0601 UasDatalinkLs
// ----------------------------------------------------------------------------

use tst_core::klv::st0601::{self, UasDatalinkLs};

// Three categories of tag for the per-tag strategy. Each round-trips
// through `encode_to_vec` + `decode` but with a different comparison
// shape, so we keep them as distinct proptests for readable failures.

proptest! {
    /// Exact-equality tags (strings, opaque bytes, integer types).
    ///
    /// Strategy: pick one tag uniformly at random, populate just that
    /// field in an otherwise-default record, round-trip. All these
    /// tags use byte-passthrough or integer encoding; the comparison
    /// is `assert_eq!`.
    ///
    /// **Tag 65 quirk:** the ST 0601 encoder unconditionally emits Tag 65
    /// ("UAS LS Version Number") — if `record.uas_ls_version` is None it
    /// auto-emits the default `EncodeOptions::version` byte (19, per ST
    /// 0601.19). The decoder reads that back into `uas_ls_version =
    /// Some(19)`, breaking `assert_eq` if the input had None. We pre-set
    /// `uas_ls_version = Some(19)` on the input so both sides match;
    /// the `which == 6` arm overwrites this with the random `v_u8` to
    /// actually exercise the tag's value space.
    #[test]
    fn st0601_exact_equality_per_tag_roundtrip(
        // Discriminator selecting which tag to populate. Range covers
        // the 10 exact-equality tag arms in the match below.
        which in 0u8..10,
        // Generic value space large enough to feed any single field.
        // Bytes: any opaque payload. String: ASCII subset 0x20..=0x7E
        // (printable ASCII; lenient UTF-8 decode is identity on this
        // subset, so round-trip is exact). Range 1..=32 keeps things
        // BER-short-form (length ≤ 127) and proptest case fast.
        bytes in proptest::collection::vec(any::<u8>(), 1..=32),
        s in "[ -~]{1,32}",
        v_u8 in any::<u8>(),
        v_u64 in any::<u64>(),
    ) {
        // Pre-populate the auto-emitted Tag 65 to match what the decoder
        // will produce; see method-level docstring for the rationale.
        let mut record = UasDatalinkLs {
            uas_ls_version: Some(19),
            ..Default::default()
        };
        match which {
            0 => record.mission_id = Some(s),
            1 => record.platform_tail_number = Some(s),
            2 => record.platform_designation = Some(s),
            3 => record.image_source_sensor = Some(s),
            // Tag 12 image_coordinate_system: lenient decode accepts
            // any string but spec values are short ("WGS84" etc).
            // ASCII strategy is in-spec for the values encoders produce.
            4 => record.image_coordinate_system = Some(s),
            5 => record.platform_call_sign = Some(s),
            // Overwrite the default-version pre-set with a random byte
            // to actually exercise Tag 65's value space.
            6 => record.uas_ls_version = Some(v_u8),
            7 => record.timestamp_us = Some(v_u64),
            8 => record.generic_flag_data = Some(v_u8),
            // Tag 48 security_local_set: opaque pass-through to ST 0102
            // sibling layer (per convention #1 in
            // reference_klv_typed_set_conventions). Round-trip is
            // byte-identical at the ST 0601 layer.
            9 => record.security_local_set = Some(bytes.clone()),
            _ => unreachable!(),
        }
        let buf = st0601::encode_to_vec(&record)
            .expect("ST 0601 encode of valid record must succeed");
        let decoded = st0601::decode(&buf)
            .expect("ST 0601 decode of self-encoded bytes must succeed");
        // Compare just the field we set — `field_errors` and `unknown`
        // are decode-side diagnostics, empty on a clean self-encoded
        // record; the default fields stay None on both sides.
        prop_assert_eq!(decoded, record);
    }

    /// Tag 74 vmti opaque pass-through (separate so failure messages
    /// localize when the ST 0903 sibling layering breaks the parent's
    /// byte-passthrough contract). Plan #35 ratified this surface
    /// (memory: project_klv_st0903_shipped).
    #[test]
    fn st0601_vmti_passthrough_roundtrip(
        bytes in proptest::collection::vec(any::<u8>(), 1..=64),
    ) {
        let record = UasDatalinkLs { vmti: Some(bytes.clone()), ..Default::default() };
        let buf = st0601::encode_to_vec(&record).expect("encode");
        let decoded = st0601::decode(&buf).expect("decode");
        prop_assert_eq!(decoded.vmti.as_deref(), Some(bytes.as_slice()));
    }

    /// IMAPB f64 tags: pick one ranged tag uniformly, encode value at
    /// random `t ∈ [0,1]` lerped into the tag's [min, max] range,
    /// decode, assert within IMAPB tolerance.
    ///
    /// Tolerance derivation mirrors `klv_proptest.rs::imapb_roundtrip`:
    /// max of (quantization scale, f64 ULP × span/magnitude × safety
    /// factor). See that file lines 73-94 for the rationale.
    ///
    /// `which` indexes into the ranged-tag table built below. The
    /// table is a hand-curated subset covering both IMAPB encodings
    /// commonly used (signed +/- ranges and unsigned 0..N ranges) and
    /// the three integer widths in play (i8, i16, i32, u8, u16, u32).
    /// Full coverage of every tag is the job of `every_typed_tag_round_trips`
    /// (st0601/mod.rs:1498); this proptest's job is value-space.
    #[test]
    fn st0601_imapb_tag_value_space_roundtrip(
        which in 0usize..8,
        t in 0.0f64..=1.0,
    ) {
        // (tag_id, min, max, field-getter). Eight representative tags
        // covering signed/unsigned and the integer-width spectrum.
        struct ImapbTag {
            #[allow(dead_code)] // ID retained for failure-message clarity
            id: u8,
            min: f64,
            max: f64,
            set: fn(&mut UasDatalinkLs, f64),
            get: fn(&UasDatalinkLs) -> Option<f64>,
        }
        let tags: [ImapbTag; 8] = [
            // Tag 5: platform_heading_angle 0..360 (u16 → 2 bytes, ST 0601 §item5)
            ImapbTag { id: 5, min: 0.0, max: 360.0,
                set: |r, v| r.platform_heading_deg = Some(v),
                get: |r| r.platform_heading_deg },
            // Tag 6: platform_pitch_angle ±20° (i16 → 2 bytes)
            ImapbTag { id: 6, min: -20.0, max: 20.0,
                set: |r, v| r.platform_pitch_deg = Some(v),
                get: |r| r.platform_pitch_deg },
            // Tag 13: sensor_latitude ±90° (i32 → 4 bytes)
            ImapbTag { id: 13, min: -90.0, max: 90.0,
                set: |r, v| r.sensor_lat_deg = Some(v),
                get: |r| r.sensor_lat_deg },
            // Tag 14: sensor_longitude ±180° (i32 → 4 bytes)
            ImapbTag { id: 14, min: -180.0, max: 180.0,
                set: |r, v| r.sensor_lon_deg = Some(v),
                get: |r| r.sensor_lon_deg },
            // Tag 15: sensor_altitude_m -900..19000 (u16 → 2 bytes, asymmetric range)
            ImapbTag { id: 15, min: -900.0, max: 19000.0,
                set: |r, v| r.sensor_alt_m = Some(v),
                get: |r| r.sensor_alt_m },
            // Tag 21: slant_range 0..5,000,000 m (u32 → 4 bytes, large unsigned range)
            ImapbTag { id: 21, min: 0.0, max: 5_000_000.0,
                set: |r, v| r.slant_range_m = Some(v),
                get: |r| r.slant_range_m },
            // Tag 90: platform_pitch_full ±90° (i32 → 4 bytes)
            ImapbTag { id: 90, min: -90.0, max: 90.0,
                set: |r, v| r.platform_pitch_full_deg = Some(v),
                get: |r| r.platform_pitch_full_deg },
            // Tag 50: platform_angle_of_attack ±20° (i16 → 2 bytes; renamed by plan #44)
            ImapbTag { id: 50, min: -20.0, max: 20.0,
                set: |r, v| r.platform_angle_of_attack_deg = Some(v),
                get: |r| r.platform_angle_of_attack_deg },
        ];
        let tag = &tags[which];
        let value = tag.min + t * (tag.max - tag.min);

        let mut record = UasDatalinkLs::default();
        (tag.set)(&mut record, value);

        let buf = st0601::encode_to_vec(&record).expect("encode");
        let decoded = st0601::decode(&buf).expect("decode");
        let got = (tag.get)(&decoded).expect("field must be present after round-trip");

        // Tolerance derivation (see klv_proptest.rs::imapb_roundtrip
        // lines 73-94): max of IMAPB quantization step and f64-ULP
        // floor scaled by field magnitude with a safety factor.
        let span = tag.max - tag.min;
        // Wire byte width for this tag — read from the encoded bytes is
        // overkill; use the known per-tag widths above. The 4-byte tags
        // (13, 14, 21, 90) give the tightest scale; widen the tolerance
        // by computing for the smallest width we encode (2 bytes) so the
        // bound is safe for all members of the array.
        let length = 2usize;  // conservative — i32 tags get tighter actual scale
        let log2_ceil = span.log2().ceil();
        let scale = 2f64.powf(log2_ceil) / 2f64.powi(8 * length as i32 - 1);
        let magnitude = span.max(tag.min.abs()).max(tag.max.abs()).max(1.0);
        let fp_eps = f64::EPSILON * magnitude * 4.0;
        let tol = scale.max(fp_eps);

        prop_assert!(
            (got - value).abs() <= tol,
            "ST 0601 tag {} value {} round-tripped to {} (delta {}, tol {})",
            tag.id, value, got, (got - value).abs(), tol
        );
    }
}
