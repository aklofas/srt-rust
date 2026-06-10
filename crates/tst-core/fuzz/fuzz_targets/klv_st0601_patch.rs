#![no_main]

use libfuzzer_sys::fuzz_target;
use tst_core::klv::st0601::{decode, decode_unchecked, patch, UasDatalinkLs};

fuzz_target!(|data: &[u8]| {
    // Identity property: an input that decodes cleanly patched with NO
    // edits must round-trip byte-identically (verbatim TLV copy,
    // preserved outer-length bytes + trailing bytes, checksum recompute
    // reproducing the already-valid value).
    if decode(data).is_ok() {
        let out = patch(data, &UasDatalinkLs::default())
            .expect("patch must succeed on a decodable input");
        assert_eq!(out.as_slice(), data);
    }

    // Edit property: a successful patch must stay decodable and carry
    // the edit; patch must never panic on arbitrary input.
    let edits = UasDatalinkLs {
        frame_center_lat_deg: Some(12.5),
        ..UasDatalinkLs::default()
    };
    if let Ok(out) = patch(data, &edits) {
        let re = decode_unchecked(&out).expect("patched output must decode");
        let lat = re.frame_center_lat_deg.expect("edited tag must be present");
        assert!((lat - 12.5).abs() < 1e-4);
    }
});
