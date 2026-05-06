#![no_main]

use libfuzzer_sys::fuzz_target;
use tst_core::mpegts::mux::{Config, Muxer};

fuzz_target!(|data: &[u8]| {
    let mut mux = Muxer::new(Config::default()).unwrap();
    let _ = mux.push_video(data, 0, false);
});
