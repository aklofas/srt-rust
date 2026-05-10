#![no_main]

use libfuzzer_sys::fuzz_target;

// Fuzz a minimal TS packet scanner. Exercises packet-boundary recognition
// and adaptation-field length parsing — same logic used in the test
// parser. Should never panic on any input.
fuzz_target!(|data: &[u8]| {
    for pkt in data.chunks(188) {
        if pkt.len() < 4 || pkt[0] != 0x47 {
            continue;
        }
        let _pid = (((pkt[1] as u16) & 0x1F) << 8) | (pkt[2] as u16);
        let afc = (pkt[3] >> 4) & 0x3;
        let mut payload_offset = 4usize;
        if afc & 0x2 != 0 && pkt.len() > 4 {
            let af_len = pkt[4] as usize;
            payload_offset = payload_offset.saturating_add(1 + af_len);
        }
        if payload_offset >= pkt.len() {
            continue;
        }
        // Touch the payload to ensure no out-of-bounds.
        let _ = &pkt[payload_offset..];
    }
});
