// crates/srt-core/src/mpegts/demux/strict.rs
//! `StrictMode` — turns `NonConformant` events into hard errors per category.

use crate::mpegts::demux::event::NonConformantIssue;

/// Strictness category for the demuxer. Default `Off`.
///
/// Real-world ISR streams routinely omit `metadata_descriptor` (so a
/// "strict means strict-everything" mode would be unusable on most live
/// data). The categories let consumers fail on timing while tolerating
/// descriptor omissions, or vice versa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StrictMode {
    /// Lenient — `NonConformant` events surface as data; receive loop
    /// continues. This is the default.
    #[default]
    Off,
    /// Hard-fail on timing anomalies (`PcrAnomaly`, `PusiMidPes`).
    /// Tolerate descriptor and stream-type issues.
    TimingOnly,
    /// Hard-fail on missing `metadata_descriptor` and on stream-type
    /// mismatches. Tolerate timing anomalies.
    DescriptorsOnly,
    /// Hard-fail on every `NonConformantIssue` variant including
    /// future-added ones.
    Full,
}

impl StrictMode {
    /// Should this issue convert to a fatal `DemuxError::StrictRejection`?
    #[allow(dead_code)] // wired up by Task 9.
    pub(crate) fn rejects(self, issue: &NonConformantIssue) -> bool {
        match self {
            StrictMode::Off => false,
            StrictMode::TimingOnly => matches!(
                issue,
                NonConformantIssue::PcrAnomaly { .. }
                    | NonConformantIssue::PusiMidPes
                    | NonConformantIssue::PsiChecksumMismatch { .. }
            ),
            StrictMode::DescriptorsOnly => matches!(
                issue,
                NonConformantIssue::StreamTypeMismatchSyncOnAsyncPid
                    | NonConformantIssue::StreamTypeMismatchAsyncOnSyncPid
                    | NonConformantIssue::MissingMetadataDescriptor
            ),
            StrictMode::Full => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_rejects_nothing() {
        assert!(!StrictMode::Off.rejects(&NonConformantIssue::PusiMidPes));
        assert!(!StrictMode::Off.rejects(&NonConformantIssue::MissingMetadataDescriptor));
    }

    #[test]
    fn timing_only_rejects_timing() {
        let m = StrictMode::TimingOnly;
        assert!(m.rejects(&NonConformantIssue::PcrAnomaly { delta: 100_000 }));
        assert!(m.rejects(&NonConformantIssue::PusiMidPes));
        assert!(!m.rejects(&NonConformantIssue::MissingMetadataDescriptor));
    }

    #[test]
    fn descriptors_only_rejects_descriptors() {
        let m = StrictMode::DescriptorsOnly;
        assert!(m.rejects(&NonConformantIssue::MissingMetadataDescriptor));
        assert!(m.rejects(&NonConformantIssue::StreamTypeMismatchSyncOnAsyncPid));
        assert!(!m.rejects(&NonConformantIssue::PcrAnomaly { delta: 100_000 }));
    }

    #[test]
    fn full_rejects_all() {
        let m = StrictMode::Full;
        assert!(m.rejects(&NonConformantIssue::Other("anything".into())));
        assert!(m.rejects(&NonConformantIssue::MissingMetadataDescriptor));
        assert!(m.rejects(&NonConformantIssue::PcrAnomaly { delta: 0 }));
    }
}
