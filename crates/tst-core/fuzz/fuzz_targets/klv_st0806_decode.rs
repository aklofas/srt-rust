#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = tst_core::klv::st0806::decode(data);
    let _ = tst_core::klv::st0806::decode_standalone(data);
});
