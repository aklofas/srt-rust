#![no_main]

use libfuzzer_sys::fuzz_target;
use tst_core::mpegts::demux::Demuxer;

// End-to-end demuxer panic-freedom: arbitrary bytes through `feed`
// followed by a complete event drain. Lenient mode never escalates
// non-conformance to a panic; this target locks that contract in
// against random input. Companion targets exercise the PSI parser
// (`demux_psi`) and the PES reassembler (`demux_pes_reassembly`)
// directly, which trade end-to-end coverage for tighter steering of
// libfuzzer onto a single state machine.
fuzz_target!(|data: &[u8]| {
    let mut d = Demuxer::new();
    let _ = d.feed(data);
    while d.next_event().is_some() {}
});
