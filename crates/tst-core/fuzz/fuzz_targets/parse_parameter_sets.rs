//! Property: every parser must terminate with Ok or Err on any byte
//! input — never panic, never allocate without bound.

#![no_main]
use libfuzzer_sys::fuzz_target;
use tst_core::codec::{h264, h265, h266};

fuzz_target!(|data: &[u8]| {
    // Run every parser independently — each is exercised against the
    // same input regardless of whether earlier parsers succeeded. We
    // only check for panic-freedom; result values are intentionally
    // discarded.
    let _ = h264::parse_sps(data);
    let _ = h264::parse_pps(data);
    let _ = h265::parse_vps(data);
    let _ = h265::parse_sps(data);
    let _ = h265::parse_pps(data);
    let _ = h266::parse_vps(data);
    let _ = h266::parse_sps(data);
    let _ = h266::parse_pps(data);
});
