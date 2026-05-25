#![no_main]

use libfuzzer_sys::fuzz_target;
use tst_rtp::RtpHeader;

// Feed arbitrary bytes to RtpHeader::decode. Successful and failed
// decodes are both fine; the harness asserts no panics, no unsoundness.
fuzz_target!(|data: &[u8]| {
    let _ = RtpHeader::decode(data);
});
