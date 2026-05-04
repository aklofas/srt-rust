//! Property: every parser must terminate with Ok or Err on any byte
//! input — never panic, never allocate without bound.

#![no_main]
use libfuzzer_sys::fuzz_target;
use srt_core::codec::{h264, h265};

fuzz_target!(|data: &[u8]| {
    let _ = h264::parse_sps(data);
    let _ = h264::parse_pps(data);
    let _ = h265::parse_vps(data);
    let _ = h265::parse_sps(data);
    let _ = h265::parse_pps(data);
});
