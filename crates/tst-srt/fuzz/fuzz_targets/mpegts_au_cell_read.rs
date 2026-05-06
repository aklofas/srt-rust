#![no_main]

use libfuzzer_sys::fuzz_target;
use tst_core::mpegts::au_cell::read_metadata_au_cell;

fuzz_target!(|data: &[u8]| {
    let _ = read_metadata_au_cell(data);
});
