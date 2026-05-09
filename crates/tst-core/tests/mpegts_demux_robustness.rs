//! Robustness tests for `mpegts::demux::Demuxer` against non-conformant TS input.
//!
//! Lenient-mode coverage (the default `StrictMode::Off`):
//!
//! * Garbage prefix before the first 0x47 — demuxer slides past and recovers.
//! * Sustained garbage past `SYNC_SEARCH_WINDOW` — surfaces as
//!   `DemuxError::Unrecoverable`.
//! * Corrupt PAT section payload — checksum mismatch surfaces a
//!   `NonConformant` event (lenient mode keeps the loop alive).
//! * Continuity counter jump on a video PID — surfaces a `Discontinuity`
//!   event with `DiscontinuityKind::ContinuityJump`.
//!
//! Strict-mode rejection of these same issues is exercised in
//! `mpegts_demux_strict.rs`; here we only assert the lenient-mode contract.

use tst_core::mpegts::demux::{DemuxEvent, Demuxer, DiscontinuityKind};
use tst_core::mpegts::mux::{
    KlvStreamType, Muxer, MuxerConfigBuilder, VideoCodec as MuxVideoCodec,
};

fn drain(m: &mut Muxer) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1316];
    loop {
        let n = m.pull(&mut buf);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

/// Build a known-good clean MPEG-TS byte stream: PAT + PMT + two H.264 AUs
/// large enough to span multiple TS packets each. The size matters: the
/// `cc_jump_emits_discontinuity` test needs at least a couple consecutive
/// packets on the same video PID so the demuxer's continuity-counter
/// tracker has a baseline against which to detect a CC bump. Async-KLV PID
/// is configured so the PMT carries a KLVA registration descriptor, but
/// no KLV payload is pushed — the goal is just a valid stream to corrupt.
fn build_clean_stream() -> Vec<u8> {
    let cfg = MuxerConfigBuilder::default()
        .add_program(1, 0x1000)
        .add_video(0x100, MuxVideoCodec::H264)
        .add_klv(0x101, KlvStreamType::PrivateData, false)
        .end_program()
        .build()
        .unwrap();
    let mut m = Muxer::new(cfg).unwrap();
    // ~400-byte AU with AUD prefix + filler — produces 3 video TS packets.
    let mut au1 = vec![0x00, 0x00, 0x00, 0x01, 0x09, 0x10];
    au1.extend(std::iter::repeat(0xAB).take(400));
    m.push_video(&au1, 0, true).unwrap();
    let mut au2 = vec![0x00, 0x00, 0x00, 0x01, 0x09, 0x10];
    au2.extend(std::iter::repeat(0xCD).take(400));
    m.push_video(&au2, 90_000, false).unwrap();
    drain(&mut m)
}

#[test]
fn recovers_from_garbage_prefix() {
    // 500 bytes of 0xAA before the first 0x47 — well under the demuxer's
    // sync-search window, so it should slide past and parse the rest.
    let prefix = vec![0xAA; 500];
    let mut bytes = prefix;
    bytes.extend_from_slice(&build_clean_stream());

    let mut d = Demuxer::new();
    d.feed(&bytes).unwrap();
    // The clean stream has unbounded video PES; flush the partial so any
    // trailing Sample is emitted. ProgramMap arrives at PMT-parse time so
    // it's already queued, but flush keeps the round-trip pattern.
    d.flush();

    let mut saw_program_map = false;
    while let Some(e) = d.next_event() {
        if matches!(e, DemuxEvent::ProgramMap(_)) {
            saw_program_map = true;
        }
    }
    assert!(
        saw_program_map,
        "demuxer failed to recover from 500-byte garbage prefix"
    );
}

#[test]
fn unrecoverable_after_too_much_garbage() {
    // 188 * 1000 bytes of 0xAA — well past `SYNC_SEARCH_WINDOW`. Demuxer
    // never finds a 0x47 and must surface `DemuxError::Unrecoverable`.
    let huge_garbage = vec![0xAA; 188 * 1000];
    let mut d = Demuxer::new();
    let res = d.feed(&huge_garbage);
    assert!(
        res.is_err(),
        "expected Unrecoverable error on sustained garbage, got Ok"
    );
}

#[test]
fn corrupted_pat_surfaces_psi_checksum_event() {
    let mut bytes = build_clean_stream();
    // Find the first 0x47 PID-0 (PAT) packet and flip a byte inside the
    // PAT section (NOT the post-CRC32 stuffing). Layout for a single-program
    // PAT in a 188-byte TS packet with no adaptation field:
    //
    //   bytes 0..4   TS header (sync + flags + PID + cc)
    //   byte  4      pointer_field (0)
    //   bytes 5..    PAT section: table_id, section_length, transport_stream_id,
    //                version/cur/next, sec_num, last_sec_num, [program_num, PID]+,
    //                CRC32. Total ~21 bytes.
    //   bytes 26..   stuffing (0xFF), post-CRC32 — flipping these would NOT
    //                affect the checksum and the test would silently pass
    //                without exercising the path under test.
    //
    // Offset 10 lands in the `version/current_next_indicator` byte
    // (PAT section offset 5: TS header 0..4 + pointer_field at 4 + section
    // bytes table_id/section_length/transport_stream_id occupy 5..10),
    // inside the CRC-protected region. XOR with 0xFF flips the value but
    // leaves section_length intact so the parser still reaches the
    // checksum step.
    let mut patched = false;
    for i in (0..bytes.len()).step_by(188) {
        if bytes[i] == 0x47 && bytes[i + 1] & 0x1F == 0 && bytes[i + 2] == 0 {
            bytes[i + 10] ^= 0xFF;
            patched = true;
            break;
        }
    }
    assert!(patched, "no PAT packet found in clean stream");

    let mut d = Demuxer::new();
    d.feed(&bytes).unwrap();

    let mut saw_checksum_issue = false;
    while let Some(e) = d.next_event() {
        if let DemuxEvent::NonConformant { issue, .. } = e {
            if format!("{issue:?}").contains("PsiChecksumMismatch") {
                saw_checksum_issue = true;
            }
        }
    }
    assert!(
        saw_checksum_issue,
        "expected PsiChecksumMismatch NonConformant event from corrupt PAT"
    );
}

#[test]
fn cc_jump_emits_discontinuity() {
    let mut bytes = build_clean_stream();
    // Find the SECOND packet on a non-PSI PID and bump its CC by 5. The
    // demuxer's `check_continuity` only fires `ContinuityJump` if there's
    // a prior CC value cached for the PID — bumping the first packet on a
    // PID is silently ignored. We also need the PMT to have been parsed
    // before the bumped packet arrives (so `lookup_stream` resolves);
    // the PMT precedes any video packet in our clean stream, so finding
    // the second video packet by PID-seen tracking covers both
    // preconditions in one walk.
    //
    // CC lives in the low nibble of TS header byte 3.
    use std::collections::HashSet;
    let mut seen_pids: HashSet<u16> = HashSet::new();
    let mut patched = false;
    for i in (0..bytes.len()).step_by(188) {
        if bytes[i] != 0x47 {
            continue;
        }
        let pid = ((u16::from(bytes[i + 1] & 0x1F)) << 8) | u16::from(bytes[i + 2]);
        if pid == 0x0000 || pid == 0x1000 {
            // PAT or PMT — skip.
            continue;
        }
        if seen_pids.contains(&pid) {
            bytes[i + 3] = (bytes[i + 3] & 0xF0) | (((bytes[i + 3] & 0x0F) + 5) & 0x0F);
            patched = true;
            break;
        }
        seen_pids.insert(pid);
    }
    assert!(patched, "no second non-PSI packet found in clean stream");

    let mut d = Demuxer::new();
    d.feed(&bytes).unwrap();

    let mut saw_jump = false;
    while let Some(e) = d.next_event() {
        if matches!(
            e,
            DemuxEvent::Discontinuity {
                kind: DiscontinuityKind::ContinuityJump { .. },
                ..
            }
        ) {
            saw_jump = true;
        }
    }
    assert!(
        saw_jump,
        "expected ContinuityJump discontinuity from CC bump"
    );
}
