//! Round-trip integrity: mux→demux subtitle PES bytes must come
//! back identical.

use tst_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload};
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
    // The demuxer strips that envelope before surfacing, so the round-tripped
    // payload is byte-identical to what the caller pushed.
    let segments = vec![0x0F, 0x10, 0x00, 0x01, 0x00, 0x02, 0x00, 0x10];

    let output = round_trip(
        MuxSub::DvbSubtitling {
            language: *b"eng",
            subtitling_type: 0x10,
            composition_page_id: 1,
            ancillary_page_id: 1,
        },
        &segments,
    );
    assert_eq!(segments, output);
}

#[test]
fn roundtrip_dvb_teletext_payload_prefixes_pes_with_tail_stuffing() {
    // EN 300 472 §4.2 mandates a 45-byte stuffed PES header and a PES that
    // is exactly N×184 bytes long — the muxer pads the tail with 0xFF
    // stuffing to reach that boundary. The demuxer surfaces everything
    // after the 45-byte header (PES payload + 0xFF stuffing), so the
    // round-tripped bytes start with the input and the rest is 0xFF.
    let input = vec![0x10, 0x02, 0x10, 0xFC, 0x40, 0x40, 0x80];
    let output = round_trip(
        MuxSub::DvbTeletext {
            language: *b"eng",
            teletext_type: 0x02,
            magazine_number: 0,
            page_number: 0x88,
        },
        &input,
    );
    assert_eq!(&output[..input.len()], &input[..]);
    assert!(
        output[input.len()..].iter().all(|&b| b == 0xFF),
        "EN 300 472 §4.2 PES tail past caller payload must be 0xFF stuffing",
    );
    // PES is exactly N×184 bytes; demuxer surfaces (N×184 − 45) bytes of body.
    let body_len = output.len();
    let total = body_len + 45;
    assert_eq!(
        total % 184,
        0,
        "demuxed body length implies non-conformant PES total length"
    );
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
