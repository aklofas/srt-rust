//! Outbound-only, segment-aware sinks (HLS, future MPEG-DASH, ...).
//!
//! Sits alongside [`crate::transport::Transport`] and
//! [`crate::transport::RecvTransport`] as a third trait family.
//!
//! Unlike `Transport`, a `Publisher` is outbound-only — there is no peer to
//! receive from. Unlike a simple byte sink, a `Publisher` is aware of segment
//! boundaries (e.g., HLS .ts segment cuts, .m3u8 playlist rotation).

use std::time::Duration;

/// A sink for MPEG-TS bytes that produces a segmented output stream.
pub trait Publisher {
    /// Concrete error type returned by this publisher.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Push MPEG-TS bytes for the current segment.
    ///
    /// Bytes must be a whole multiple of 188.  The publisher is free to
    /// buffer; segment cuts happen at the next [`cut_segment`] call or
    /// when the configured segment duration cap is reached.
    ///
    /// [`cut_segment`]: Self::cut_segment
    fn push_ts(&mut self, ts_bytes: &[u8]) -> Result<(), Self::Error>;

    /// Hint that the next call to [`push_ts`] should start a new segment.
    ///
    /// Callers should invoke this on keyframe boundaries (IDR / I-frame)
    /// so segments are decodable from byte zero.  May be a no-op for
    /// publishers that segment purely on duration.
    ///
    /// [`push_ts`]: Self::push_ts
    fn cut_segment(&mut self) -> Result<(), Self::Error>;

    /// Cleanly finalize: flush pending segment, write a terminating
    /// playlist tag (e.g., HLS `#EXT-X-ENDLIST`), tear down sinks.
    ///
    /// Consumes `self`.  After this returns the publisher's resources
    /// (HTTP server, file handles, runtime) are released.
    fn finish(self) -> Result<(), Self::Error>
    where
        Self: Sized;

    /// Snapshot of publisher health.  Cheap to call (read-only over
    /// atomic counters).
    fn stats(&self) -> PublisherStats;
}

/// Universal cross-publisher stats.
///
/// Concrete impls expose richer stats via an inherent `stats() -> ConcreteStats`
/// method that this trait method projects from.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct PublisherStats {
    /// Total number of completed segments written.
    pub segments_written: u64,
    /// Total bytes pushed (counts toward whatever sink the publisher uses;
    /// for HLS this is bytes written to .ts files).
    pub bytes_written: u64,
    /// Wall-clock age of the segment currently open for writes.  `None`
    /// when no segment is open (between cuts or before first push).
    pub current_segment_age: Option<Duration>,
    /// Wall-clock duration of the most recently completed segment.
    pub last_segment_duration: Option<Duration>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal Publisher impl proving the trait is object-safe-via-generics
    /// and that `finish` consumes self correctly.
    struct NoopPublisher {
        segments: u64,
        bytes: u64,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("noop")]
    struct NoopErr;

    impl Publisher for NoopPublisher {
        type Error = NoopErr;
        fn push_ts(&mut self, ts: &[u8]) -> Result<(), NoopErr> {
            self.bytes += ts.len() as u64;
            Ok(())
        }
        fn cut_segment(&mut self) -> Result<(), NoopErr> {
            self.segments += 1;
            Ok(())
        }
        fn finish(self) -> Result<(), NoopErr> {
            Ok(())
        }
        fn stats(&self) -> PublisherStats {
            PublisherStats {
                segments_written: self.segments,
                bytes_written: self.bytes,
                ..Default::default()
            }
        }
    }

    #[test]
    fn noop_round_trip() {
        let mut p = NoopPublisher {
            segments: 0,
            bytes: 0,
        };
        p.push_ts(&[0u8; 188]).unwrap();
        p.cut_segment().unwrap();
        let s = p.stats();
        assert_eq!(s.bytes_written, 188);
        assert_eq!(s.segments_written, 1);
        p.finish().unwrap();
    }
}
