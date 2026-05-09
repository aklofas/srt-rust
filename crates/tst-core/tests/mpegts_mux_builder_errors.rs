//! Regression tests for sub-phase 1.1.4: descriptor-index builder methods
//! must never panic; out-of-range indices surface as
//! [`MuxError::DescriptorIndexOutOfRange`] from `MuxerConfigBuilder::build()` (and
//! transitively from `MuxSender::new`).

use tst_core::MuxError;
use tst_core::mpegts::mux::{
    AudioCodec, MuxerConfig, KlvStreamType, StreamKind, SubtitleCodec, VideoCodec,
};

#[test]
fn stream_descriptors_for_video_out_of_range_does_not_panic() {
    // Only one video stream (index 0); index 5 is out of range.
    let result = MuxerConfig::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264)
        .stream_descriptors_for_video(5, vec![vec![0x05, 0x04, 0x4B, 0x4C, 0x56, 0x41]])
        .end_program()
        .build();

    match result {
        Err(MuxError::DescriptorIndexOutOfRange {
            kind,
            index,
            program_number,
        }) => {
            assert_eq!(kind, StreamKind::Video);
            assert_eq!(index, 5);
            assert_eq!(program_number, 1);
        }
        other => panic!("expected DescriptorIndexOutOfRange, got {:?}", other),
    }
}

#[test]
fn stream_descriptors_for_klv_out_of_range_does_not_panic() {
    // Only one KLV stream (index 0); index 5 is out of range.
    let result = MuxerConfig::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264)
        .add_klv(0x1031, KlvStreamType::PrivateData, false)
        .stream_descriptors_for_klv(5, vec![vec![0x05, 0x04, 0x4B, 0x4C, 0x56, 0x41]])
        .end_program()
        .build();

    match result {
        Err(MuxError::DescriptorIndexOutOfRange {
            kind,
            index,
            program_number,
        }) => {
            assert_eq!(kind, StreamKind::Klv);
            assert_eq!(index, 5);
            assert_eq!(program_number, 1);
        }
        other => panic!("expected DescriptorIndexOutOfRange, got {:?}", other),
    }
}

#[test]
fn stream_descriptors_for_audio_out_of_range_does_not_panic() {
    // Only one audio stream (index 0); index 5 is out of range.
    let result = MuxerConfig::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264)
        .add_audio(0x1021, AudioCodec::Aac)
        .stream_descriptors_for_audio(5, vec![vec![]])
        .end_program()
        .build();

    match result {
        Err(MuxError::DescriptorIndexOutOfRange {
            kind,
            index,
            program_number,
        }) => {
            assert_eq!(kind, StreamKind::Audio);
            assert_eq!(index, 5);
            assert_eq!(program_number, 1);
        }
        other => panic!("expected DescriptorIndexOutOfRange, got {:?}", other),
    }
}

#[test]
fn stream_descriptors_for_subtitle_out_of_range_does_not_panic() {
    // Only one subtitle stream (index 0); index 5 is out of range.
    let result = MuxerConfig::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264)
        .add_subtitle(
            0x1041,
            SubtitleCodec::DvbSubtitling {
                language: *b"eng",
                subtitling_type: 0x10,
                composition_page_id: 1,
                ancillary_page_id: 1,
            },
        )
        .stream_descriptors_for_subtitle(5, vec![vec![]])
        .end_program()
        .build();

    match result {
        Err(MuxError::DescriptorIndexOutOfRange {
            kind,
            index,
            program_number,
        }) => {
            assert_eq!(kind, StreamKind::Subtitle);
            assert_eq!(index, 5);
            assert_eq!(program_number, 1);
        }
        other => panic!("expected DescriptorIndexOutOfRange, got {:?}", other),
    }
}

#[test]
fn stream_descriptors_for_stream_out_of_range_does_not_panic() {
    // One stream total (abs index 0); abs_idx 99 is out of range.
    let result = MuxerConfig::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264)
        .stream_descriptors_for_stream(99, vec![vec![]])
        .end_program()
        .build();

    assert!(
        matches!(
            result,
            Err(MuxError::AbsIndexOutOfRange {
                abs_idx: 99,
                len: 1,
                program_number: 1
            })
        ),
        "expected AbsIndexOutOfRange, got {:?}",
        result
    );
}

#[test]
fn first_descriptor_index_error_wins() {
    // Two consecutive out-of-range calls — only the FIRST should surface
    // from build(). The deferred_error field is first-error-wins: the
    // is_none() guard on each setter means a second bad index can never
    // overwrite the first.
    let cfg = MuxerConfig::builder()
        .add_program(1, 0x1000)
        .add_video(0x1011, VideoCodec::H264)
        .stream_descriptors_for_video(7, vec![vec![]])  // first error: index 7
        .stream_descriptors_for_video(99, vec![vec![]]) // must NOT overwrite
        .end_program()
        .build();

    match cfg {
        Err(MuxError::DescriptorIndexOutOfRange {
            kind,
            index,
            program_number,
        }) => {
            assert_eq!(kind, StreamKind::Video);
            assert_eq!(
                index, 7,
                "first-error-wins: expected the first bad index (7), got {}",
                index
            );
            assert_eq!(program_number, 1);
        }
        other => panic!("expected DescriptorIndexOutOfRange, got {:?}", other),
    }
}
