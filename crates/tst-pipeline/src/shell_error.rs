//! Unified shell-layer error vocabulary for binding authors.
//!
//! Every pipeline shell (`MuxSender`, `Sender`, `RawSender`, `DemuxReceiver`,
//! `Receiver`, `RawReceiver`) exposes a `struct { kind, source }` error
//! type. Bindings categorize failures by matching on `err.kind()` (6
//! variants, 1:1 with TST_E codes); power users `match err.source` for the
//! full inner-error variant set.
//!
//! See `docs/refactor-1/_wave-4-plan-design.md` Plan A for the design
//! decisions behind this shape.

use tst_core::error::{DemuxError, MuxError};
use tst_core::transport::TransportError;

use crate::sender::TsFramingError;

/// Categorical reason for a shell-layer failure.
///
/// Bindings (`tst-c`, `srt-jni`, `srt-uniffi`) map each kind directly to
/// a language-native error code or exception. Per-shell applicability:
///
/// | Kind | MuxSender | Sender | RawSender | DemuxReceiver | Receiver | RawReceiver |
/// |------|:---------:|:------:|:---------:|:-------------:|:--------:|:-----------:|
/// | `ConfigInvalid` | ✓ | — | — | — | — | — |
/// | `InputMalformed` | ✓ | ✓ | — | ✓ | — | — |
/// | `Backpressure` | ✓ | ✓ | ✓ | — | — | — |
/// | `TransportBroken` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
/// | `Closed` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
/// | `EndOfStream` | — | — | — | ✓ | ✓ | ✓ |
///
/// Kinds marked "—" cannot be produced by that shell. Bindings can
/// document the unreachable arms with `unreachable!()` or
/// `debug_assert!(false)`; the runtime never reaches them.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShellErrorKind {
    /// Configuration is invalid — either the construction-time
    /// `*Config` failed `validate()`, or a runtime push referenced an
    /// invalid handle / ambiguous target.
    ///
    /// Maps to `TST_E_INVALID_CONFIG` (-1) in `tst-c`.
    ConfigInvalid,

    /// Input bytes don't conform to the expected shape (Annex-B NAL,
    /// well-formed PSI / PES, valid KLV LS bytes, TS sync alignment,
    /// payload size within PES caps).
    ///
    /// Maps to `TST_E_INVALID_TS` (-3) in `tst-c`.
    InputMalformed,

    /// Transport is alive but momentarily refused (libsrt `SRT_EASYNCSND`,
    /// muxer internal buffer full and waiting for drain). Caller can retry
    /// the same input after backing off.
    ///
    /// Maps to `TST_E_BACKPRESSURE` (-5) in `tst-c` on transport-side; the
    /// muxer-internal `MuxError::BufferFull` also folds here, mapping to
    /// `TST_E_BUFFER_FULL` (-4) via the kind-to-code table.
    Backpressure,

    /// Transport-layer failure — socket broken, libsrt error, factory
    /// closure errored during managed reconnect.
    ///
    /// Maps to `TST_E_TRANSPORT` (-8) in `tst-c`.
    TransportBroken,

    /// Caller invoked `close()` / `cancel()` on this shell (or on its
    /// underlying transport). Subsequent calls on this handle return
    /// the same kind.
    ///
    /// Maps to `TST_E_CLOSED` (-7) in `tst-c`.
    Closed,

    /// Peer closed the connection cleanly — receiver shells only.
    /// The shell's recv loop reached end-of-stream. The handle is dead;
    /// subsequent calls return `Closed`.
    ///
    /// Maps to `TST_E_END_OF_STREAM` (-12) in `tst-c`.
    EndOfStream,
}

/// Implemented by all 6 pipeline shell error types. Bindings use this
/// trait to categorize failures uniformly across shells.
///
/// # Example
///
/// ```ignore
/// // MuxSenderError::kind() is wired in Task 3 (shell error type refactors).
/// use tst_pipeline::{ShellError, ShellErrorKind, MuxSenderError};
/// use tst_core::error::MuxError;
///
/// let err = MuxSenderError::from(MuxError::InvalidConfig("test"));
/// assert_eq!(err.kind(), ShellErrorKind::ConfigInvalid);
/// ```
pub trait ShellError: std::error::Error {
    /// Categorical reason for this failure. See [`ShellErrorKind`] for
    /// the per-shell-applicability matrix.
    fn kind(&self) -> ShellErrorKind;
}

/// Direction parameter for `kind_from_transport` — `TransportError::Closed`
/// has different shell-kind disposition on senders (`Closed`) vs receivers
/// (`EndOfStream`). See `kind_from_transport` for the routing.
#[allow(dead_code)] // used in Tasks 3-4 when shell error types are wired up
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    Send,
    Recv,
}

/// Compute the shell-kind for a `MuxError`. Used by `MuxSender`'s
/// `From<MuxError>` impl. **Every MuxError variant is matched explicitly**
/// — the CI ratchet `scripts/check-pipeline-kind-classification.sh`
/// enforces no variant escapes through a wildcard.
#[allow(dead_code)] // used in Task 3 when MuxSenderError is wired up
pub(crate) fn kind_from_mux(e: &MuxError) -> ShellErrorKind {
    use ShellErrorKind::*;
    match e {
        MuxError::InvalidConfig(_) => ConfigInvalid,
        MuxError::ConfigInvalid { .. } => ConfigInvalid,
        MuxError::InvalidNal => InputMalformed,
        MuxError::BufferFull { .. } => Backpressure,
        MuxError::KlvTooLarge { .. } => InputMalformed,
        MuxError::AudioTooLarge { .. } => InputMalformed,
        MuxError::InvalidStreamHandle { .. } => ConfigInvalid,
        MuxError::AmbiguousTarget { .. } => ConfigInvalid,
        MuxError::NoKlvStreamsConfigured => ConfigInvalid,
        MuxError::NoAudioStreamsConfigured => ConfigInvalid,
        MuxError::NoSubtitleStreamsConfigured => ConfigInvalid,
        MuxError::TooManyVideoStreams { .. } => ConfigInvalid,
        MuxError::TooManyKlvStreams { .. } => ConfigInvalid,
        MuxError::TooManyAudioStreams { .. } => ConfigInvalid,
        MuxError::TooManySubtitleStreams { .. } => ConfigInvalid,
        MuxError::SubtitleTooLarge { .. } => InputMalformed,
        MuxError::SubtitlePidUsedAsPcrPid { .. } => ConfigInvalid,
        MuxError::KlvPidUsedAsPcrPid { .. } => ConfigInvalid,
        MuxError::InvalidLanguageCode { .. } => ConfigInvalid,
        MuxError::InvalidTeletextField { .. } => ConfigInvalid,
        MuxError::PmtTooLarge { .. } => ConfigInvalid,
        MuxError::MalformedDescriptor { .. } => ConfigInvalid,
        MuxError::TooManyPrograms { .. } => ConfigInvalid,
        MuxError::EmptyProgram { .. } => ConfigInvalid,
        MuxError::DuplicateProgramNumber { .. } => ConfigInvalid,
        MuxError::DuplicatePmtPid { .. } => ConfigInvalid,
        MuxError::DuplicatePidAcrossPrograms { .. } => ConfigInvalid,
        MuxError::ProgramNotFound { .. } => ConfigInvalid,
        MuxError::PmtPidConflictsWithStream { .. } => ConfigInvalid,
        MuxError::SubtitleOnlyProgram { .. } => ConfigInvalid,
        MuxError::DescriptorIndexOutOfRange { .. } => ConfigInvalid,
        MuxError::AbsIndexOutOfRange { .. } => ConfigInvalid,
        // Required by #[non_exhaustive]. CI ratchet
        // scripts/check-pipeline-kind-classification.sh enforces every
        // upstream MuxError variant is matched above before this arm.
        // If this arm fires, the ratchet failed or was bypassed; the
        // shell error reports ConfigInvalid as a safe-default category
        // because the most common cause of an unmatched MuxError would
        // be a new config-validate failure.
        _ => ConfigInvalid,
    }
}

/// Compute the shell-kind for a `TransportError`. The `direction`
/// parameter distinguishes sender-shell-side (`Closed` -> `Closed` kind,
/// caller-initiated) from receiver-shell-side (`Closed` -> `EndOfStream`
/// kind, peer-initiated). `ExplicitClose` always maps to `Closed` kind
/// regardless of direction.
#[allow(dead_code)] // used in Tasks 3-4 when shell error types are wired up
pub(crate) fn kind_from_transport(e: &TransportError, direction: Direction) -> ShellErrorKind {
    use ShellErrorKind::*;
    match e {
        TransportError::Backpressure(_) => Backpressure,
        TransportError::Broken(_) => TransportBroken,
        TransportError::Closed => match direction {
            Direction::Send => Closed,
            Direction::Recv => EndOfStream,
        },
        TransportError::TooLarge { .. } => InputMalformed,
        TransportError::ExplicitClose => Closed,
        // Required by #[non_exhaustive]. CI ratchet enforces every variant
        // matched above. Safe default: TransportBroken (most common
        // category for a new transport-layer failure).
        _ => TransportBroken,
    }
}

/// Compute the shell-kind for a `DemuxError`. All 5 variants represent
/// "the input byte stream is malformed at this point" — all map to
/// `InputMalformed`. The exhaustive match documents that no variant has
/// a different disposition.
#[allow(dead_code)] // used in Task 4 when DemuxReceiverError is wired up
pub(crate) fn kind_from_demux(e: &DemuxError) -> ShellErrorKind {
    use ShellErrorKind::*;
    match e {
        DemuxError::Unrecoverable { .. } => InputMalformed,
        DemuxError::StrictRejection(_) => InputMalformed,
        DemuxError::MalformedPsi { .. } => InputMalformed,
        DemuxError::MalformedPes { .. } => InputMalformed,
        DemuxError::SyncBufExhausted { .. } => InputMalformed,
        // Required by #[non_exhaustive]. Safe default: InputMalformed
        // (matches every existing variant's disposition).
        _ => InputMalformed,
    }
}

/// Compute the shell-kind for a `TsFramingError`. Both variants are TS
/// sync-loss failures — `InputMalformed`. Same exhaustive-match pattern.
///
/// Note: `TsFramingError` is defined in the same crate so Rust treats
/// the match as exhaustive; no wildcard arm is needed (or permitted).
#[allow(dead_code)] // used in Task 3 when SenderError is wired up
pub(crate) fn kind_from_framing(e: &TsFramingError) -> ShellErrorKind {
    use ShellErrorKind::*;
    match e {
        TsFramingError::SyncLost { .. } => InputMalformed,
        TsFramingError::NoSyncAfterLimit { .. } => InputMalformed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_from_mux_invalid_config_is_config_invalid() {
        let kind = kind_from_mux(&MuxError::InvalidConfig("test"));
        assert_eq!(kind, ShellErrorKind::ConfigInvalid);
    }

    #[test]
    fn kind_from_mux_buffer_full_is_backpressure() {
        let kind = kind_from_mux(&MuxError::BufferFull {
            capacity_packets: 100,
        });
        assert_eq!(kind, ShellErrorKind::Backpressure);
    }

    #[test]
    fn kind_from_mux_invalid_nal_is_input_malformed() {
        let kind = kind_from_mux(&MuxError::InvalidNal);
        assert_eq!(kind, ShellErrorKind::InputMalformed);
    }

    #[test]
    fn kind_from_transport_closed_send_vs_recv() {
        assert_eq!(
            kind_from_transport(&TransportError::Closed, Direction::Send),
            ShellErrorKind::Closed
        );
        assert_eq!(
            kind_from_transport(&TransportError::Closed, Direction::Recv),
            ShellErrorKind::EndOfStream
        );
    }

    #[test]
    fn kind_from_transport_explicit_close_is_closed() {
        assert_eq!(
            kind_from_transport(&TransportError::ExplicitClose, Direction::Send),
            ShellErrorKind::Closed
        );
        assert_eq!(
            kind_from_transport(&TransportError::ExplicitClose, Direction::Recv),
            ShellErrorKind::Closed
        );
    }

    #[test]
    fn kind_from_transport_too_large_is_input_malformed() {
        assert_eq!(
            kind_from_transport(
                &TransportError::TooLarge {
                    len: 2000,
                    max: 1316
                },
                Direction::Send
            ),
            ShellErrorKind::InputMalformed
        );
    }

    #[test]
    fn kind_from_demux_all_variants_are_input_malformed() {
        for e in [
            DemuxError::Unrecoverable { after_bytes: 100 },
            DemuxError::StrictRejection("test".into()),
            DemuxError::MalformedPsi {
                pid: 0,
                reason: "test",
            },
            DemuxError::MalformedPes {
                pid: 0,
                reason: "test",
            },
            DemuxError::SyncBufExhausted {
                observed: 4096,
                max: 4096,
            },
        ] {
            assert_eq!(kind_from_demux(&e), ShellErrorKind::InputMalformed);
        }
    }

    #[test]
    fn kind_from_framing_all_variants_are_input_malformed() {
        assert_eq!(
            kind_from_framing(&TsFramingError::SyncLost { offset: 0 }),
            ShellErrorKind::InputMalformed
        );
        assert_eq!(
            kind_from_framing(&TsFramingError::NoSyncAfterLimit { max: 4096 }),
            ShellErrorKind::InputMalformed
        );
    }
}
