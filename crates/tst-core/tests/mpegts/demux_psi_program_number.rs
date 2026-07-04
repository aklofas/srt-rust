//! REF-PSI-01 — PMT body `program_number` must match PAT assignment.
//!
//! A validly-checksummed PMT whose body carries `program_number = M` that
//! arrives on a PMT PID whose PAT entry declared `program_number = N` (N ≠ M)
//! must be rejected: the demuxer emits a
//! `NonConformantIssue::PmtProgramNumberMismatch` event and DOES NOT adopt the
//! bogus topology (no `ProgramMap` event, no video `Sample` events from the
//! mislabeled program's streams).
//!
//! `StrictMode::Full` additionally converts the mismatch into a fatal
//! `DemuxError::StrictRejection`.

use tst_core::error::DemuxError;
use tst_core::mpegts::common::crc32::crc32_mpeg2;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, DemuxerConfig, NonConformantIssue, StrictMode};

// ─────────────────────────────────────────────────────────────────────────────
// Packet / section builders (self-contained, mirrors demux_strict.rs helpers)
// ─────────────────────────────────────────────────────────────────────────────

/// Build a valid-CRC PAT section for one program.
///
/// Declares `program_number` on PMT PID `pmt_pid`. Returns raw section bytes
/// (table_id .. CRC32 inclusive).
fn build_pat_section(program_number: u16, pmt_pid: u16) -> Vec<u8> {
    // section_length = 5 (fixed post-length fields) + 4 (one program entry) + 4 (CRC) = 13
    let section_length: u16 = 13;
    let mut sec = Vec::with_capacity(17);
    sec.push(0x00); // table_id = PAT
    sec.push(0xB0 | (((section_length >> 8) & 0x0F) as u8));
    sec.push((section_length & 0xFF) as u8);
    sec.push(0x00); // transport_stream_id high
    sec.push(0x01); // transport_stream_id low
    sec.push(0xC1); // reserved + version=0 + current_next_indicator=1
    sec.push(0x00); // section_number
    sec.push(0x00); // last_section_number
    // Program entry
    sec.push((program_number >> 8) as u8);
    sec.push((program_number & 0xFF) as u8);
    sec.push(0xE0 | ((pmt_pid >> 8) as u8 & 0x1F));
    sec.push((pmt_pid & 0xFF) as u8);
    let crc = crc32_mpeg2(&sec);
    sec.push((crc >> 24) as u8);
    sec.push((crc >> 16) as u8);
    sec.push((crc >> 8) as u8);
    sec.push(crc as u8);
    sec
}

/// Build a valid-CRC PMT section with a single H.264 video elementary stream.
///
/// `program_number` goes into the PMT body's `program_number` field — caller
/// sets this to a value that may differ from the PAT's declared number to
/// trigger REF-PSI-01.
fn build_pmt_section(program_number: u16, pcr_pid: u16, video_pid: u16) -> Vec<u8> {
    // section_length = 9 (fixed body after section_length field) + 5 (one ES entry) + 4 (CRC)
    // = 18; section_length covers from program_number to end of CRC.
    let section_length: u16 = 18;
    let mut sec = Vec::with_capacity(21);
    sec.push(0x02); // table_id = PMT
    sec.push(0xB0 | (((section_length >> 8) & 0x0F) as u8));
    sec.push((section_length & 0xFF) as u8);
    sec.push((program_number >> 8) as u8);
    sec.push((program_number & 0xFF) as u8);
    sec.push(0xC1); // reserved + version=0 + current_next_indicator=1
    sec.push(0x00); // section_number
    sec.push(0x00); // last_section_number
    sec.push(0xE0 | ((pcr_pid >> 8) as u8 & 0x1F));
    sec.push((pcr_pid & 0xFF) as u8);
    sec.push(0xF0); // program_info_length high (reserved + 0)
    sec.push(0x00); // program_info_length low
    // ES entry: H.264 (0x1B) on video_pid, no descriptors
    sec.push(0x1B);
    sec.push(0xE0 | ((video_pid >> 8) as u8 & 0x1F));
    sec.push((video_pid & 0xFF) as u8);
    sec.push(0xF0); // ES_info_length high
    sec.push(0x00); // ES_info_length low
    let crc = crc32_mpeg2(&sec);
    sec.push((crc >> 24) as u8);
    sec.push((crc >> 16) as u8);
    sec.push((crc >> 8) as u8);
    sec.push(crc as u8);
    sec
}

/// Pack a raw PSI section into one or more 188-byte TS packets on `pid`.
///
/// First packet: PUSI=1 + pointer_field=0. Continuation packets: PUSI=0.
/// Tail padding with 0xFF. Continuity counter starts at `cc_start`.
fn pack_section(pid: u16, section: &[u8], cc_start: u8) -> Vec<[u8; 188]> {
    let mut pkts = Vec::new();
    let mut cc = cc_start & 0x0F;
    let mut pos = 0usize;
    let pid_hi = (pid >> 8) as u8 & 0x1F;
    let pid_lo = (pid & 0xFF) as u8;

    // First packet with PUSI=1 and pointer_field.
    {
        let mut pkt = [0xFFu8; 188];
        pkt[0] = 0x47;
        pkt[1] = 0x40 | pid_hi; // PUSI=1
        pkt[2] = pid_lo;
        pkt[3] = 0x10 | cc;
        pkt[4] = 0x00; // pointer_field
        let chunk = section.len().min(183);
        pkt[5..5 + chunk].copy_from_slice(&section[pos..pos + chunk]);
        pos += chunk;
        pkts.push(pkt);
        cc = (cc + 1) & 0x0F;
    }

    // Continuation packets.
    while pos < section.len() {
        let mut pkt = [0xFFu8; 188];
        pkt[0] = 0x47;
        pkt[1] = pid_hi; // PUSI=0
        pkt[2] = pid_lo;
        pkt[3] = 0x10 | cc;
        let chunk = (section.len() - pos).min(184);
        pkt[4..4 + chunk].copy_from_slice(&section[pos..pos + chunk]);
        pos += chunk;
        pkts.push(pkt);
        cc = (cc + 1) & 0x0F;
    }

    pkts
}

/// Drain all queued events; return `(nonconformant_issues, program_map_count)`.
fn drain_events(demuxer: &mut Demuxer) -> (Vec<NonConformantIssue>, usize) {
    let mut issues = Vec::new();
    let mut maps = 0usize;
    while let Some(ev) = demuxer.next_event() {
        match ev {
            DemuxEvent::NonConformant { issue, .. } => issues.push(issue),
            DemuxEvent::ProgramMap(_) => maps += 1,
            _ => {}
        }
    }
    (issues, maps)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

const PMT_PID: u16 = 0x0100;
const PCR_PID: u16 = 0x0200;
const VIDEO_PID: u16 = 0x0201;
const PAT_PROGRAM: u16 = 1;
const PMT_PROGRAM: u16 = 99; // deliberately ≠ PAT_PROGRAM

/// Build the raw bytes for a PAT declaring program 1 on PMT_PID, followed by
/// a PMT on PMT_PID whose body claims program 99 (≠ 1).
fn build_mismatched_stream() -> Vec<u8> {
    let pat_sec = build_pat_section(PAT_PROGRAM, PMT_PID);
    let pmt_sec = build_pmt_section(PMT_PROGRAM, PCR_PID, VIDEO_PID);

    let mut buf = Vec::new();
    for pkt in pack_section(0x0000, &pat_sec, 0) {
        buf.extend_from_slice(&pkt);
    }
    for pkt in pack_section(PMT_PID, &pmt_sec, 0) {
        buf.extend_from_slice(&pkt);
    }
    buf
}

/// REF-PSI-01 lenient (default StrictMode::Off):
///   - a `PmtProgramNumberMismatch` event is emitted,
///   - NO `ProgramMap` event is emitted (topology NOT adopted),
///   - the video PID is not registered (no Sample events possible).
#[test]
fn pmt_program_number_mismatch_emits_nonconformant_event() {
    let bytes = build_mismatched_stream();
    let mut d = Demuxer::new();
    d.feed(&bytes).unwrap();
    let (issues, map_count) = drain_events(&mut d);

    // Exactly one PmtProgramNumberMismatch issue.
    let mismatch_issues: Vec<_> = issues
        .iter()
        .filter(|i| matches!(i, NonConformantIssue::PmtProgramNumberMismatch { .. }))
        .collect();
    assert_eq!(
        mismatch_issues.len(),
        1,
        "expected exactly one PmtProgramNumberMismatch, got issues: {issues:?}"
    );

    // The mismatch event must carry the right pids/programs.
    if let NonConformantIssue::PmtProgramNumberMismatch {
        pid,
        pat_program,
        pmt_program,
    } = mismatch_issues[0]
    {
        assert_eq!(*pid, PMT_PID, "pid must be the PMT PID");
        assert_eq!(
            *pat_program, PAT_PROGRAM,
            "pat_program must match the PAT entry"
        );
        assert_eq!(
            *pmt_program, PMT_PROGRAM,
            "pmt_program must be the PMT body value"
        );
    }

    // Topology must NOT be adopted under the false identity.
    assert_eq!(
        map_count, 0,
        "no ProgramMap must be emitted for a mismatched PMT"
    );
}

/// REF-PSI-01 Display: the issue formats to a string that mentions the PID
/// and both program numbers.
#[test]
fn pmt_program_number_mismatch_display() {
    let issue = NonConformantIssue::PmtProgramNumberMismatch {
        pid: 0x0100,
        pat_program: 1,
        pmt_program: 99,
    };
    let s = format!("{issue}");
    assert!(s.contains("0x0100"), "Display must mention the PID: {s}");
    assert!(
        s.contains("program_number=1"),
        "Display must mention pat_program: {s}"
    );
    assert!(
        s.contains("program_number=99"),
        "Display must mention pmt_program: {s}"
    );
}

/// REF-PSI-01 strict rejection: StrictMode::Full converts the mismatch into a
/// fatal `DemuxError::StrictRejection` returned from `feed`.
#[test]
fn pmt_program_number_mismatch_strict_full_rejects() {
    let bytes = build_mismatched_stream();
    let mut d = Demuxer::with_config(DemuxerConfig::builder().strict(StrictMode::Full).build());
    // PAT may succeed; the PMT is what triggers the rejection.
    // Feed the whole buffer and check for StrictRejection.
    let result = d.feed(&bytes);
    assert!(
        matches!(result, Err(DemuxError::StrictRejection(_))),
        "StrictMode::Full must reject on PmtProgramNumberMismatch; got {result:?}"
    );
}

/// Regression guard: a well-formed PMT where program_number MATCHES the PAT
/// must still emit a ProgramMap and must NOT emit a PmtProgramNumberMismatch.
#[test]
fn pmt_program_number_matches_pat_is_accepted() {
    let pat_sec = build_pat_section(PAT_PROGRAM, PMT_PID);
    // PMT body also claims PAT_PROGRAM — correct.
    let pmt_sec = build_pmt_section(PAT_PROGRAM, PCR_PID, VIDEO_PID);

    let mut buf = Vec::new();
    for pkt in pack_section(0x0000, &pat_sec, 0) {
        buf.extend_from_slice(&pkt);
    }
    for pkt in pack_section(PMT_PID, &pmt_sec, 0) {
        buf.extend_from_slice(&pkt);
    }

    let mut d = Demuxer::new();
    d.feed(&buf).unwrap();
    let (issues, map_count) = drain_events(&mut d);

    let mismatch_issues: Vec<_> = issues
        .iter()
        .filter(|i| matches!(i, NonConformantIssue::PmtProgramNumberMismatch { .. }))
        .collect();
    assert!(
        mismatch_issues.is_empty(),
        "matching PMT must not trigger PmtProgramNumberMismatch"
    );
    assert_eq!(
        map_count, 1,
        "matching PMT must emit exactly one ProgramMap"
    );
}
