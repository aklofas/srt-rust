//! Integration tests for multi-program TS muxing.
//!
//! Verifies that PAT carries N program entries and that one PMT is emitted per
//! program per PSI tick.

mod common;

use common::synthetic_nal;
use srt_core::mpegts::mux::{Config, KlvStreamType, Muxer, ProgramConfig, StreamSpec, VideoCodec};

/// Two-program config:
///   prog 1 (H.264 + KLV) at PMT=0x1000, video=0x1011, klv=0x1031
///   prog 2 (H.265 + KLV) at PMT=0x1100, video=0x1111, klv=0x1131
///
/// All PIDs are in the valid user range 0x0010..=0x1FFE.
fn two_program_config() -> Config {
    Config {
        programs: vec![
            ProgramConfig {
                program_number: 1,
                pmt_pid: 0x1000,
                streams: vec![
                    StreamSpec::Video {
                        pid: 0x1011,
                        codec: VideoCodec::H264,
                    },
                    StreamSpec::Klv {
                        pid: 0x1031,
                        stream_type: KlvStreamType::PrivateData,
                        carries_pts: false,
                    },
                ],
                pcr_pid: None,
                program_descriptors: Vec::new(),
                stream_descriptors: vec![Vec::new(), Vec::new()],
            },
            ProgramConfig {
                program_number: 2,
                pmt_pid: 0x1100,
                streams: vec![
                    StreamSpec::Video {
                        pid: 0x1111,
                        codec: VideoCodec::H265,
                    },
                    StreamSpec::Klv {
                        pid: 0x1131,
                        stream_type: KlvStreamType::PrivateData,
                        carries_pts: false,
                    },
                ],
                pcr_pid: None,
                program_descriptors: Vec::new(),
                stream_descriptors: vec![Vec::new(), Vec::new()],
            },
        ],
        ..Config::default()
    }
}

fn drain_all(mux: &mut Muxer) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1316];
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&buf[..n]);
    }
}

/// Trigger PSI emission for program 0 by pushing a video frame (PTS=0 forces
/// the first-ever PSI emit regardless of interval).
fn trigger_psi(mux: &mut Muxer) -> Vec<u8> {
    let nal = synthetic_nal::h264_au(200, true);
    // Using prog 0 video stream with handle pack(0,0) via push_video_to.
    // For two-program muxers, push_video is ambiguous (2 video streams), so
    // we use the handle-based variant.
    use srt_core::mpegts::mux::VideoStreamHandle;
    let handle = VideoStreamHandle::pack(0, 0);
    mux.push_video_to(handle, &nal, 0, true).unwrap();
    drain_all(mux)
}

#[test]
fn pat_carries_two_program_entries() {
    let mut muxer = Muxer::new(two_program_config()).unwrap();

    // Push a video frame on prog 0 — psi_last[0] == None so PSI is due immediately.
    let out = trigger_psi(&mut muxer);

    let pat_packet = out
        .chunks_exact(188)
        .find(|p| {
            let pid = ((p[1] as u16 & 0x1F) << 8) | p[2] as u16;
            pid == 0x0000
        })
        .expect("PAT packet must be emitted");

    // TS header = bytes 0..3 (4 bytes)
    // pointer field = byte 4 (= 0x00, so section starts at byte 5)
    // PAT section layout from byte 5:
    //   table_id(1)=5, section_syntax+length(2)=6..7,
    //   tsid(2)=8..9, ver/cni(1)=10, sect_no(1)=11, last_sect(1)=12
    //   program loop starts at byte 13
    let prog_loop_start = 13;
    let prog1_num =
        u16::from_be_bytes([pat_packet[prog_loop_start], pat_packet[prog_loop_start + 1]]);
    let prog1_pid = u16::from_be_bytes([
        pat_packet[prog_loop_start + 2] & 0x1F,
        pat_packet[prog_loop_start + 3],
    ]);
    let prog2_num = u16::from_be_bytes([
        pat_packet[prog_loop_start + 4],
        pat_packet[prog_loop_start + 5],
    ]);
    let prog2_pid = u16::from_be_bytes([
        pat_packet[prog_loop_start + 6] & 0x1F,
        pat_packet[prog_loop_start + 7],
    ]);

    assert_eq!(prog1_num, 1, "first program_number must be 1");
    assert_eq!(prog1_pid, 0x1000, "first pmt_pid must be 0x1000");
    assert_eq!(prog2_num, 2, "second program_number must be 2");
    assert_eq!(prog2_pid, 0x1100, "second pmt_pid must be 0x1100");
}

#[test]
fn both_pmts_emitted_per_psi_tick() {
    let mut muxer = Muxer::new(two_program_config()).unwrap();

    let out = trigger_psi(&mut muxer);

    let pmt1_count = out
        .chunks_exact(188)
        .filter(|p| (((p[1] as u16 & 0x1F) << 8) | p[2] as u16) == 0x1000)
        .count();
    let pmt2_count = out
        .chunks_exact(188)
        .filter(|p| (((p[1] as u16 & 0x1F) << 8) | p[2] as u16) == 0x1100)
        .count();

    assert_eq!(
        pmt1_count, 1,
        "PMT for program 1 (PID 0x1000) should be emitted once per tick"
    );
    assert_eq!(
        pmt2_count, 1,
        "PMT for program 2 (PID 0x1100) should be emitted once per tick"
    );
}

#[test]
fn pmt2_carries_correct_program_number() {
    // Verify that the PMT emitted on PID 0x2000 encodes program_number=2 in
    // its section header (bytes 8..9 of the PMT section body, which starts at
    // payload[1] = packet[5]).
    let mut muxer = Muxer::new(two_program_config()).unwrap();
    let out = trigger_psi(&mut muxer);

    let pmt2_packet = out
        .chunks_exact(188)
        .find(|p| (((p[1] as u16 & 0x1F) << 8) | p[2] as u16) == 0x1100)
        .expect("PMT for program 2 (PID 0x1100) must be emitted");

    // PMT section starts at pkt[5] (4-byte TS header + 1-byte pointer field).
    // section layout: table_id(1)=pkt[5], section_syntax+length(2)=pkt[6..7],
    //   program_number(2)=pkt[8..9]
    let program_number = u16::from_be_bytes([pmt2_packet[8], pmt2_packet[9]]);
    assert_eq!(
        program_number, 2,
        "PMT on PID 0x2000 must encode program_number=2"
    );
}

#[test]
fn single_program_pat_unchanged() {
    // Single-program config must produce the same PAT byte layout as before:
    // one program entry at bytes 13..16 of the PAT packet.
    let mut muxer = Muxer::new(Config::default()).unwrap();
    let nal = synthetic_nal::h264_au(200, true);
    muxer.push_video(&nal, 0, true).unwrap();
    let out = drain_all(&mut muxer);

    let pat_packet = out
        .chunks_exact(188)
        .find(|p| ((p[1] as u16 & 0x1F) << 8) | p[2] as u16 == 0x0000)
        .expect("PAT must be emitted for single-program config");

    // Single program entry at bytes 13..16.
    let prog_num = u16::from_be_bytes([pat_packet[13], pat_packet[14]]);
    let pmt_pid = u16::from_be_bytes([pat_packet[15] & 0x1F, pat_packet[16]]);
    assert_eq!(prog_num, 1, "single-program PAT: program_number must be 1");
    assert_eq!(
        pmt_pid, 0x1000,
        "single-program PAT: pmt_pid must be 0x1000"
    );

    // Bytes after the one program entry + CRC should be 0xFF padding.
    // single program: 1 entry (4 bytes) + CRC (4 bytes) = 8 bytes after the 8-byte PSI header.
    // Padding starts at byte 13 + 8 = 21.
    assert_eq!(
        pat_packet[21], 0xFF,
        "single-program PAT must have 0xFF padding after the one entry + CRC"
    );
}

#[test]
fn config_builder_emits_multi_program_config() {
    let config = Config::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264)
        .add_klv(0x1031, KlvStreamType::PrivateData, false)
        .end_program()
        .add_program(2, 0x1100)
        .add_video(0x1111, VideoCodec::H265)
        .add_klv(0x1131, KlvStreamType::PrivateData, false)
        .end_program()
        .build()
        .unwrap();

    assert_eq!(config.programs.len(), 2);
    assert_eq!(config.programs[0].program_number, 1);
    assert_eq!(config.programs[0].pmt_pid, 0x1000);
    assert_eq!(config.programs[0].streams.len(), 2);
    assert_eq!(config.programs[1].program_number, 2);
    assert_eq!(config.programs[1].pmt_pid, 0x1100);
}

#[test]
fn push_video_to_routes_to_correct_program_and_pid() {
    let mut muxer = Muxer::new(two_program_config()).unwrap();
    let prog1_video = muxer.video_handles_for_program(1).unwrap();
    let prog2_video = muxer.video_handles_for_program(2).unwrap();
    assert_eq!(prog1_video.len(), 1);
    assert_eq!(prog2_video.len(), 1);

    // Annex B IDR NAL — valid for both H.264 (prog 1) and H.265 (prog 2):
    // validate_annex_b only checks for the start code, not the codec.
    let nal = synthetic_nal::h264_au(64, true);
    muxer
        .push_video_to(prog1_video[0], &nal, 90_000, true)
        .unwrap();
    muxer
        .push_video_to(prog2_video[0], &nal, 90_000, true)
        .unwrap();

    let mut out = vec![0u8; 64 * 188];
    let n = muxer.pull(&mut out);
    let out = &out[..n];

    let prog1_pid_count = out
        .chunks_exact(188)
        .filter(|p| (((p[1] as u16 & 0x1F) << 8) | p[2] as u16) == 0x1011)
        .count();
    let prog2_pid_count = out
        .chunks_exact(188)
        .filter(|p| (((p[1] as u16 & 0x1F) << 8) | p[2] as u16) == 0x1111)
        .count();
    assert!(
        prog1_pid_count > 0,
        "video on program 1 PID 0x1011 must be emitted"
    );
    assert!(
        prog2_pid_count > 0,
        "video on program 2 PID 0x1111 must be emitted"
    );
}

#[test]
fn bare_push_video_returns_ambiguous_target_with_two_programs() {
    use srt_core::error::MuxError;
    let mut muxer = Muxer::new(two_program_config()).unwrap();
    let nal = synthetic_nal::h264_au(64, true);
    let err = muxer.push_video(&nal, 90_000, true).unwrap_err();
    assert!(
        matches!(err, MuxError::AmbiguousTarget { count: 2, .. }),
        "expected AmbiguousTarget {{ count: 2, .. }}, got {err:?}"
    );
}

#[test]
fn config_builder_descriptors_for_video_attaches_to_correct_program() {
    use srt_core::mpegts::descriptors as desc;
    let config = Config::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264)
        .stream_descriptors_for_video(0, vec![desc::user_private(b"EO 1080p")])
        .end_program()
        .add_program(2, 0x1100)
        .add_video(0x1111, VideoCodec::H265)
        .stream_descriptors_for_video(0, vec![desc::user_private(b"EO 4K")])
        .end_program()
        .build()
        .unwrap();

    assert_eq!(
        config.programs[0].stream_descriptors[0][0],
        desc::user_private(b"EO 1080p")
    );
    assert_eq!(
        config.programs[1].stream_descriptors[0][0],
        desc::user_private(b"EO 4K")
    );
}

// ── PCR pacing tests ─────────────────────────────────────────────────────────

/// Return true if any 188-byte TS packet in `data` with the given PID carries
/// a PCR: adaptation field present, AF length ≥ 7, and PCR_flag set.
fn has_pcr_on_pid(data: &[u8], pid: u16) -> bool {
    data.chunks_exact(188).any(|p| {
        let p_pid = ((p[1] as u16 & 0x1F) << 8) | p[2] as u16;
        p_pid == pid
            && (p[3] & 0x20) != 0  // adaptation_field_control has AF
            && p[4] >= 7           // AF length covers at least 1 + 6 PCR bytes
            && (p[5] & 0x10) != 0 // PCR_flag set
    })
}

#[test]
fn per_program_pcr_pids_resolved_independently() {
    // Program 1 pins PCR to its KLV PID (0x1031).  Program 2 leaves
    // pcr_pid = None, so it auto-falls back to its first video PID (0x1111).
    // We verify the actual byte output: PCR-bearing packets must appear on
    // 0x1031 for program 1 and on 0x1111 for program 2, and NOT on each
    // program's other streams.
    let mut config = two_program_config();
    config.programs[0].pcr_pid = Some(0x1031); // program 1 PCR → KLV PID

    let mut muxer = Muxer::new(config).unwrap();
    let p1_video = muxer.video_handles_for_program(1).unwrap()[0];
    let p1_klv = muxer.klv_handles_for_program(1).unwrap()[0];
    let p2_video = muxer.video_handles_for_program(2).unwrap()[0];
    let p2_klv = muxer.klv_handles_for_program(2).unwrap()[0];

    let nal = synthetic_nal::h264_au(64, true);
    let klv = synthetic_nal::klv_blob(32);

    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 188];
    for tick in 0..30i64 {
        let pts = tick * 3_003;
        muxer.push_video_to(p1_video, &nal, pts, tick == 0).unwrap();
        muxer.push_klv_to(p1_klv, &klv, pts).unwrap();
        muxer.push_video_to(p2_video, &nal, pts, tick == 0).unwrap();
        muxer.push_klv_to(p2_klv, &klv, pts).unwrap();
        loop {
            let n = muxer.pull(&mut buf);
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
    }

    // Program 1: PCR must appear on the pinned KLV PID, not the video PID.
    assert!(
        has_pcr_on_pid(&out, 0x1031),
        "program 1 PCR PID 0x1031 (KLV, pinned) must carry PCR-bearing packets"
    );
    assert!(
        !has_pcr_on_pid(&out, 0x1011),
        "program 1 video PID 0x1011 must NOT carry PCR when PCR is pinned to KLV"
    );

    // Program 2: PCR must appear on the auto-fallback video PID.
    assert!(
        has_pcr_on_pid(&out, 0x1111),
        "program 2 PCR PID 0x1111 (video, auto-fallback) must carry PCR-bearing packets"
    );
    assert!(
        !has_pcr_on_pid(&out, 0x1131),
        "program 2 KLV PID 0x1131 must NOT carry PCR when PCR is on video"
    );
}

#[test]
fn per_program_pcr_emitted_on_each_program_pid() {
    // Default two-program config: both programs leave pcr_pid = None, so
    // each falls back to its first video PID (0x1011 and 0x1111 respectively).
    // Push enough frames (~1 second of 30 fps) to span several PCR intervals
    // (default interval is 40 ms) and confirm PCR-bearing packets land on
    // both video PIDs independently.
    let mut muxer = Muxer::new(two_program_config()).unwrap();
    let p1_video = muxer.video_handles_for_program(1).unwrap()[0];
    let p2_video = muxer.video_handles_for_program(2).unwrap()[0];

    let nal = synthetic_nal::h264_au(64, true);

    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 188];
    for tick in 0..30i64 {
        let pts = tick * 3_003; // ~33 ms per frame at 90 kHz
        muxer.push_video_to(p1_video, &nal, pts, tick == 0).unwrap();
        muxer.push_video_to(p2_video, &nal, pts, tick == 0).unwrap();
        loop {
            let n = muxer.pull(&mut buf);
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
    }

    assert!(
        has_pcr_on_pid(&out, 0x1011),
        "program 1 PCR PID 0x1011 must carry at least one PCR-bearing TS packet"
    );
    assert!(
        has_pcr_on_pid(&out, 0x1111),
        "program 2 PCR PID 0x1111 must carry at least one PCR-bearing TS packet"
    );
}
