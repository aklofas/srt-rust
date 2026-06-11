#![no_main]

use libfuzzer_sys::fuzz_target;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{Muxer, MuxerConfig};

fuzz_target!(|data: &[u8]| {
    let mut mux = Muxer::new(MuxerConfig::default()).unwrap();
    // Need a video PUSI before KLV makes sense — push a tiny synthetic
    // AU first to get the muxer past first-frame state.
    let _ = mux.push_video(
        &[0x00, 0x00, 0x00, 0x01, 0x09, 0x10],
        Pts90khz::new(0),
        true,
    );
    let _ = mux.push_klv(data, Pts90khz::new(0), 0);
});
