#![no_main]

use libfuzzer_sys::fuzz_target;
use tst_core::codec::misp_time;
use tst_core::mpegts::mux::VideoCodec;

fuzz_target!(|data: &[u8]| {
    let _ = misp_time::extract(data, VideoCodec::H264);
    let _ = misp_time::extract(data, VideoCodec::H265);
});
