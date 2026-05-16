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
