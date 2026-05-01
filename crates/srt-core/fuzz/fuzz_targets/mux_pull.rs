#![no_main]

use libfuzzer_sys::fuzz_target;
use srt_core::mpegts::mux::{Config, Muxer};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let mut mux = Muxer::new(Config::default()).unwrap();
    // Always push something so pull has work.
    let _ = mux.push_video(&[0x00, 0x00, 0x00, 0x01, 0x09, 0x10], 0, true);
    // Use first byte as buffer-size knob.
    let buf_size = (data[0] as usize) * 7; // 0..=1785
    let mut buf = vec![0u8; buf_size];
    let _ = mux.pull(&mut buf);
});
