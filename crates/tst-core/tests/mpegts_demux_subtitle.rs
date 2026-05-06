//! Integration tests for subtitle / caption receiver side
//! (`mpegts::demux`).

use tst_core::mpegts::demux::{
    DemuxEvent, Demuxer, SamplePayload, StreamKind, SubtitleCodec as DemuxSub,
};
use tst_core::mpegts::mux::{Config, Muxer, SubtitleCodec as MuxSub, VideoCodec};

/// Drain every queued packet from the muxer into a single Vec.
fn drain_all(mux: &mut Muxer) -> Vec<u8> {
    let mut all = Vec::new();
    let mut buf = vec![0u8; 188 * 256];
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        all.extend_from_slice(&buf[..n]);
    }
    all
}

/// Build a TS byte stream containing a single subtitle PES on PID 0x200,
/// alongside a placeholder H.264 video PID required by the muxer config.
fn build_ts_with_one_subtitle_pes(codec: MuxSub, payload: &[u8]) -> Vec<u8> {
    let cfg = Config::builder()
        .add_program(1, 0x100)
        .add_video(0x101, VideoCodec::H264)
        .add_subtitle(0x200, codec)
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();
    let h = mux.subtitle_handles()[0];
    mux.push_subtitle_to(h, 90_000, payload).unwrap();
    drain_all(&mut mux)
}

/// Drain every event from the demuxer into a Vec for matching.
fn collect_events(bytes: &[u8]) -> Vec<DemuxEvent> {
    let mut demux = Demuxer::new();
    demux.feed(bytes).unwrap();
    demux.flush();
    let mut events = Vec::new();
    while let Some(e) = demux.next_event() {
        events.push(e);
    }
    events
}

#[test]
fn demux_classifies_webvtt_in_ts_via_vttc_format_identifier() {
    let bytes = build_ts_with_one_subtitle_pes(MuxSub::WebVttInTs, b"WEBVTT\nx-cue\n");
    let events = collect_events(&bytes);
    assert!(
        events.iter().any(|e| matches!(
            e,
            DemuxEvent::Sample {
                stream,
                payload: SamplePayload::Subtitle { codec: DemuxSub::WebVttInTs, .. },
                ..
            } if stream.pid == 0x200
        )),
        "expected WebVttInTs subtitle Sample on PID 0x200, got {events:?}"
    );
}

#[test]
fn demux_classifies_dvb_subtitling_via_descriptor_tag_0x59() {
    let bytes = build_ts_with_one_subtitle_pes(
        MuxSub::DvbSubtitling {
            language: *b"eng",
            subtitling_type: 0x10,
            composition_page_id: 1,
            ancillary_page_id: 1,
        },
        // Synthetic page composition segment header bytes.
        &[0x20, 0x00, 0x10, 0x01, 0x00, 0x14, 0x00, 0x00],
    );
    let events = collect_events(&bytes);
    assert!(
        events.iter().any(|e| matches!(
            e,
            DemuxEvent::Sample {
                payload: SamplePayload::Subtitle {
                    codec: DemuxSub::DvbSubtitling,
                    ..
                },
                ..
            }
        )),
        "expected DvbSubtitling subtitle Sample, got {events:?}"
    );
}

#[test]
fn demux_classifies_dvb_teletext_via_descriptor_tag_0x56() {
    let bytes = build_ts_with_one_subtitle_pes(
        MuxSub::DvbTeletext {
            language: *b"eng",
            teletext_type: 0x02,
            magazine_number: 1,
            page_number: 0x88,
        },
        // Synthetic teletext data unit prefix bytes.
        &[0x10, 0x02, 0x10, 0x00, 0x00],
    );
    let events = collect_events(&bytes);
    assert!(
        events.iter().any(|e| matches!(
            e,
            DemuxEvent::Sample {
                payload: SamplePayload::Subtitle {
                    codec: DemuxSub::DvbTeletext,
                    ..
                },
                ..
            }
        )),
        "expected DvbTeletext subtitle Sample, got {events:?}"
    );
}

#[test]
fn demux_classifies_cea708_standalone_via_ga94_format_identifier() {
    let bytes = build_ts_with_one_subtitle_pes(
        MuxSub::Cea708Standalone,
        // Synthetic cc_data bytes.
        &[0xFE, 0x80, 0x80, 0x80],
    );
    let events = collect_events(&bytes);
    assert!(
        events.iter().any(|e| matches!(
            e,
            DemuxEvent::Sample {
                payload: SamplePayload::Subtitle {
                    codec: DemuxSub::Cea708Standalone,
                    ..
                },
                ..
            }
        )),
        "expected Cea708Standalone subtitle Sample, got {events:?}"
    );
}

#[test]
fn demux_program_map_event_lists_subtitle_pid_with_program_number() {
    let bytes = build_ts_with_one_subtitle_pes(MuxSub::WebVttInTs, b"WEBVTT\n");
    let events = collect_events(&bytes);
    let pm = events
        .iter()
        .find_map(|e| match e {
            DemuxEvent::ProgramMap(pm) => Some(pm),
            _ => None,
        })
        .expect("ProgramMap event");
    let sub = pm
        .streams
        .iter()
        .find(|s| s.pid == 0x200)
        .expect("subtitle stream on PID 0x200 in ProgramMap");
    assert!(matches!(
        sub.kind,
        StreamKind::Subtitle(DemuxSub::WebVttInTs)
    ));
    assert_eq!(sub.program_number, 1);
}
