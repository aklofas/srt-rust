//! Integration tests for receiver-side audio carriage in `mpegts::demux`.

use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::demux::{
    Demuxer,
    event::{AudioCodec, DemuxEvent, SamplePayload},
};
use tst_core::mpegts::mux::{
    AudioCodec as MuxAudioCodec, Muxer, MuxerConfig, MuxerProgramConfigBuilder, VideoCodec,
};

/// Helper: mux audio + video, drain bytes, return them.
fn mux_audio_video(codec: MuxAudioCodec, audio_pid: u16) -> Vec<u8> {
    let cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x100, VideoCodec::H264);
        prog.add_audio(audio_pid, codec);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().unwrap()
    };
    let mut muxer = Muxer::new(cfg).unwrap();
    let nal = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1F];
    muxer.push_video(&nal, Pts90khz::new(90_000), true).unwrap();
    muxer
        .push_audio(b"synthetic_audio_frame_payload", Pts90khz::new(90_000))
        .unwrap();
    let mut buf = vec![0u8; 188 * 256];
    let n = muxer.pull(&mut buf);
    buf.truncate(n);
    buf
}

#[test]
fn demux_classifies_each_audio_codec_correctly() {
    let cases = [
        (MuxAudioCodec::Mp2, AudioCodec::Mp2),
        (MuxAudioCodec::Aac, AudioCodec::Aac),
        (MuxAudioCodec::AacLatm, AudioCodec::AacLatm),
        (MuxAudioCodec::Ac3, AudioCodec::Ac3),
    ];
    for (mux_codec, expected_demux_codec) in cases {
        let bytes = mux_audio_video(mux_codec, 0x300);
        let mut demuxer = Demuxer::new();
        demuxer.feed(&bytes).unwrap();
        demuxer.flush();

        let mut events = Vec::new();
        while let Some(e) = demuxer.next_event() {
            events.push(e);
        }

        let sample = events
            .iter()
            .find(|e| {
                matches!(
                    e,
                    DemuxEvent::Sample {
                        payload: SamplePayload::Audio { .. },
                        ..
                    }
                )
            })
            .unwrap_or_else(|| panic!("no audio sample for {mux_codec:?}"));
        if let DemuxEvent::Sample {
            payload: SamplePayload::Audio { codec, frames },
            ..
        } = sample
        {
            assert_eq!(*codec, expected_demux_codec);
            assert!(!frames.is_empty(), "audio frames non-empty");
        }
    }
}

#[test]
fn demux_audio_pts_surfaces() {
    let bytes = mux_audio_video(MuxAudioCodec::Aac, 0x300);
    let mut demuxer = Demuxer::new();
    demuxer.feed(&bytes).unwrap();
    demuxer.flush();

    let mut events = Vec::new();
    while let Some(e) = demuxer.next_event() {
        events.push(e);
    }

    let sample = events
        .iter()
        .find_map(|e| match e {
            DemuxEvent::Sample {
                pts,
                dts,
                payload: SamplePayload::Audio { .. },
                ..
            } => Some((pts, dts)),
            _ => None,
        })
        .unwrap();
    assert_eq!(*sample.0, 90_000);
    assert_eq!(*sample.1, None, "audio always has dts: None");
}
