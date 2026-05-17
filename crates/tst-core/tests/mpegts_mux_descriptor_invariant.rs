//! Plan #72 (Wave 2.3): the `MuxerProgramConfig.stream_descriptors`
//! length-mismatch invariant is enforced via the rich
//! `MuxError::ConfigInvalid { reason: String }` variant. The reason
//! string names the offending program_number plus both lengths so
//! callers can locate the bug in a multi-program config without
//! re-reading the code.

use tst_core::error::MuxError;
use tst_core::mpegts::mux::{
    KlvStreamType, Muxer, MuxerConfig, MuxerProgramConfig, StreamSpec, VideoCodec,
};

/// Hand-built `MuxerProgramConfig` with mismatched `stream_descriptors`
/// length is rejected by `Muxer::new` with the rich
/// `MuxError::ConfigInvalid` variant. The reason names the program
/// number and both lengths.
#[test]
fn muxer_new_rejects_descriptor_length_mismatch_with_rich_diagnostic() {
    let cfg = MuxerConfig {
        programs: vec![MuxerProgramConfig {
            program_number: 7,
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
            program_descriptors: vec![],
            // INTENTIONAL MISMATCH: 2 streams but only 1 descriptor list.
            stream_descriptors: vec![vec![]],
        }],
        pcr_interval_ms: 40,
        psi_interval_ms: 100,
        buffer_packets: 10_000,
    };
    let result = Muxer::new(cfg);
    let err = match result {
        Ok(_) => panic!("descriptor-length mismatch must be rejected"),
        Err(e) => e,
    };
    match err {
        MuxError::ConfigInvalid { reason } => {
            assert!(
                reason.contains("program 7"),
                "reason missing program_number: {reason}"
            );
            assert!(
                reason.contains("2 streams"),
                "reason missing actual streams.len: {reason}"
            );
            assert!(
                reason.contains("1 stream_descriptors"),
                "reason missing actual stream_descriptors.len: {reason}",
            );
        }
        other => panic!("expected MuxError::ConfigInvalid, got {other:?}"),
    }
}

/// Inverse: matched lengths construct the Muxer successfully.
#[test]
fn muxer_new_accepts_descriptor_length_match() {
    let cfg = MuxerConfig {
        programs: vec![MuxerProgramConfig {
            program_number: 1,
            pmt_pid: 0x1000,
            streams: vec![StreamSpec::Video {
                pid: 0x1011,
                codec: VideoCodec::H264,
            }],
            pcr_pid: None,
            program_descriptors: vec![],
            // 1 stream, 1 descriptor list (empty is fine).
            stream_descriptors: vec![vec![]],
        }],
        pcr_interval_ms: 40,
        psi_interval_ms: 100,
        buffer_packets: 10_000,
    };
    let result = Muxer::new(cfg);
    let err = result.err();
    assert!(
        err.is_none(),
        "matched lengths must construct: {:?}",
        err.unwrap()
    );
}
