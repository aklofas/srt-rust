#![no_main]

use libfuzzer_sys::fuzz_target;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{Muxer, MuxerConfig};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let mut mux = Muxer::new(MuxerConfig::default()).unwrap();
    // Always push something so pull has work.
    let _ = mux.push_video(
        &[0x00, 0x00, 0x00, 0x01, 0x09, 0x10],
        Pts90khz::new(0),
        true,
    );
    // Use first byte as buffer-size knob.
    let buf_size = (data[0] as usize) * 7; // 0..=1785
    let mut buf = vec![0u8; buf_size];
    let _ = mux.pull(&mut buf);
});
