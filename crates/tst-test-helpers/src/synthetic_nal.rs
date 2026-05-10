//! Synthetic Annex-B access unit generator for mux tests.
//!
//! Produces byte sequences that *look* like H.264/H.265 access units
//! (start codes + NALU header + body) without actually decoding to a
//! valid picture. The mux treats the bytes opaquely, so this is enough
//! to exercise PES fragmentation, continuity counters, and PSI cadence.

/// Build a synthetic H.264 access unit.
///
/// `body_size` controls the total NALU payload size (excluding the start
/// code). `key_frame=true` produces an IDR slice NAL type (5); false
/// produces a non-IDR slice NAL type (1).
pub fn h264_au(body_size: usize, key_frame: bool) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + body_size);
    buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    let nal_type: u8 = if key_frame { 5 } else { 1 };
    // forbidden_zero(1)=0 | nal_ref_idc(2)=11 (IDR) or 10 (P) | nal_unit_type(5)
    let nri: u8 = if key_frame { 0b11 } else { 0b10 };
    buf.push((nri << 5) | nal_type);
    // Filler bytes: deterministic pattern so tests can verify recovery.
    let pattern_byte = if key_frame { 0xA5 } else { 0x5A };
    for i in 1..body_size {
        buf.push(pattern_byte ^ (i as u8));
    }
    buf
}

/// Build a synthetic H.265 access unit (for `VideoCodec::H265` tests).
pub fn h265_au(body_size: usize, key_frame: bool) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + body_size);
    buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    // H.265 NAL header is 2 bytes:
    // forbidden_zero(1) | nal_unit_type(6) | layer_id(6) | tid(3)
    let nal_unit_type: u8 = if key_frame { 19 } else { 1 }; // IDR_W_RADL = 19
    buf.push(nal_unit_type << 1);
    buf.push(0x01); // tid = 1
    for i in 2..body_size {
        buf.push((i as u8).rotate_left(3));
    }
    buf
}

/// Build a synthetic KLV blob — just a fixed pattern of bytes. The mux
/// treats KLV opaquely, so we don't need a parseable record here.
pub fn klv_blob(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i as u8) ^ 0xC3).collect()
}
