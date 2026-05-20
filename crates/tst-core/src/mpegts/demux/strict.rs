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
    pub(crate) fn rejects(self, issue: &NonConformantIssue) -> bool {
        match self {
            StrictMode::Off => false,
            StrictMode::TimingOnly => matches!(
                issue,
                NonConformantIssue::PcrAnomaly { .. }
                    | NonConformantIssue::PtsAnomaly { .. }
                    | NonConformantIssue::MissingRequiredPts { .. }
                    | NonConformantIssue::PcrMalformed { .. }
                    | NonConformantIssue::PusiMidPes
                    | NonConformantIssue::PsiChecksumMismatch { .. }
            ),
            StrictMode::DescriptorsOnly => matches!(
                issue,
                NonConformantIssue::StreamTypeMismatchSyncOnAsyncPid
                    | NonConformantIssue::StreamTypeMismatchAsyncOnSyncPid
                    | NonConformantIssue::MissingMetadataDescriptor
                    | NonConformantIssue::SubtitleMissingDescriptor { .. }
                    | NonConformantIssue::SubtitleDescriptorAmbiguous { .. }
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
    fn descriptors_only_rejects_subtitle_missing_descriptor() {
        // SubtitleMissingDescriptor is the subtitle-side parallel to
        // MissingMetadataDescriptor for KLV — both are descriptor-shape
        // concerns and DescriptorsOnly should escalate both.
        let m = StrictMode::DescriptorsOnly;
        assert!(m.rejects(&NonConformantIssue::SubtitleMissingDescriptor { pid: 0x100 }));
    }

    #[test]
    fn descriptors_only_rejects_subtitle_descriptor_ambiguous() {
        // Multi-marker ambiguity on a 0x06 PID is also a descriptor-shape
        // concern — DescriptorsOnly should escalate.
        let m = StrictMode::DescriptorsOnly;
        assert!(m.rejects(&NonConformantIssue::SubtitleDescriptorAmbiguous {
            pid: 0x100,
            tags: vec![0x59, 0xF0],
        }));
    }

    #[test]
    fn full_rejects_all() {
        let m = StrictMode::Full;
        assert!(m.rejects(&NonConformantIssue::Other("anything".into())));
        assert!(m.rejects(&NonConformantIssue::MissingMetadataDescriptor));
        assert!(m.rejects(&NonConformantIssue::PcrAnomaly { delta: 0 }));
    }

    #[test]
    fn timing_only_rejects_pts_anomaly_and_missing_required_pts() {
        // validate-1 B4 — PTS timing concerns join the timing-only cascade.
        let m = StrictMode::TimingOnly;
        assert!(m.rejects(&NonConformantIssue::PtsAnomaly { delta: -100_000 }));
        assert!(m.rejects(&NonConformantIssue::MissingRequiredPts { pid: 0x100 }));
    }

    #[test]
    fn ac3_sync_missing_only_rejected_by_full() {
        // validate-1 C12 — AC-3 syncframe alignment is a content-layer
        // concern, neither timing nor descriptor; only StrictMode::Full
        // escalates it.
        let issue = NonConformantIssue::Ac3SyncMissing { pid: 0x300 };
        assert!(!StrictMode::Off.rejects(&issue));
        assert!(!StrictMode::TimingOnly.rejects(&issue));
        assert!(!StrictMode::DescriptorsOnly.rejects(&issue));
        assert!(StrictMode::Full.rejects(&issue));
    }

    /// C11 — LATM framing is data-conformance (the bitstream itself, not
    /// timing or descriptors). Only `Full` strict mode rejects it; the
    /// narrower `TimingOnly` / `DescriptorsOnly` modes leave it as a
    /// surface-only NonConformant event.
    #[test]
    fn latm_framing_rejected_only_by_full() {
        use crate::codec::aac::latm::LatmFramingKind;
        let issue = NonConformantIssue::LatmFraming {
            pid: 0x101,
            kind: LatmFramingKind::MissingSyncword,
        };
        assert!(!StrictMode::Off.rejects(&issue));
        assert!(!StrictMode::TimingOnly.rejects(&issue));
        assert!(!StrictMode::DescriptorsOnly.rejects(&issue));
        assert!(StrictMode::Full.rejects(&issue));
    }
}
