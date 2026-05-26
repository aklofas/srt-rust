#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;

// Feed arbitrary bytes through the RFC 7826 §14 interleaved demuxer.
// The reader alternates between $-framed binary chunks and ASCII RTSP
// messages, so the fuzzer exercises both paths. We cap the loop at 100
// frames so a pathological seed (e.g. zero-length binary frames) can't
// exhaust memory or wall-clock budget.
fuzz_target!(|data: &[u8]| {
    let mut r = tst_rtp::InterleavedReader::new(Cursor::new(data));
    for _ in 0..100 {
        match r.next_frame() {
            Ok(None) | Err(_) => break,
            Ok(Some(_)) => continue,
        }
    }
});
