#![no_main]

use libfuzzer_sys::fuzz_target;
use tst_core::codec;

const MAX_FRAMES_PER_INPUT: usize = 1000;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let codec_select = data[0] & 1;
    let body = &data[1..];

    if codec_select == 0 {
        let mut count = 0;
        for r in codec::mpegaudio::frames(body) {
            let _ = r; // ignore; the goal is panic-freedom
            count += 1;
            if count >= MAX_FRAMES_PER_INPUT {
                break;
            }
        }
    } else {
        let mut count = 0;
        for r in codec::aac::frames(body) {
            let _ = r;
            count += 1;
            if count >= MAX_FRAMES_PER_INPUT {
                break;
            }
        }
    }
});
