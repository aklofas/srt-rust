#![no_main]

use libfuzzer_sys::fuzz_target;
use srt_core::klv::pack::Iter;

fuzz_target!(|data: &[u8]| {
    // Local-set iteration must not panic on arbitrary input.
    for r in Iter::local_set(data) {
        let _ = r;
    }
    // Universal-set iteration must not panic on arbitrary input.
    for r in Iter::universal_set(data) {
        let _ = r;
    }
});
