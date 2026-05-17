#![no_main]

use libfuzzer_sys::fuzz_target;
use tst_core::mpegts::demux::low_level::{parse_pat, parse_pmt};

// Direct fuzz of the PAT and PMT section parsers. Calling both on the
// same arbitrary slice is intentional — each parser returns a `Result`
// for any malformed input, and feeding both lets libfuzzer cover both
// state machines from one corpus entry. Should never panic.
fuzz_target!(|data: &[u8]| {
    let _ = parse_pat(data);
    let _ = parse_pmt(data);
});
