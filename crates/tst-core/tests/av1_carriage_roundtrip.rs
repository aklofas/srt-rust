//! AV1 mux -> demux carriage round-trip.
//!
//! Asserts that pushing a synthetic AV1 access unit (Temporal Delimiter +
//! Sequence Header + Frame Header + Tile Group OBUs) through the muxer +
//! demuxer round-trip preserves the codec classification (PMT stream_type
//! 0x06 with `format_identifier "AV01"` -> `VideoCodec::Av1`) and the
//! per-OBU header bytes.

use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::demux::Demuxer;
use tst_core::mpegts::demux::event::{
    DemuxEvent, Obu, SamplePayload, StreamId, StreamKind, VideoCodec, VideoPayload,
};
use tst_core::mpegts::mux::{
    Muxer, MuxerConfig, MuxerProgramConfigBuilder, VideoCodec as MuxVideoCodec,
};

/// Build a minimal AV1 access unit: Temporal Delimiter + Sequence Header +
/// Frame Header + Tile Group. Each OBU has `obu_has_size_field = 1`. Bodies
/// are placeholders — what matters for this test is that the demuxer recovers
/// each OBU with the correct `obu_type`.
fn synthetic_av1_au() -> Vec<u8> {
    fn obu(obu_type: u8, body: &[u8]) -> Vec<u8> {
        // AV1 spec §5.3.2 OBU header byte:
        //   obu_forbidden_bit  f(1) = 0
        //   obu_type           f(4)
        //   obu_extension_flag f(1) = 0
        //   obu_has_size_field f(1) = 1   <-- required by AV1-in-MPEG-2-TS §3.1
        //   obu_reserved_1bit  f(1) = 0
        // = (obu_type << 3) | 0b010
        let header = (obu_type << 3) | 0x02;
        let mut v = vec![header];
        // Single-byte LEB128 size (body lengths < 128 here).
        v.push(body.len() as u8);
        v.extend_from_slice(body);
        v
    }
    let mut au = Vec::new();
    au.extend(obu(2, &[])); // Temporal Delimiter (always empty body)
    au.extend(obu(1, &[0x00, 0x00])); // Sequence Header (placeholder body)
    au.extend(obu(3, &[0x00])); // Frame Header (placeholder body)
    au.extend(obu(4, &[0x00, 0x01, 0x02])); // Tile Group (placeholder body)
    au
}

fn drain_mux(mux: &mut Muxer) -> Vec<u8> {
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
fn av1_mux_demux_roundtrip_emits_obus() {
    let cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x100);
        prog.add_video(0x101, MuxVideoCodec::Av1);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().unwrap()
    };
    let mut mux = Muxer::new(cfg).unwrap();
    let video_handle = mux.video_handles()[0];
    let au = synthetic_av1_au();

    // Push one AU at PTS=90000 (1 second), key frame.
    mux.push_video_to(video_handle, &au, Pts90khz::new(90_000), true)
        .expect("push");
    let ts_bytes = drain_mux(&mut mux);

    let mut demux = Demuxer::new();
    demux.feed(&ts_bytes).unwrap();
    // Unbounded video PES (PES_packet_length=0) buffers in-flight; flush
    // drains it. Live receive loops do this on TransportError::Closed.
    demux.flush();

    let mut sample_evt: Option<(StreamId, SamplePayload)> = None;
    while let Some(e) = demux.next_event() {
        if let DemuxEvent::Sample {
            stream, payload, ..
        } = e
        {
            sample_evt = Some((stream, payload));
            break;
        }
    }
    let (stream, payload) = sample_evt.expect("expected Sample event");
    assert_eq!(stream.kind, StreamKind::Video(VideoCodec::Av1));
    match payload {
        SamplePayload::Video {
            codec: VideoCodec::Av1,
            payload: VideoPayload::Obus(obus),
            ..
        } => {
            assert_eq!(
                obus.len(),
                4,
                "expected 4 OBUs (TD/SeqHeader/FrameHeader/TileGroup)"
            );
            let types: Vec<u8> = obus.iter().map(|o: &Obu| o.obu_type).collect();
            assert_eq!(types, vec![2, 1, 3, 4]);
        }
        other => panic!("unexpected SamplePayload: {other:?}"),
    }
}
