//! `MuxerConfig::from_program_map` — rebuilding a single-program muxer
//! config from a demuxed `ProgramMap`.
//!
//! Covers the strict-by-default offender contract (Unknown / DVB-subtitle
//! streams error unless their kinds are passed in `drop`), the PCR copy
//! rule (the demuxed `pcr_pid` is copied only when it lands on a kept
//! non-KLV stream), ISO 639 language recovery from raw PMT descriptors,
//! and a real mux→demux→`from_program_map` round-trip.

use tst_core::MuxError;
use tst_core::mpegts::common::{Pts90khz, StreamTypeCode};
use tst_core::mpegts::demux::{
    AudioCodec as DemuxAudio, DemuxEvent, Demuxer, ProgramMap, StreamInfo, StreamKind as DemuxKind,
    StreamKindTag, SubtitleCodec as DemuxSub, VideoCodec as DemuxVideo,
};
use tst_core::mpegts::descriptors::RawDescriptor;
use tst_core::mpegts::mux::{
    AudioCodec, KlvStreamType, Muxer, MuxerConfig, MuxerProgramConfigBuilder, StreamSpec,
    SubtitleCodec, VideoCodec,
};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// `StreamInfo` literal with no descriptors (program 1).
fn stream(pid: u16, stream_type: u8, kind: DemuxKind) -> StreamInfo {
    StreamInfo {
        pid,
        stream_type: StreamTypeCode::from_byte(stream_type),
        kind,
        program_number: 1,
        raw_descriptors: Vec::new(),
    }
}

/// `ProgramMap` literal for program 1 with no KLV links.
fn pm(pmt_pid: u16, pcr_pid: u16, streams: Vec<StreamInfo>) -> ProgramMap {
    ProgramMap {
        program_number: 1,
        pcr_pid,
        pmt_pid,
        streams,
        klv_links: Vec::new(),
    }
}

/// PID of a mux-side `StreamSpec` regardless of variant.
fn spec_pid(s: &StreamSpec) -> u16 {
    match s {
        StreamSpec::Video { pid, .. }
        | StreamSpec::Klv { pid, .. }
        | StreamSpec::Audio { pid, .. }
        | StreamSpec::Subtitle { pid, .. } => *pid,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Conversion matrix (synthetic ProgramMap literals)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn happy_path_video_sync_klv_audio_with_language() {
    let mut audio = stream(0x103, 0x0F, DemuxKind::Audio(DemuxAudio::Aac));
    audio.raw_descriptors = vec![RawDescriptor {
        tag: 0x0A,
        data: b"eng\x00".to_vec(),
    }];
    let p = pm(
        0x100,
        0x101,
        vec![
            stream(0x101, 0x1B, DemuxKind::Video(DemuxVideo::H264)),
            stream(
                0x102,
                0x15,
                DemuxKind::KlvSync {
                    declared_link: Some(0x101),
                },
            ),
            audio,
        ],
    );

    let cfg = MuxerConfig::from_program_map(&p, &[]).expect("fully representable program map");
    assert_eq!(cfg.programs.len(), 1, "single-program conversion");
    let prog = &cfg.programs[0];
    assert_eq!(prog.program_number, 1);
    assert_eq!(prog.pmt_pid, 0x100);
    // Demuxed pcr_pid lands on the (kept, non-KLV) video stream → copied.
    assert_eq!(prog.pcr_pid, Some(0x101));
    assert_eq!(prog.streams.len(), 3);
    assert!(prog.streams.contains(&StreamSpec::Video {
        pid: 0x101,
        codec: VideoCodec::H264,
    }));
    assert!(prog.streams.contains(&StreamSpec::Klv {
        pid: 0x102,
        stream_type: KlvStreamType::SynchronousMetadata,
        carries_pts: true,
    }));
    assert!(prog.streams.contains(&StreamSpec::Audio {
        pid: 0x103,
        codec: AudioCodec::Aac,
        language: Some(*b"eng"),
    }));
}

#[test]
fn async_klv_maps_to_private_data_with_pts() {
    let p = pm(
        0x100,
        0x101,
        vec![
            stream(0x101, 0x1B, DemuxKind::Video(DemuxVideo::H264)),
            stream(0x102, 0x06, DemuxKind::KlvAsync),
        ],
    );
    let cfg = MuxerConfig::from_program_map(&p, &[]).expect("async KLV is representable");
    // carries_pts defaults to true: it is a PES-level property the PMT
    // cannot declare, and PTS-carrying KLV is the STANAG 4609 norm.
    assert!(cfg.programs[0].streams.contains(&StreamSpec::Klv {
        pid: 0x102,
        stream_type: KlvStreamType::PrivateData,
        carries_pts: true,
    }));
}

#[test]
fn unknown_stream_errors_unless_dropped() {
    let p = pm(
        0x100,
        0x101,
        vec![
            stream(0x101, 0x1B, DemuxKind::Video(DemuxVideo::H264)),
            stream(0x104, 0xC0, DemuxKind::Unknown(0xC0)),
        ],
    );

    // Strict by default: the unrepresentable stream is an error naming the
    // offender's PID and stream_type.
    match MuxerConfig::from_program_map(&p, &[]) {
        Err(MuxError::ConfigInvalid { reason }) => {
            assert!(reason.contains("0x0104"), "reason names the pid: {reason}");
            assert!(
                reason.contains("stream_type 0xC0"),
                "reason names the stream_type: {reason}"
            );
        }
        other => panic!("expected ConfigInvalid, got {other:?}"),
    }

    // Opt-out via `drop`: succeeds with the unknown stream absent.
    let cfg = MuxerConfig::from_program_map(&p, &[StreamKindTag::Unknown])
        .expect("dropping Unknown makes the map representable");
    let prog = &cfg.programs[0];
    assert!(
        prog.streams.iter().all(|s| spec_pid(s) != 0x104),
        "dropped stream must be absent"
    );
    assert_eq!(prog.streams.len(), 1, "only the video stream remains");
}

#[test]
fn dvb_subtitle_errors_cea708_and_webvtt_map() {
    // DVB subtitling: per-stream parameters (language, page IDs) are not
    // recoverable from the PMT entry alone → offender.
    let dvb = pm(
        0x100,
        0x101,
        vec![
            stream(0x101, 0x1B, DemuxKind::Video(DemuxVideo::H264)),
            stream(0x105, 0x06, DemuxKind::Subtitle(DemuxSub::DvbSubtitling)),
        ],
    );
    match MuxerConfig::from_program_map(&dvb, &[]) {
        Err(MuxError::ConfigInvalid { reason }) => {
            assert!(reason.contains("0x0105"), "reason names the pid: {reason}");
        }
        other => panic!("expected ConfigInvalid, got {other:?}"),
    }

    // CEA-708 standalone: parameter-free demux variant maps to the
    // parameter-free mux variant.
    let cea = pm(
        0x100,
        0x101,
        vec![
            stream(0x101, 0x1B, DemuxKind::Video(DemuxVideo::H264)),
            stream(0x105, 0x06, DemuxKind::Subtitle(DemuxSub::Cea708Standalone)),
        ],
    );
    let cfg = MuxerConfig::from_program_map(&cea, &[]).expect("CEA-708 is representable");
    assert!(cfg.programs[0].streams.contains(&StreamSpec::Subtitle {
        pid: 0x105,
        codec: SubtitleCodec::Cea708Standalone,
    }));

    // WebVTT-in-TS: likewise parameter-free → mapped.
    let vtt = pm(
        0x100,
        0x101,
        vec![
            stream(0x101, 0x1B, DemuxKind::Video(DemuxVideo::H264)),
            stream(0x105, 0x06, DemuxKind::Subtitle(DemuxSub::WebVttInTs)),
        ],
    );
    let cfg = MuxerConfig::from_program_map(&vtt, &[]).expect("WebVTT is representable");
    assert!(cfg.programs[0].streams.contains(&StreamSpec::Subtitle {
        pid: 0x105,
        codec: SubtitleCodec::WebVttInTs,
    }));
}

#[test]
fn drop_video_from_audio_klv_program_fails_pcr_resolution() {
    let p = pm(
        0x100,
        0x101,
        vec![
            stream(0x101, 0x1B, DemuxKind::Video(DemuxVideo::H264)),
            stream(0x103, 0x0F, DemuxKind::Audio(DemuxAudio::Aac)),
            stream(
                0x102,
                0x15,
                DemuxKind::KlvSync {
                    declared_link: None,
                },
            ),
        ],
    );
    // Dropping the video also drops the demuxed PCR carrier, so the rebuilt
    // program has `pcr_pid: None`. `MuxerConfig::validate` then resolves the
    // default PCR via first-video → first-KLV → first-audio; with no video
    // the fallback lands on the KLV PID, and validate rejects KLV-carried
    // PCR (KLV cadence is too sparse for ETSI TR 101 290 §5.6.1's 100 ms
    // ceiling) — so a video-less audio+KLV program errors rather than
    // falling through to the audio stream.
    match MuxerConfig::from_program_map(&p, &[StreamKindTag::Video]) {
        Err(MuxError::KlvPidUsedAsPcrPid { pid }) => assert_eq!(pid, 0x102),
        other => panic!("expected KlvPidUsedAsPcrPid, got {other:?}"),
    }
}

#[test]
fn drop_video_from_audio_only_program_validates_with_pcr_none() {
    // Without a KLV stream the default-PCR fallback reaches the audio
    // stream, so dropping the video yields a valid audio-only program.
    let p = pm(
        0x100,
        0x101,
        vec![
            stream(0x101, 0x1B, DemuxKind::Video(DemuxVideo::H264)),
            stream(0x103, 0x0F, DemuxKind::Audio(DemuxAudio::Aac)),
        ],
    );
    let cfg = MuxerConfig::from_program_map(&p, &[StreamKindTag::Video])
        .expect("audio-only program resolves PCR to the audio stream");
    let prog = &cfg.programs[0];
    assert!(
        prog.streams.iter().all(|s| spec_pid(s) != 0x101),
        "dropped video must be absent"
    );
    // The demuxed pcr_pid (the dropped video PID) is not on any kept
    // stream → not copied.
    assert_eq!(prog.pcr_pid, None);
}

#[test]
fn pcr_pid_not_on_any_stream_is_not_copied() {
    // pcr_pid 0x1FFF (the null-packet PID — some encoders declare it when
    // no PCR is carried in-program) matches no stream → leave None.
    let p = pm(
        0x100,
        0x1FFF,
        vec![
            stream(0x101, 0x1B, DemuxKind::Video(DemuxVideo::H264)),
            stream(0x103, 0x0F, DemuxKind::Audio(DemuxAudio::Aac)),
        ],
    );
    let cfg = MuxerConfig::from_program_map(&p, &[]).expect("unmatched pcr_pid is not an error");
    assert_eq!(cfg.programs[0].pcr_pid, None);
}

#[test]
fn pcr_on_klv_pid_falls_back_to_default() {
    // Explicit PCR-on-KLV is rejected by MuxerConfig::validate, so the
    // conversion leaves pcr_pid None — the builder default (first video)
    // applies and the program validates.
    let p = pm(
        0x100,
        0x102,
        vec![
            stream(0x101, 0x1B, DemuxKind::Video(DemuxVideo::H264)),
            stream(
                0x102,
                0x15,
                DemuxKind::KlvSync {
                    declared_link: None,
                },
            ),
        ],
    );
    let cfg = MuxerConfig::from_program_map(&p, &[]).expect("PCR-on-KLV falls back, not errors");
    assert_eq!(cfg.programs[0].pcr_pid, None);
}

#[test]
fn all_streams_dropped_is_an_error() {
    let p = pm(
        0x100,
        0x101,
        vec![stream(0x101, 0x1B, DemuxKind::Video(DemuxVideo::H264))],
    );
    // Dropping every stream leaves an empty program; the builder's existing
    // validation rejects it (don't over-pin the exact variant).
    assert!(
        MuxerConfig::from_program_map(&p, &[StreamKindTag::Video]).is_err(),
        "empty program must not validate"
    );
}

#[test]
fn bad_language_descriptors_fall_back_to_no_language() {
    // Tag 0x0A with fewer than 3 data bytes: not a decodable code → None.
    let mut short = stream(0x103, 0x0F, DemuxKind::Audio(DemuxAudio::Aac));
    short.raw_descriptors = vec![RawDescriptor {
        tag: 0x0A,
        data: vec![b'e', b'n'],
    }];
    // Uppercase code: the `iso639_language` recovery helper only accepts
    // plausible lowercase ISO 639-2 codes (`validate_language_code` itself
    // tolerates uppercase), so the conversion falls back to plain
    // `add_audio` → None.
    let mut upper = stream(0x104, 0x0F, DemuxKind::Audio(DemuxAudio::Aac));
    upper.raw_descriptors = vec![RawDescriptor {
        tag: 0x0A,
        data: b"ENG\x00".to_vec(),
    }];
    let p = pm(
        0x100,
        0x101,
        vec![
            stream(0x101, 0x1B, DemuxKind::Video(DemuxVideo::H264)),
            short,
            upper,
        ],
    );
    let cfg = MuxerConfig::from_program_map(&p, &[]).expect("bad descriptors are not an error");
    let prog = &cfg.programs[0];
    assert!(prog.streams.contains(&StreamSpec::Audio {
        pid: 0x103,
        codec: AudioCodec::Aac,
        language: None,
    }));
    assert!(prog.streams.contains(&StreamSpec::Audio {
        pid: 0x104,
        codec: AudioCodec::Aac,
        language: None,
    }));
}

// ─────────────────────────────────────────────────────────────────────────────
// Integration round-trip: mux → demux → from_program_map
// ─────────────────────────────────────────────────────────────────────────────

/// Minimal Annex-B H.264 access unit: SPS + PPS + IDR slice.
fn synthetic_h264_au() -> Vec<u8> {
    fn nal(nal_type: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![0x00, 0x00, 0x00, 0x01];
        v.push((0b11 << 5) | nal_type); // high nal_ref_idc
        v.extend_from_slice(body);
        v
    }
    let mut au = Vec::new();
    au.extend(nal(7, &[0x42, 0xC0, 0x28, 0xD9])); // SPS
    au.extend(nal(8, &[0xCE, 0x38, 0x80])); // PPS
    au.extend(nal(5, &[0x88, 0x84, 0x0A, 0x7C, 0x11])); // IDR slice
    au
}

/// Minimal KLV LS: 16-byte ST 0601 UL + BER short-form length + 4 body bytes.
fn synthetic_klv_ls() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&[
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
        0x00,
    ]);
    v.push(0x04);
    v.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    v
}

fn drain(mux: &mut Muxer) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1316];
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

#[test]
fn mux_demux_program_map_rebuilds_the_config() {
    // Config A: H.264 video + sync KLV, pcr_pid left None (auto-resolve).
    let cfg_a = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x100);
        prog.add_video(0x101, VideoCodec::H264);
        prog.add_klv(0x102, KlvStreamType::SynchronousMetadata, true);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().unwrap()
    };

    let mut mux = Muxer::new(cfg_a.clone()).unwrap();
    for i in 0..3i64 {
        let pts = Pts90khz::new(90_000 + i * 3_000);
        mux.push_video(&synthetic_h264_au(), pts, true)
            .expect("push_video");
        mux.push_klv(&synthetic_klv_ls(), pts, 0x00)
            .expect("push_klv");
    }
    let ts = drain(&mut mux);

    let mut dem = Demuxer::new();
    dem.feed(&ts).unwrap();
    let mut last_pm = None;
    while let Some(evt) = dem.next_event() {
        if let DemuxEvent::ProgramMap(m) = evt {
            last_pm = Some(m);
        }
    }
    let demuxed = last_pm.expect("PMT discovery must emit a ProgramMap");

    let rebuilt = MuxerConfig::from_program_map(&demuxed, &[]).expect("rebuild from demuxed map");
    assert_eq!(rebuilt.programs.len(), 1);
    let (a, b) = (&cfg_a.programs[0], &rebuilt.programs[0]);
    assert_eq!(b.program_number, a.program_number);
    assert_eq!(b.pmt_pid, a.pmt_pid);
    assert_eq!(b.streams, a.streams, "stream specs survive the round-trip");
    // Asymmetry: config A left pcr_pid None, which the muxer resolved to
    // the first video PID (0x101) at mux time. The PMT declares that
    // resolved PID, so the demuxed map carries pcr_pid == 0x101 and the
    // rebuilt config pins it explicitly. Both configs drive the same wire
    // behavior.
    assert_eq!(a.pcr_pid, None);
    assert_eq!(demuxed.pcr_pid, 0x101);
    assert_eq!(b.pcr_pid, Some(0x101));
}
