//! Fuzz `srt_core::codec::av1::parse_sequence_header`.
//!
//! Property: never panic on arbitrary bytes. The hand-rolled AV1
//! Sequence Header parser walks dozens of bitfields with conditional
//! reads — high surface area for bit-cursor / off-by-one / unbounded-
//! loop bugs. AV1 spec §5.5.1 requires careful attention to the
//! reduced_still_picture_header and operating_points loops.

#![no_main]
use libfuzzer_sys::fuzz_target;
use srt_core::codec::av1::parse_sequence_header;

fuzz_target!(|data: &[u8]| {
    let _ = parse_sequence_header(data);
});
