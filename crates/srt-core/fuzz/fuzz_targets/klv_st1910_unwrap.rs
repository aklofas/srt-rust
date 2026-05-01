#![no_main]

use libfuzzer_sys::fuzz_target;
use srt_core::klv::st1910::unwrap_au_cell;

fuzz_target!(|data: &[u8]| {
    let _ = unwrap_au_cell(data);
});
