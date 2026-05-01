#![no_main]

use libfuzzer_sys::fuzz_target;
use srt_core::klv::st0601::{decode, decode_strict, decode_unchecked, encode_to_vec};

fuzz_target!(|data: &[u8]| {
    let _ = decode(data);
    let _ = decode_strict(data);
    if let Ok(record) = decode_unchecked(data) {
        // Round-trip property: re-encoding a decoded record produces a buffer
        // that decodes back to an equivalent record.
        if let Ok(re_encoded) = encode_to_vec(&record) {
            if let Ok(re_decoded) = decode_unchecked(&re_encoded) {
                // Field equality across the round trip — typed fields only;
                // unknown ordering may differ.
                assert_eq!(record.timestamp_us, re_decoded.timestamp_us);
                assert_eq!(record.sensor_lat_deg, re_decoded.sensor_lat_deg);
            }
        }
    }
});
