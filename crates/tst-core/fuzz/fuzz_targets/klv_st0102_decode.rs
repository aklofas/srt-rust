//! Fuzz target — panic-freedom on arbitrary bytes for both
//! `klv::st0102::decode` (lenient) and `klv::st0102::decode_strict`.
//!
//! Either function may return `Err`, but neither may panic on any
//! input byte sequence (including empty buffers, oversized lengths,
//! malformed UTF-16, codepoint extremes, etc.).

#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = tst_core::klv::st0102::decode(data);
    let _ = tst_core::klv::st0102::decode_strict(data);
});
