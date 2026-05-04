//! Integration tests for multi-program TS demuxing — PAT diffing.
//!
//! Covers: first PAT creates trackers, version bump adds a program, version
//! bump removes a dropped program.  PMT contents are intentionally absent
//! here — those arrive in Task 11.

use srt_core::mpegts::demux::Demuxer;

// ---------------------------------------------------------------------------
// Test helper — synthesise a well-formed PAT TS packet
// ---------------------------------------------------------------------------
//
// Layout (188 bytes):
//   [0]    sync byte 0x47
//   [1]    PUSI bit (0x40) | PID hi nibble (PAT = 0x0000, so 0x40)
//   [2]    PID lo byte (0x00)
//   [3]    adaptation_field_control = 0b01 (payload only), CC = 0 → 0x10
//   [4]    pointer_field = 0x00
//   [5]    table_id = 0x00 (PAT)
//   [6]    0xB0 | section_length_hi   (section_syntax=1, private=0, reserved=11)
//   [7]    section_length_lo
//   [8-9]  transport_stream_id = 0x0001
//   [10]   reserved(2)=11 | version_number(5) << 1 | current_next=1
//   [11]   section_number = 0
//   [12]   last_section_number = 0
//   [13..] program loop: 4 bytes per entry (program_number:16, reserved:3+pmt_pid:13)
//   ...    CRC-32/MPEG-2 (4 bytes)
//   rest   0xFF padding to 188
//
// section_length = 5 (header tail) + 4*N (program loop) + 4 (CRC)
fn pat_packet_with_programs(programs: &[(u16, u16)], version: u8) -> Vec<u8> {
    // ---- build the PAT section bytes (starting from table_id) ----
    let section_length = 5 + 4 * programs.len() + 4; // bytes after section_length field
    let mut sec: Vec<u8> = Vec::with_capacity(3 + section_length);

    sec.push(0x00); // table_id = PAT
    sec.push(0xB0 | ((section_length >> 8) as u8 & 0x0F)); // section_syntax=1, reserved=11, length hi
    sec.push((section_length & 0xFF) as u8);
    sec.push(0x00); // transport_stream_id hi
    sec.push(0x01); // transport_stream_id lo
    sec.push(0xC1 | ((version & 0x1F) << 1)); // reserved(2)=11 | version(5) | current_next=1
    sec.push(0x00); // section_number
    sec.push(0x00); // last_section_number
    for &(pn, pmt_pid) in programs {
        sec.push((pn >> 8) as u8);
        sec.push((pn & 0xFF) as u8);
        sec.push(0xE0 | ((pmt_pid >> 8) as u8 & 0x1F)); // reserved(3)=111 | pid hi
        sec.push((pmt_pid & 0xFF) as u8);
    }

    // CRC-32/MPEG-2: generator polynomial 0x04C11DB7, initial value 0xFFFFFFFF
    let crc = crc32_mpeg2(&sec);
    sec.push((crc >> 24) as u8);
    sec.push((crc >> 16) as u8);
    sec.push((crc >> 8) as u8);
    sec.push(crc as u8);

    // ---- wrap in a 188-byte TS packet ----
    let mut pkt = vec![0xFFu8; 188];
    pkt[0] = 0x47; // sync
    pkt[1] = 0x40; // PUSI | PID hi = 0 (PAT PID = 0x0000)
    pkt[2] = 0x00; // PID lo
    pkt[3] = 0x10; // adaptation_field_control = 0b01, CC = 0
    pkt[4] = 0x00; // pointer_field
    // Section starts at byte 5.
    let sec_start = 5;
    let sec_end = sec_start + sec.len();
    assert!(sec_end <= 188, "PAT section too large for one TS packet");
    pkt[sec_start..sec_end].copy_from_slice(&sec);
    // Remaining bytes stay 0xFF (already initialised).

    pkt
}

/// Minimal CRC-32/MPEG-2 used by the PAT section builder above.
/// Polynomial 0x04C11DB7, initial value 0xFFFFFFFF, no final XOR.
fn crc32_mpeg2(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            if crc & 0x8000_0000 != 0 {
                crc = (crc << 1) ^ 0x04C1_1DB7;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn first_pat_creates_program_trackers_for_all_entries() {
    let mut demuxer = Demuxer::new();
    let pat = pat_packet_with_programs(&[(1, 0x1000), (2, 0x1100)], 0);
    demuxer.feed(&pat).unwrap();

    let progs = demuxer.programs_for_test();
    assert_eq!(
        progs.len(),
        2,
        "expected 2 program trackers, got {}",
        progs.len()
    );
    assert!(
        progs.contains_key(&0x1000),
        "missing tracker for pmt_pid=0x1000"
    );
    assert!(
        progs.contains_key(&0x1100),
        "missing tracker for pmt_pid=0x1100"
    );
}

#[test]
fn pat_version_bump_adds_new_program() {
    let mut demuxer = Demuxer::new();
    demuxer
        .feed(&pat_packet_with_programs(&[(1, 0x1000)], 0))
        .unwrap();
    assert_eq!(demuxer.programs_for_test().len(), 1);

    demuxer
        .feed(&pat_packet_with_programs(&[(1, 0x1000), (2, 0x1100)], 1))
        .unwrap();
    let progs = demuxer.programs_for_test();
    assert_eq!(
        progs.len(),
        2,
        "expected 2 trackers after version bump, got {}",
        progs.len()
    );
    assert!(progs.contains_key(&0x1000));
    assert!(progs.contains_key(&0x1100));
}

#[test]
fn pat_version_bump_removes_dropped_program() {
    let mut demuxer = Demuxer::new();
    demuxer
        .feed(&pat_packet_with_programs(&[(1, 0x1000), (2, 0x1100)], 0))
        .unwrap();
    demuxer
        .feed(&pat_packet_with_programs(&[(1, 0x1000)], 1))
        .unwrap();

    let progs = demuxer.programs_for_test();
    assert_eq!(
        progs.len(),
        1,
        "expected 1 tracker after program removal, got {}",
        progs.len()
    );
    assert!(
        progs.contains_key(&0x1000),
        "surviving program 1 tracker missing"
    );
    assert!(
        !progs.contains_key(&0x1100),
        "dropped program 2 tracker still present"
    );
}
