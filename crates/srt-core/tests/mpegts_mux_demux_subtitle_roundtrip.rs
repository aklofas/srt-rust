//! Round-trip integrity: mux→demux subtitle PES bytes must come
//! back identical.

use srt_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload};
use srt_core::mpegts::mux::{Config, Muxer, SubtitleCodec as MuxSub, VideoCodec};

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

/// Push `payload` through a subtitle PID 0x200, demux, and return the
/// payload bytes from the first `SamplePayload::Subtitle` event. Returns
/// an empty Vec if no Sample event surfaces (e.g. empty PES dropped).
fn round_trip(codec: MuxSub, payload: &[u8]) -> Vec<u8> {
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
    let bytes = drain_all(&mut mux);

    let mut demux = Demuxer::new();
    demux.feed(&bytes).unwrap();
    demux.flush();
    let mut out: Option<Vec<u8>> = None;
    while let Some(e) = demux.next_event() {
        if let DemuxEvent::Sample {
            payload: SamplePayload::Subtitle { payload, .. },
            ..
        } = e
        {
            out = Some(payload);
            break;
        }
    }
    out.unwrap_or_default()
}

#[test]
fn roundtrip_webvtt_payload_byte_identical() {
    let input = b"WEBVTT\n\n00:00:01.000 --> 00:00:05.000\nhello world\n".to_vec();
    let output = round_trip(MuxSub::WebVttInTs, &input);
    assert_eq!(input, output);
}

#[test]
fn roundtrip_dvb_subtitling_payload_byte_identical() {
    // Caller supplies raw subtitling_segment bytes (per EN 300 743 §7.2.2,
    // each starting with sync_byte 0x0F); the muxer auto-prepends the
    // §6.2 PES_data_field envelope (data_identifier=0x20 +
    // subtitle_stream_id=0x00 + segments + end_of_PES_data_field_marker=0xFF).
    // Future demuxer work may strip the envelope before surfacing it; until
    // then the demuxer surfaces the full PES_data_field, so the round-tripped
    // payload matches the wrapped form (asserted on below).
    let segments = vec![0x0F, 0x10, 0x00, 0x01, 0x00, 0x02, 0x00, 0x10];
    let mut wrapped = Vec::new();
    wrapped.push(0x20);
    wrapped.push(0x00);
    wrapped.extend_from_slice(&segments);
    wrapped.push(0xFF);

    let output = round_trip(
        MuxSub::DvbSubtitling {
            language: *b"eng",
            subtitling_type: 0x10,
            composition_page_id: 1,
            ancillary_page_id: 1,
        },
        &segments,
    );
    assert_eq!(wrapped, output);
}

#[test]
fn roundtrip_dvb_teletext_payload_byte_identical() {
    let input = vec![0x10, 0x02, 0x10, 0xFC, 0x40, 0x40, 0x80];
    let output = round_trip(
        MuxSub::DvbTeletext {
            language: *b"eng",
            teletext_type: 0x02,
            magazine_number: 1,
            page_number: 0x88,
        },
        &input,
    );
    assert_eq!(input, output);
}

#[test]
fn roundtrip_cea708_standalone_payload_byte_identical() {
    let input = vec![0xFE, 0x80, 0x80, 0x80, 0xFC, 0x80, 0x80, 0x80];
    let output = round_trip(MuxSub::Cea708Standalone, &input);
    assert_eq!(input, output);
}

#[test]
fn roundtrip_empty_payload_accepted() {
    let output = round_trip(MuxSub::WebVttInTs, &[]);
    assert!(output.is_empty());
}
