#![no_main]

use libfuzzer_sys::fuzz_target;
use tst_core::codec::h265::parse_slice_header_light;

fuzz_target!(|data: &[u8]| {
    // Fuzz no-SPS path (most adversarial — exercises every bit-cursor branch
    // without bit-width SPS dependency). Use nal_unit_type=19 to also exercise
    // the idr = true branch.
    let _ = parse_slice_header_light(data, None, 19);
});
