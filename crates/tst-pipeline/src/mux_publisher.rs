//! [`MuxPublisher`] — pipeline shell that owns a [`Muxer`] and pushes its
//! output to a [`Publisher`].

use std::sync::Mutex;
use std::time::Duration;

use tracing::info_span;
use tst_core::error::MuxError;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{Muxer, MuxerConfig};
use tst_core::publisher::{Publisher, PublisherStats};

use crate::shell_error::{ShellErrorKind, kind_from_mux};

/// Cumulative stats for a single `MuxPublisher`.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct MuxPublisherStats {
    /// Total TS bytes drained from the muxer and handed to the publisher.
    pub bytes_pushed: u64,
    /// Total muxer drain calls that produced ≥1 chunk.
    pub drain_calls: u64,
    /// Total explicit `cut_segment()` calls.
    pub cut_calls: u64,
}

struct Inner<P: Publisher> {
    muxer: Muxer,
    publisher: P,
    stats: MuxPublisherStats,
    closed: bool,
    /// PTS of the first AU pushed into the segment currently being built;
    /// `None` immediately after a cut (the next push re-baselines it).
    segment_start_pts: Option<Pts90khz>,
    /// PTS of the most recent video AU (drives the explicit `cut_segment`).
    last_video_pts: Option<Pts90khz>,
    /// Reusable scratch buffer for `drain_locked`. Allocated once on first
    /// push and reused for every subsequent drain call, avoiding a fresh
    /// 16 KB zero-init alloc per push.
    scratch: Vec<u8>,
}

/// Shell that owns a [`Muxer`] and pushes its output to a [`Publisher`].
/// Mirrors [`crate::MuxSender`].
pub struct MuxPublisher<P: Publisher> {
    inner: Mutex<Inner<P>>,
    /// Lifetime span, entered only during construction and `Drop` — see
    /// [`crate::shell_error::ShellSpan`] for the unwind-safety rationale.
    _span: crate::shell_error::ShellSpan,
}

impl<P: Publisher> std::fmt::Debug for MuxPublisher<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.inner.try_lock() {
            Ok(inner) => f
                .debug_struct("MuxPublisher")
                .field("bytes_pushed", &inner.stats.bytes_pushed)
                .field("drain_calls", &inner.stats.drain_calls)
                .field("cut_calls", &inner.stats.cut_calls)
                .field("closed", &inner.closed)
                .finish(),
            Err(_) => f
                .debug_struct("MuxPublisher")
                .field("locked", &true)
                .finish(),
        }
    }
}

impl<P: Publisher> MuxPublisher<P> {
    /// Build a publisher shell from a publisher + muxer config.
    pub fn with_config(
        publisher: P,
        config: MuxerConfig,
    ) -> Result<Self, MuxPublisherError<P::Error>> {
        let span = info_span!(
            target: "tst_pipeline::mux_publisher",
            "mux_publisher",
            program_count = config.programs.len(),
            publisher_kind = std::any::type_name::<P>(),
        );
        let _enter = span.enter();
        let muxer = Muxer::new(config).map_err(MuxPublisherError::Mux)?;
        tracing::info!("MuxPublisher opened");
        drop(_enter);
        Ok(Self {
            inner: Mutex::new(Inner {
                muxer,
                publisher,
                stats: MuxPublisherStats::default(),
                closed: false,
                segment_start_pts: None,
                last_video_pts: None,
                scratch: Vec::new(),
            }),
            _span: std::panic::AssertUnwindSafe(span),
        })
    }

    /// Send one video access unit (Annex-B framing required).  Calls
    /// [`Publisher::cut_segment_with_duration`] automatically when `key_frame` is true,
    /// passing the PTS span of the segment that just ended.
    pub fn send_video(
        &self,
        nal: &[u8],
        pts: Pts90khz,
        key_frame: bool,
    ) -> Result<(), MuxPublisherError<P::Error>> {
        let mut inner = self.inner.lock().expect("MuxPublisher poisoned");
        if inner.closed {
            return Err(MuxPublisherError::Closed);
        }
        if inner.segment_start_pts.is_none() {
            inner.segment_start_pts = Some(pts);
        }
        inner
            .muxer
            .push_video(nal, pts, key_frame)
            .map_err(MuxPublisherError::Mux)?;
        Self::drain_locked(&mut inner)?;
        inner.last_video_pts = Some(pts);
        if key_frame {
            let start = inner.segment_start_pts.unwrap_or(pts);
            let media_dur = media_span(start, pts);
            inner
                .publisher
                .cut_segment_with_duration(media_dur)
                .map_err(MuxPublisherError::Publisher)?;
            inner.stats.cut_calls = inner.stats.cut_calls.saturating_add(1);
            inner.segment_start_pts = None;
        }
        Ok(())
    }

    /// Send one KLV unit.
    ///
    /// `metadata_service_id` selects the KLV stream when multiple KLV PIDs
    /// are configured; pass `0` for single-stream muxer configs.
    pub fn send_klv(
        &self,
        klv: &[u8],
        pts: Pts90khz,
        metadata_service_id: u8,
    ) -> Result<(), MuxPublisherError<P::Error>> {
        let mut inner = self.inner.lock().expect("MuxPublisher poisoned");
        if inner.closed {
            return Err(MuxPublisherError::Closed);
        }
        inner
            .muxer
            .push_klv(klv, pts, metadata_service_id)
            .map_err(MuxPublisherError::Mux)?;
        Self::drain_locked(&mut inner)
    }

    /// Send one audio frame.
    pub fn send_audio(
        &self,
        frames: &[u8],
        pts: Pts90khz,
    ) -> Result<(), MuxPublisherError<P::Error>> {
        let mut inner = self.inner.lock().expect("MuxPublisher poisoned");
        if inner.closed {
            return Err(MuxPublisherError::Closed);
        }
        inner
            .muxer
            .push_audio(frames, pts)
            .map_err(MuxPublisherError::Mux)?;
        Self::drain_locked(&mut inner)
    }

    /// Send one subtitle payload.
    pub fn send_subtitle(
        &self,
        payload: &[u8],
        pts: Pts90khz,
    ) -> Result<(), MuxPublisherError<P::Error>> {
        let mut inner = self.inner.lock().expect("MuxPublisher poisoned");
        if inner.closed {
            return Err(MuxPublisherError::Closed);
        }
        // Note: Muxer::push_subtitle takes (pts, payload) — pts first.
        inner
            .muxer
            .push_subtitle(pts, payload)
            .map_err(MuxPublisherError::Mux)?;
        Self::drain_locked(&mut inner)
    }

    /// Send one data payload on the muxer's single data stream.
    ///
    /// Data streams are a PES pass-through — no AU-cell wrap, no framing;
    /// [`Muxer::push_data_to`] holds the contract (`pts` is written into
    /// the PES header only for `carries_pts: true` streams).
    pub fn send_data(&self, data: &[u8], pts: Pts90khz) -> Result<(), MuxPublisherError<P::Error>> {
        let mut inner = self.inner.lock().expect("MuxPublisher poisoned");
        if inner.closed {
            return Err(MuxPublisherError::Closed);
        }
        inner
            .muxer
            .push_data(data, pts)
            .map_err(MuxPublisherError::Mux)?;
        Self::drain_locked(&mut inner)
    }

    /// Explicit segment-cut hint.
    pub fn cut_segment(&self) -> Result<(), MuxPublisherError<P::Error>> {
        let mut inner = self.inner.lock().expect("MuxPublisher poisoned");
        if inner.closed {
            return Err(MuxPublisherError::Closed);
        }
        let media_dur = match (inner.segment_start_pts, inner.last_video_pts) {
            (Some(start), Some(end)) => media_span(start, end),
            _ => Duration::ZERO,
        };
        inner
            .publisher
            .cut_segment_with_duration(media_dur)
            .map_err(MuxPublisherError::Publisher)?;
        inner.stats.cut_calls = inner.stats.cut_calls.saturating_add(1);
        inner.segment_start_pts = None;
        Ok(())
    }

    /// Snapshot stats.
    pub fn stats(&self) -> MuxPublisherStats {
        self.inner.lock().expect("MuxPublisher poisoned").stats
    }

    /// Publisher-side stats (universal subset across publisher impls).
    pub fn publisher_stats(&self) -> PublisherStats {
        self.inner
            .lock()
            .expect("MuxPublisher poisoned")
            .publisher
            .stats()
    }

    /// Consume the shell, flush, and return the owned publisher.
    pub fn finish(self) -> Result<P, MuxPublisherError<P::Error>> {
        let Inner {
            muxer: _,
            publisher,
            stats: _,
            closed: _,
            segment_start_pts: _,
            last_video_pts: _,
            scratch: _,
        } = self.inner.into_inner().expect("MuxPublisher poisoned");
        Ok(publisher)
    }

    fn drain_locked(inner: &mut Inner<P>) -> Result<(), MuxPublisherError<P::Error>> {
        // Grow the reusable scratch to 16 KB on first use; subsequent calls
        // reuse the already-allocated buffer (no zero-init per push).
        const DRAIN_BUF_SIZE: usize = 16 * 1024;
        if inner.scratch.len() < DRAIN_BUF_SIZE {
            inner.scratch.resize(DRAIN_BUF_SIZE, 0);
        }
        loop {
            let n = inner.muxer.pull(&mut inner.scratch);
            if n == 0 {
                return Ok(());
            }
            inner
                .publisher
                .push_ts(&inner.scratch[..n])
                .map_err(MuxPublisherError::Publisher)?;
            inner.stats.bytes_pushed = inner.stats.bytes_pushed.saturating_add(n as u64);
            inner.stats.drain_calls = inner.stats.drain_calls.saturating_add(1);
        }
    }
}

/// Error returned by [`MuxPublisher`] methods.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MuxPublisherError<E: std::error::Error + Send + Sync + 'static> {
    /// The inner muxer rejected the input.
    #[error("muxer error: {0}")]
    Mux(#[source] MuxError),

    /// The publisher sink returned an error.
    #[error("publisher error: {0}")]
    Publisher(#[source] E),

    /// The shell has been consumed via [`MuxPublisher::finish`].
    #[error("MuxPublisher closed")]
    Closed,
}

impl<E: std::error::Error + Send + Sync + 'static> MuxPublisherError<E> {
    /// Cross-impl categorization for bindings.
    pub fn kind(&self) -> ShellErrorKind {
        match self {
            Self::Mux(e) => kind_from_mux(e),
            Self::Publisher(_) => ShellErrorKind::TransportBroken,
            Self::Closed => ShellErrorKind::Closed,
        }
    }
}

/// Media-presentation duration spanned from `start` to `end` PTS (90 kHz).
///
/// Saturates to zero on a non-increasing delta. PTS wraparound is out of
/// scope — a single segment does not span the ~26.5 h 33-bit PTS period.
fn media_span(start: Pts90khz, end: Pts90khz) -> Duration {
    let ticks = end.as_ticks().saturating_sub(start.as_ticks()).max(0) as u64;
    Duration::from_nanos(ticks * 1_000_000_000 / 90_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    struct MemoryPublisher {
        buffers: Vec<Vec<u8>>,
    }

    #[derive(Debug, thiserror::Error)]
    #[error("memory")]
    struct MemErr;

    impl Publisher for MemoryPublisher {
        type Error = MemErr;
        fn push_ts(&mut self, ts: &[u8]) -> Result<(), MemErr> {
            if self.buffers.is_empty() {
                self.buffers.push(Vec::new());
            }
            self.buffers.last_mut().unwrap().extend_from_slice(ts);
            Ok(())
        }
        fn cut_segment(&mut self) -> Result<(), MemErr> {
            self.buffers.push(Vec::new());
            Ok(())
        }
        fn finish(self) -> Result<(), MemErr> {
            Ok(())
        }
        fn stats(&self) -> PublisherStats {
            let mut s = PublisherStats::default();
            s.segments_written = self.buffers.len() as u64;
            s.bytes_written = self.buffers.iter().map(|v| v.len() as u64).sum();
            s
        }
    }

    fn test_muxer_config() -> MuxerConfig {
        use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x100, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.psi_interval_ms(10);
        b.build().unwrap()
    }

    #[test]
    fn cut_creates_new_segment() {
        let p = MemoryPublisher { buffers: vec![] };
        let pub_ = MuxPublisher::with_config(p, test_muxer_config()).unwrap();
        pub_.cut_segment().unwrap();
        pub_.cut_segment().unwrap();
        let publisher = pub_.finish().unwrap();
        assert_eq!(publisher.buffers.len(), 2);
    }

    #[test]
    fn stats_track_cut_calls() {
        let p = MemoryPublisher { buffers: vec![] };
        let pub_ = MuxPublisher::with_config(p, test_muxer_config()).unwrap();
        pub_.cut_segment().unwrap();
        pub_.cut_segment().unwrap();
        assert_eq!(pub_.stats().cut_calls, 2);
    }

    #[test]
    fn send_data_pushes_ts_bytes() {
        use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x100, VideoCodec::H264);
        prog.add_data(0x101, 0xF0, /*carries_pts=*/ true);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        let cfg = b.build().unwrap();
        let p = MemoryPublisher { buffers: vec![] };
        let pub_ = MuxPublisher::with_config(p, cfg).unwrap();
        pub_.send_data(&[0x42; 64], Pts90khz::new(0)).unwrap();
        assert!(pub_.stats().bytes_pushed > 0);
    }

    #[test]
    fn debug_impl_does_not_panic() {
        let p = MemoryPublisher { buffers: vec![] };
        let pub_ = MuxPublisher::with_config(p, test_muxer_config()).unwrap();
        let _ = format!("{:?}", pub_);
    }

    struct RecordingPublisher {
        cuts: Vec<Duration>,
    }

    impl Publisher for RecordingPublisher {
        type Error = MemErr;
        fn push_ts(&mut self, _ts: &[u8]) -> Result<(), MemErr> {
            Ok(())
        }
        fn cut_segment(&mut self) -> Result<(), MemErr> {
            self.cuts.push(Duration::ZERO);
            Ok(())
        }
        fn cut_segment_with_duration(&mut self, d: Duration) -> Result<(), MemErr> {
            self.cuts.push(d);
            Ok(())
        }
        fn finish(self) -> Result<(), MemErr> {
            Ok(())
        }
        fn stats(&self) -> PublisherStats {
            PublisherStats::default()
        }
    }

    fn h264_au() -> Vec<u8> {
        // AUD + one slice NAL + filler — the muxer packetizes the bytes; the
        // key_frame *bool* (not the NAL type) drives the segment cut.
        let mut v = vec![0x00, 0x00, 0x00, 0x01, 0x09, 0x10];
        v.extend([0x00, 0x00, 0x00, 0x01, 0x65]);
        v.extend(std::iter::repeat(0xab).take(64));
        v
    }

    #[test]
    fn send_video_derives_media_duration_from_pts() {
        // Sparse keyframes: IDR@0 closes a degenerate first segment (span 0);
        // P@9000, P@18000 build segment 1; IDR@270000 closes it with the PTS
        // span 9000..270000 = 261000 ticks = 2.9 s.
        let p = RecordingPublisher { cuts: vec![] };
        let mp = MuxPublisher::with_config(p, test_muxer_config()).unwrap();
        let au = h264_au();
        mp.send_video(&au, Pts90khz::new(0), true).unwrap();
        mp.send_video(&au, Pts90khz::new(9000), false).unwrap();
        mp.send_video(&au, Pts90khz::new(18000), false).unwrap();
        mp.send_video(&au, Pts90khz::new(270000), true).unwrap();
        let p = mp.finish().unwrap();
        assert_eq!(p.cuts.len(), 2);
        assert_eq!(p.cuts[0], Duration::ZERO);
        assert_eq!(
            p.cuts[1],
            Duration::from_nanos(261_000 * 1_000_000_000 / 90_000)
        );
    }
}
