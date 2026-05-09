//! End-to-end roundtrip: mux audio frames + video AUs + KLV records,
//! demux the resulting bytes, assert per-frame byte equality.

use tst_core::mpegts::demux::{
    Demuxer,
    event::{DemuxEvent, SamplePayload},
};
use tst_core::mpegts::mux::{AudioCodec as MuxAudioCodec, Muxer, MuxerConfig, VideoCodec};

fn roundtrip_one_codec(codec: MuxAudioCodec) {
    let cfg = MuxerConfig::builder()
        .add_program(1, 0x1000)
        .add_video(0x100, VideoCodec::H264)
        .add_audio(0x300, codec)
        .end_program()
        .build()
        .unwrap();
    let mut muxer = Muxer::new(cfg).unwrap();

    // Synthesize 5 audio frames with distinct content + 5 video AUs.
    let audio_frames: Vec<Vec<u8>> = (0..5)
        .map(|i| (0..200).map(|b| (b ^ i) as u8).collect())
        .collect();
    let video_nal = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1F];

    for (i, frame) in audio_frames.iter().enumerate() {
        let pts = 90_000 + (i as i64) * 1024; // 1024 samples per AAC frame
        muxer.push_video(&video_nal, pts, true).unwrap();
        muxer.push_audio(frame, pts).unwrap();
    }

    // Drain.
    let mut buf = vec![0u8; 188 * 4096];
    let n = muxer.pull(&mut buf);
    buf.truncate(n);

    // Demux.
    let mut demuxer = Demuxer::new();
    demuxer.feed(&buf).unwrap();
    demuxer.flush();

    let mut audio_recovered: Vec<Vec<u8>> = Vec::new();
    while let Some(event) = demuxer.next_event() {
        if let DemuxEvent::Sample {
            payload: SamplePayload::Audio { frames, .. },
            ..
        } = event
        {
            audio_recovered.push(frames);
        }
    }

    assert_eq!(
        audio_recovered.len(),
        audio_frames.len(),
        "frame count match"
    );
    for (i, (out, inp)) in audio_recovered.iter().zip(audio_frames.iter()).enumerate() {
        assert_eq!(out, inp, "frame {i} byte-equal roundtrip");
    }
}

#[test]
fn mp2_roundtrip() {
    roundtrip_one_codec(MuxAudioCodec::Mp2);
}

#[test]
fn aac_adts_roundtrip() {
    roundtrip_one_codec(MuxAudioCodec::Aac);
}

#[test]
fn aac_latm_roundtrip() {
    roundtrip_one_codec(MuxAudioCodec::AacLatm);
}

#[test]
fn ac3_roundtrip() {
    roundtrip_one_codec(MuxAudioCodec::Ac3);
}
