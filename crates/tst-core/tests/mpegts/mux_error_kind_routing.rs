//! Verify every `MuxError` variant routes to the expected
//! `MuxSenderErrorKind` via the `MuxError::kind()` method.
//!
//! One (variant, expected kind) row per MuxError variant. Maintained as a flat table; if
//! a new variant is added upstream the CI ratchet
//! `scripts/check/rust/mux-error-kind-coverage.sh` catches the missing
//! match arm before this test runs. This test is the per-variant
//! correctness check (the ratchet guarantees coverage; this test
//! guarantees correctness of the mapping).
//!
//! The canonical routing table is the `MuxError::kind()` match in
//! `crates/tst-core/src/error.rs`.

use tst_core::error::{MuxError, MuxSenderErrorKind};
use tst_core::mpegts::mux::{StreamKind, TeletextField};

/// Helper: assert a `MuxError` variant returns the expected kind.
fn assert_kind(e: MuxError, expected: MuxSenderErrorKind) {
    let got = e.kind();
    assert_eq!(
        got, expected,
        "MuxError variant did not route to expected kind: got {got:?}, expected {expected:?}, variant: {e:?}"
    );
}

// === InputMalformed (4 variants) ===

#[test]
fn invalid_nal_routes_to_input_malformed() {
    assert_kind(MuxError::InvalidNal, MuxSenderErrorKind::InputMalformed);
}

#[test]
fn klv_too_large_routes_to_input_malformed() {
    assert_kind(
        MuxError::KlvTooLarge { size: 100, max: 50 },
        MuxSenderErrorKind::InputMalformed,
    );
}

#[test]
fn audio_too_large_routes_to_input_malformed() {
    assert_kind(
        MuxError::AudioTooLarge { size: 100, max: 50 },
        MuxSenderErrorKind::InputMalformed,
    );
}

#[test]
fn subtitle_too_large_routes_to_input_malformed() {
    assert_kind(
        MuxError::SubtitleTooLarge { size: 100, max: 50 },
        MuxSenderErrorKind::InputMalformed,
    );
}

// === Backpressure (1 variant) ===

#[test]
fn buffer_full_routes_to_backpressure() {
    assert_kind(
        MuxError::BufferFull {
            capacity_packets: 100,
        },
        MuxSenderErrorKind::Backpressure,
    );
}

// === ConfigInvalid (19 variants) ===

#[test]
fn invalid_config_routes_to_config_invalid() {
    assert_kind(
        MuxError::InvalidConfig("test"),
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn config_invalid_with_reason_routes_to_config_invalid() {
    assert_kind(
        MuxError::ConfigInvalid {
            reason: "test reason".into(),
        },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn invalid_language_code_routes_to_config_invalid() {
    assert_kind(
        MuxError::InvalidLanguageCode {
            code: [b'X', b'X', b'X'],
        },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn invalid_teletext_field_routes_to_config_invalid() {
    assert_kind(
        MuxError::InvalidTeletextField {
            field: TeletextField::MagazineNumber,
            value: 99,
            max: 7,
        },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn too_many_video_streams_routes_to_config_invalid() {
    assert_kind(
        MuxError::TooManyVideoStreams { count: 17, cap: 16 },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn too_many_klv_streams_routes_to_config_invalid() {
    assert_kind(
        MuxError::TooManyKlvStreams { count: 17, cap: 16 },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn too_many_audio_streams_routes_to_config_invalid() {
    assert_kind(
        MuxError::TooManyAudioStreams { count: 17, cap: 16 },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn too_many_subtitle_streams_routes_to_config_invalid() {
    assert_kind(
        MuxError::TooManySubtitleStreams { count: 17, cap: 16 },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn too_many_programs_routes_to_config_invalid() {
    assert_kind(
        MuxError::TooManyPrograms { count: 17, cap: 16 },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn empty_program_routes_to_config_invalid() {
    assert_kind(
        MuxError::EmptyProgram { program_number: 1 },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn duplicate_program_number_routes_to_config_invalid() {
    assert_kind(
        MuxError::DuplicateProgramNumber { program_number: 1 },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn duplicate_pmt_pid_routes_to_config_invalid() {
    assert_kind(
        MuxError::DuplicatePmtPid {
            pid: 0x100,
            programs: [1, 2],
        },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn duplicate_pid_across_programs_routes_to_config_invalid() {
    assert_kind(
        MuxError::DuplicatePidAcrossPrograms {
            pid: 0x100,
            programs: [1, 2],
        },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn pmt_pid_conflicts_with_stream_routes_to_config_invalid() {
    assert_kind(
        MuxError::PmtPidConflictsWithStream {
            pmt_pid: 0x100,
            program_number: 1,
        },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn subtitle_pid_used_as_pcr_pid_routes_to_config_invalid() {
    assert_kind(
        MuxError::SubtitlePidUsedAsPcrPid { pid: 0x100 },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn klv_pid_used_as_pcr_pid_routes_to_config_invalid() {
    assert_kind(
        MuxError::KlvPidUsedAsPcrPid { pid: 0x100 },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn no_pcr_eligible_stream_routes_to_config_invalid() {
    assert_kind(
        MuxError::NoPcrEligibleStream { program_number: 1 },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn malformed_descriptor_routes_to_config_invalid() {
    assert_kind(
        MuxError::MalformedDescriptor {
            stream_index: 0,
            descriptor_index: 0,
            reason: "test",
        },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

#[test]
fn pmt_too_large_routes_to_config_invalid() {
    assert_kind(
        MuxError::PmtTooLarge {
            used_bytes: 200,
            max_bytes: 183,
        },
        MuxSenderErrorKind::ConfigInvalid,
    );
}

// === InvalidUsage (8 variants) ===

#[test]
fn invalid_stream_handle_routes_to_invalid_usage() {
    assert_kind(
        MuxError::InvalidStreamHandle {
            kind: StreamKind::Video,
            index: 0,
        },
        MuxSenderErrorKind::InvalidUsage,
    );
}

#[test]
fn ambiguous_target_routes_to_invalid_usage() {
    assert_kind(
        MuxError::AmbiguousTarget {
            kind: StreamKind::Video,
            count: 2,
        },
        MuxSenderErrorKind::InvalidUsage,
    );
}

#[test]
fn no_klv_streams_configured_routes_to_invalid_usage() {
    assert_kind(
        MuxError::NoKlvStreamsConfigured,
        MuxSenderErrorKind::InvalidUsage,
    );
}

#[test]
fn no_audio_streams_configured_routes_to_invalid_usage() {
    assert_kind(
        MuxError::NoAudioStreamsConfigured,
        MuxSenderErrorKind::InvalidUsage,
    );
}

#[test]
fn no_subtitle_streams_configured_routes_to_invalid_usage() {
    assert_kind(
        MuxError::NoSubtitleStreamsConfigured,
        MuxSenderErrorKind::InvalidUsage,
    );
}

#[test]
fn program_not_found_routes_to_invalid_usage() {
    assert_kind(
        MuxError::ProgramNotFound { program_number: 1 },
        MuxSenderErrorKind::InvalidUsage,
    );
}

#[test]
fn descriptor_index_out_of_range_routes_to_invalid_usage() {
    assert_kind(
        MuxError::DescriptorIndexOutOfRange {
            kind: StreamKind::Video,
            index: 5,
            program_number: 1,
        },
        MuxSenderErrorKind::InvalidUsage,
    );
}

#[test]
fn abs_index_out_of_range_routes_to_invalid_usage() {
    assert_kind(
        MuxError::AbsIndexOutOfRange {
            abs_idx: 99,
            len: 3,
            program_number: 1,
        },
        MuxSenderErrorKind::InvalidUsage,
    );
}

// === Kind enum properties ===

#[test]
fn kind_is_copy_and_eq() {
    let k1 = MuxSenderErrorKind::ConfigInvalid;
    let k2 = k1; // Copy
    assert_eq!(k1, k2);
}

#[test]
fn kind_debug_formats_as_variant_name() {
    let k = MuxSenderErrorKind::InvalidUsage;
    assert_eq!(format!("{k:?}"), "InvalidUsage");
}
