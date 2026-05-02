#![no_main]

use libfuzzer_sys::fuzz_target;
use srt_core::SrtUrl;

fuzz_target!(|data: &[u8]| {
    // Property: SrtUrl::parse never panics on arbitrary bytes — it must
    // always return Result<SrtUrl, UrlError>. Spec §8.5.
    if let Ok(s) = std::str::from_utf8(data) {
        let _ = SrtUrl::parse(s);
    }
});
