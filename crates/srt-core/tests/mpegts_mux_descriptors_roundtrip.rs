//! Round-trip caller-supplied PMT descriptors through Muxer → Demuxer.
//!
//! Asserts:
//!   - Each StreamInfo carries the descriptors the caller pushed in
//!     (correct count, correct tags, correct payloads).
//!   - extract_user_label decodes user_private labels.
//!   - Auto-emit suppression on caller-supplied KLVA Registration works.

use srt_core::mpegts::demux::psi::extract_user_label;
use srt_core::mpegts::demux::{DemuxEvent, Demuxer};
use srt_core::mpegts::descriptors;
use srt_core::mpegts::mux::{Config, KlvStreamType, Muxer, VideoCodec};

fn drive_psi(cfg: Config) -> Vec<u8> {
    let mut mux = Muxer::new(cfg).unwrap();
    // Push one tiny video NAL to advance PTS past the PSI threshold.
    mux.push_video(&[0, 0, 0, 1, 0x09, 0x10], 9000, true).ok();
    let mut buf = vec![0u8; 188 * 32];
    let n = mux.pull(&mut buf);
    buf.truncate(n);
    buf
}

fn drain_events(bytes: &[u8]) -> Vec<DemuxEvent> {
    let mut demuxer = Demuxer::new();
    demuxer.feed(bytes).unwrap();
    let mut out = Vec::new();
    while let Some(ev) = demuxer.next_event() {
        out.push(ev);
    }
    out
}

#[test]
fn family_b_klv_descriptor_stack_round_trips() {
    let cfg = Config::builder()
        .add_program(1, 0x1000)
        .add_video(0x100, VideoCodec::H264)
        .stream_descriptors_for_video(0, vec![descriptors::user_private(b"EO 1080p")])
        .add_klv(0x102, KlvStreamType::SynchronousMetadata, true)
        .stream_descriptors_for_klv(
            0,
            vec![
                descriptors::metadata_klva(0x00),
                descriptors::metadata_std(0, 0, 0),
                descriptors::user_private(b"KLV_SYNC"),
            ],
        )
        .end_program()
        .build()
        .unwrap();

    let bytes = drive_psi(cfg);
    let events = drain_events(&bytes);
    let pm = events
        .iter()
        .find_map(|e| match e {
            DemuxEvent::ProgramMap(pm) => Some(pm),
            _ => None,
        })
        .expect("ProgramMap emitted");

    // Video PID — one tag 0xFF descriptor.
    let video = pm.streams.iter().find(|s| s.pid == 0x100).unwrap();
    assert_eq!(video.raw_descriptors.len(), 1);
    assert_eq!(video.raw_descriptors[0].tag, 0xFF);
    assert_eq!(
        extract_user_label(&video.raw_descriptors).as_deref(),
        Some("EO 1080p")
    );

    // KLV PID — three descriptors in caller order: 0x26, 0x27, 0xFF.
    let klv = pm.streams.iter().find(|s| s.pid == 0x102).unwrap();
    assert_eq!(klv.raw_descriptors.len(), 3);
    assert_eq!(klv.raw_descriptors[0].tag, 0x26);
    assert_eq!(klv.raw_descriptors[1].tag, 0x27);
    assert_eq!(klv.raw_descriptors[2].tag, 0xFF);
    // Metadata descriptor wins over user_private per priority order.
    assert_eq!(
        extract_user_label(&klv.raw_descriptors).as_deref(),
        Some("KLV")
    );
}

#[test]
fn klva_auto_emit_suppressed_when_caller_supplies_registration() {
    let cfg = Config::builder()
        .add_program(1, 0x1000)
        .add_video(0x100, VideoCodec::H264)
        .add_klv(0x101, KlvStreamType::PrivateData, false)
        .stream_descriptors_for_klv(0, vec![descriptors::registration(*b"KLVA", &[])])
        .end_program()
        .build()
        .unwrap();

    let bytes = drive_psi(cfg);
    let events = drain_events(&bytes);
    let pm = events
        .iter()
        .find_map(|e| match e {
            DemuxEvent::ProgramMap(pm) => Some(pm),
            _ => None,
        })
        .expect("ProgramMap emitted");

    let klv = pm.streams.iter().find(|s| s.pid == 0x101).unwrap();
    // Exactly ONE Registration descriptor — auto-emit suppressed because
    // the caller supplied their own. Two would mean we duplicated.
    let regs: Vec<_> = klv
        .raw_descriptors
        .iter()
        .filter(|d| d.tag == 0x05)
        .collect();
    assert_eq!(
        regs.len(),
        1,
        "auto-emit was not suppressed (found {} Registration descriptors)",
        regs.len()
    );
}

#[test]
fn family_a_hdmv_video_registration_round_trips() {
    // Replicate the bench-11 / N4717V / N77HS shape: video PID with
    // Registration "HDMV" + 4 trailing bytes.
    let cfg = Config::builder()
        .add_program(1, 0x1000)
        .add_video(0x100, VideoCodec::H264)
        .stream_descriptors_for_video(
            0,
            vec![descriptors::registration(
                *b"HDMV",
                &[0xFF, 0x1B, 0x44, 0x3F],
            )],
        )
        .end_program()
        .build()
        .unwrap();

    let bytes = drive_psi(cfg);
    let events = drain_events(&bytes);
    let pm = events
        .iter()
        .find_map(|e| match e {
            DemuxEvent::ProgramMap(pm) => Some(pm),
            _ => None,
        })
        .expect("ProgramMap emitted");
    let video = pm.streams.iter().find(|s| s.pid == 0x100).unwrap();
    assert_eq!(video.raw_descriptors.len(), 1);
    let r = &video.raw_descriptors[0];
    assert_eq!(r.tag, 0x05);
    assert_eq!(&r.data[..4], b"HDMV");
    assert_eq!(&r.data[4..], &[0xFF, 0x1B, 0x44, 0x3F]);
}

#[tracing_test::traced_test]
#[test]
fn non_klva_registration_on_klv_pid_logs_warning() {
    let cfg = Config::builder()
        .add_program(1, 0x1000)
        .add_klv(0x101, KlvStreamType::PrivateData, false)
        .stream_descriptors_for_klv(0, vec![descriptors::registration(*b"VEND", &[])])
        .end_program()
        .build()
        .unwrap();
    let _ = Muxer::new(cfg).unwrap();
    assert!(logs_contain("non-KLVA format_identifier"));
}
