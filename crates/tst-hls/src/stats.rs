//! Per-publisher stats.

/// Richer HLS-specific stats.  Cross-impl callers should use
/// [`tst_core::publisher::PublisherStats`] instead.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct HlsStats {
    /// Total completed segments (history + current run).
    pub segments_written: u64,
    /// Total bytes accepted by `push_ts` (sum across all segments).
    pub bytes_pushed_total: u64,
    /// Bytes in the currently-open segment (0 between cuts).
    pub open_segment_bytes: u64,
    /// Segments cut by the wall-clock hard-cap fallback because a keyframe was
    /// overdue (keyframe-driven flow only). A persistently non-zero value
    /// means the upstream GOP length exceeds the configured cap.
    pub forced_cuts: u64,
}
