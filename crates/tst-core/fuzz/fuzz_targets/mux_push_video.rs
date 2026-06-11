#![no_main]

use libfuzzer_sys::fuzz_target;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{Muxer, MuxerConfig};

fuzz_target!(|data: &[u8]| {
    let mut mux = Muxer::new(MuxerConfig::default()).unwrap();
    let _ = mux.push_video(data, Pts90khz::new(0), false);
});
