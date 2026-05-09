//! `MuxSender<T: Transport>` — composes `mpegts::mux::Muxer` with a
//! `Transport` for the canonical NAL+KLV → TS → SRT send path.
//!
//! Internally synchronized: `send_video` and `send_klv` may be called
//! from different threads concurrently. The lock is held across push →
//! mux drain → transport send for correct back-pressure.
//!
//! Lossless on transient transport errors: drained-but-not-yet-sent
//! bytes are retained in `pending_bytes` and drained first on the next
//! call. Only catastrophic transport failures (Broken/Closed) are
//! propagated to the caller; those are the cases where `ManagedTransport`
//! is the right wrapper.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use tst_core::error::MuxError;
use tst_core::mpegts::mux::{
    AudioStreamHandle, KlvStreamHandle, Muxer, MuxerConfig, SubtitleStreamHandle,
    VideoStreamHandle,
};
use tst_core::transport::{Transport, TransportError};

/// Stats snapshot for [`MuxSender`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MuxSenderStats {
    /// Cumulative bytes successfully handed off to the transport.
    pub bytes_sent: u64,
    /// Cumulative chunk count successfully handed off to the transport.
    /// Each chunk is one `transport.send_bytes` call that returned `Ok`.
    pub packets_sent: u64,
    /// Live gauge — bytes currently buffered in `pending_bytes` after
    /// a transport flap. NOT a counter; reflects current state.
    pub pending_bytes_queued: u64,
    /// Live gauge — chunk count currently in the pending buffer.
    pub pending_chunks_queued: u64,
    /// Number of programs (PAT entries) in the muxer configuration.
    /// Delegated from the inner `MuxerStats`.
    pub programs_configured: u32,
    /// Per-stream push counters, keyed by PID. Delegated from the wrapped
    /// `Muxer`; not double-booked here.
    pub per_stream: BTreeMap<u16, tst_core::mpegts::stats::StreamStats>,
}

pub struct MuxSender<T: Transport> {
    inner: Mutex<Inner<T>>,
    /// Cancel handle snapshot, taken from the transport at construction
    /// time. Held outside the inner Mutex so `close()` can fire it
    /// without competing with a concurrent `send_*` for the lock.
    cancel: Option<Arc<dyn tst_core::transport::TransportCancel + Send + Sync>>,
}

struct Inner<T: Transport> {
    muxer: Muxer,
    transport: T,
    /// Drained-but-not-yet-sent TS chunks, oldest first. Drained on each
    /// send_* call before any new push.
    ///
    /// Unbounded across repeated transport failures — the bare `MuxSender`
    /// has no cap. Callers expecting prolonged transport unavailability
    /// should wrap with `ManagedTransport` (Task 9), which adds a
    /// gap-buffer with overflow policy.
    pending_bytes: VecDeque<Vec<u8>>,
    closed: bool,
    bytes_sent: u64,
    packets_sent: u64,
}

impl<T: Transport> MuxSender<T> {
    pub fn new(transport: T, config: MuxerConfig) -> Result<Self, MuxError> {
        let muxer = Muxer::new(config)?;
        let cancel = transport.cancel_handle();
        Ok(Self {
            inner: Mutex::new(Inner {
                muxer,
                transport,
                pending_bytes: VecDeque::new(),
                closed: false,
                bytes_sent: 0,
                packets_sent: 0,
            }),
            cancel,
        })
    }

    /// Send one video access unit. Annex-B framing is required.
    /// `pts_90khz` is in 90 kHz ticks (the TS clock); `key_frame` should
    /// be true for IDR.
    pub fn send_video(
        &self,
        nal: &[u8],
        pts_90khz: i64,
        key_frame: bool,
    ) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_video(nal, pts_90khz, key_frame)
    }

    /// Send one pre-built KLV blob. `pts_90khz` is in 90 kHz units (the
    /// TS clock); ignored unless the configured KLV stream carries PTS.
    ///
    /// `metadata_service_id` is written into the AU cell header per
    /// ITU-T H.222.0 V9 §2.12.4.2 / ST 1402.2 App. B Table 2 only for
    /// [`tst_core::mpegts::mux::KlvStreamType::SynchronousMetadata`] streams;
    /// ignored on [`tst_core::mpegts::mux::KlvStreamType::PrivateData`]
    /// streams. The spec default is `0x00`.
    pub fn send_klv(
        &self,
        klv: &[u8],
        pts_90khz: i64,
        metadata_service_id: u8,
    ) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_klv(klv, pts_90khz, metadata_service_id)
    }

    /// Send one video access unit to a specific configured video stream.
    /// `handle` is obtained from [`Self::video_handles`]; passing a handle
    /// from a different sender / muxer surfaces as
    /// [`MuxError::InvalidStreamHandle`] inside [`MuxSenderError::Mux`].
    pub fn send_video_to(
        &self,
        handle: VideoStreamHandle,
        nal: &[u8],
        pts_90khz: i64,
        key_frame: bool,
    ) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_video_to(handle, nal, pts_90khz, key_frame)
    }

    /// Send one KLV blob to a specific configured KLV stream.
    ///
    /// `metadata_service_id` is written into the AU cell header per
    /// ITU-T H.222.0 V9 §2.12.4.2 / ST 1402.2 App. B Table 2 only for
    /// [`tst_core::mpegts::mux::KlvStreamType::SynchronousMetadata`] streams;
    /// ignored on [`tst_core::mpegts::mux::KlvStreamType::PrivateData`]
    /// streams. The spec default is `0x00`.
    pub fn send_klv_to(
        &self,
        handle: KlvStreamHandle,
        klv: &[u8],
        pts_90khz: i64,
        metadata_service_id: u8,
    ) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_klv_to(handle, klv, pts_90khz, metadata_service_id)
    }

    /// Send one audio frame buffer. `pts_90khz` is in 90 kHz ticks (the
    /// TS clock); audio always carries PTS (no DTS). `frames` is one or
    /// more pre-framed audio frames concatenated by the caller.
    ///
    /// Resolves only when exactly one audio stream is configured; with
    /// zero or multiple audio streams the muxer surfaces
    /// [`MuxError::AmbiguousTarget`] inside [`MuxSenderError::Mux`] — use
    /// [`Self::send_audio_to`] in that case.
    pub fn send_audio(&self, frames: &[u8], pts_90khz: i64) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_audio(frames, pts_90khz)
    }

    /// Send one audio frame buffer to a specific configured audio stream.
    /// `handle` is obtained from [`Self::audio_handles`]; passing a handle
    /// from a different sender / muxer surfaces as
    /// [`MuxError::InvalidStreamHandle`] inside [`MuxSenderError::Mux`].
    pub fn send_audio_to(
        &self,
        handle: AudioStreamHandle,
        frames: &[u8],
        pts_90khz: i64,
    ) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_audio_to(handle, frames, pts_90khz)
    }

    /// Send one subtitle PES unit. `pts_90khz` is in 90 kHz ticks (the
    /// TS clock); subtitles carry PTS only. `payload` is one complete
    /// logical subtitle unit (DVB-sub composition page, teletext data
    /// field, CEA-708 service block, or WebVTT cue) — fragmentation
    /// across PES is not used.
    ///
    /// Resolves only when exactly one subtitle stream is configured;
    /// with zero or multiple subtitle streams the muxer surfaces
    /// [`MuxError::AmbiguousTarget`] inside [`MuxSenderError::Mux`] — use
    /// [`Self::send_subtitle_to`] in that case.
    pub fn send_subtitle(&self, payload: &[u8], pts_90khz: i64) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_subtitle(payload, pts_90khz)
    }

    /// Send one subtitle PES unit to a specific configured subtitle stream.
    /// `handle` is obtained from [`Self::subtitle_handles`]; passing a
    /// handle from a different sender / muxer surfaces as
    /// [`MuxError::InvalidStreamHandle`] inside [`MuxSenderError::Mux`].
    pub fn send_subtitle_to(
        &self,
        handle: SubtitleStreamHandle,
        payload: &[u8],
        pts_90khz: i64,
    ) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_subtitle_to(handle, payload, pts_90khz)
    }

    /// Snapshot all video stream handles for this sender's muxer, in
    /// declaration order. Allocates an owned Vec so callers don't need
    /// to hold the lock.
    pub fn video_handles(&self) -> Vec<VideoStreamHandle> {
        self.inner.lock().unwrap().muxer.video_handles()
    }

    /// Snapshot all KLV stream handles for this sender's muxer.
    pub fn klv_handles(&self) -> Vec<KlvStreamHandle> {
        self.inner.lock().unwrap().muxer.klv_handles()
    }

    /// Snapshot all audio stream handles for this sender's muxer, in
    /// declaration order.
    pub fn audio_handles(&self) -> Vec<AudioStreamHandle> {
        self.inner.lock().unwrap().muxer.audio_handles()
    }

    /// Audio stream handles for the named program, in declaration order.
    /// Returns `Err(MuxError::ProgramNotFound)` if no program with the
    /// given number exists in this sender's muxer configuration.
    pub fn audio_handles_for_program(
        &self,
        program_number: u16,
    ) -> Result<Vec<AudioStreamHandle>, MuxError> {
        self.inner
            .lock()
            .unwrap()
            .muxer
            .audio_handles_for_program(program_number)
    }

    /// Snapshot all subtitle stream handles for this sender's muxer.
    pub fn subtitle_handles(&self) -> Vec<SubtitleStreamHandle> {
        self.inner.lock().unwrap().muxer.subtitle_handles()
    }

    /// Subtitle stream handles for the named program, in declaration
    /// order. Returns `Err(MuxError::ProgramNotFound)` if no program
    /// with the given number exists in this sender's muxer
    /// configuration.
    pub fn subtitle_handles_for_program(
        &self,
        program_number: u16,
    ) -> Result<Vec<SubtitleStreamHandle>, MuxError> {
        self.inner
            .lock()
            .unwrap()
            .muxer
            .subtitle_handles_for_program(program_number)
    }

    /// Return a point-in-time stats snapshot. `per_stream` is delegated from
    /// the inner `Muxer`; `pending_*` fields are live gauges.
    pub fn stats(&self) -> MuxSenderStats {
        let inner = self.inner.lock().unwrap();
        let mux_stats = inner.muxer.stats();
        let pending_bytes_queued: u64 = inner.pending_bytes.iter().map(|c| c.len() as u64).sum();
        let pending_chunks_queued = inner.pending_bytes.len() as u64;
        MuxSenderStats {
            bytes_sent: inner.bytes_sent,
            packets_sent: inner.packets_sent,
            pending_bytes_queued,
            pending_chunks_queued,
            programs_configured: mux_stats.programs_configured,
            per_stream: mux_stats.per_stream,
        }
    }

    /// Zero all flow counters and delegate to `Muxer::reset_stats`.
    /// `pending_bytes_queued` / `pending_chunks_queued` are live gauges and
    /// are NOT cleared.
    pub fn reset_stats(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.bytes_sent = 0;
        inner.packets_sent = 0;
        inner.muxer.reset_stats();
    }

    /// Close the sender. Idempotent.
    ///
    /// Wakes any thread parked inside `send_video` / `send_klv` / `send_*_to`
    /// by cancelling the underlying transport BEFORE acquiring the inner
    /// lock — so a peer thread waiting on libsrt's `srt_sendmsg` returns
    /// promptly with `TransportError::Broken`. Without this cancel-first
    /// step the close would deadlock against the parked send for the
    /// duration of `SRTO_SNDTIMEO` (or forever, on the libsrt default).
    pub fn close(&self) {
        // Cancel-first: wake any peer thread parked inside
        // transport.send_bytes so they return TransportError::Broken and
        // release the inner Mutex. Otherwise we'd deadlock here waiting
        // for the lock.
        if let Some(c) = &self.cancel {
            c.cancel();
        }
        let mut inner = self.inner.lock().unwrap();
        inner.closed = true;
        inner.transport.close();
    }

    /// Snapshot of the underlying transport's cancel handle, if it
    /// supports cancellation. Equivalent to what `close()` calls
    /// internally; exposed for callers who want to keep the MuxSender
    /// alive but still have an out-of-band wake-up mechanism.
    pub fn cancel_handle(&self) -> Option<Arc<dyn tst_core::transport::TransportCancel + Send + Sync>> {
        self.cancel.clone()
    }

    pub fn is_alive(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        !inner.closed && inner.transport.is_alive()
    }
}

impl<T: Transport> Drop for MuxSender<T> {
    fn drop(&mut self) {
        // Best-effort drain of pending_bytes on drop; if transport rejects,
        // they're discarded.
        if let Ok(mut inner) = self.inner.lock() {
            let _ = inner.drain_pending();
            inner.transport.close();
        }
    }
}

impl<T: Transport> Inner<T> {
    fn send_video(
        &mut self,
        nal: &[u8],
        pts_90khz: i64,
        key_frame: bool,
    ) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(MuxSenderError::Transport(TransportError::Closed));
        }
        // Drain any leftover from a previous failed call first.
        self.drain_pending()?;
        // Push and drain new content.
        self.muxer.push_video(nal, pts_90khz, key_frame)?;
        self.drain_muxer()
    }

    fn send_klv(
        &mut self,
        klv: &[u8],
        pts_90khz: i64,
        metadata_service_id: u8,
    ) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(MuxSenderError::Transport(TransportError::Closed));
        }
        self.drain_pending()?;
        self.muxer.push_klv(klv, pts_90khz, metadata_service_id)?;
        self.drain_muxer()
    }

    fn send_video_to(
        &mut self,
        handle: VideoStreamHandle,
        nal: &[u8],
        pts_90khz: i64,
        key_frame: bool,
    ) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(MuxSenderError::Transport(TransportError::Closed));
        }
        self.drain_pending()?;
        self.muxer
            .push_video_to(handle, nal, pts_90khz, key_frame)?;
        self.drain_muxer()
    }

    fn send_klv_to(
        &mut self,
        handle: KlvStreamHandle,
        klv: &[u8],
        pts_90khz: i64,
        metadata_service_id: u8,
    ) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(MuxSenderError::Transport(TransportError::Closed));
        }
        self.drain_pending()?;
        self.muxer
            .push_klv_to(handle, klv, pts_90khz, metadata_service_id)?;
        self.drain_muxer()
    }

    fn send_audio(&mut self, frames: &[u8], pts_90khz: i64) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(MuxSenderError::Transport(TransportError::Closed));
        }
        self.drain_pending()?;
        self.muxer.push_audio(frames, pts_90khz)?;
        self.drain_muxer()
    }

    fn send_audio_to(
        &mut self,
        handle: AudioStreamHandle,
        frames: &[u8],
        pts_90khz: i64,
    ) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(MuxSenderError::Transport(TransportError::Closed));
        }
        self.drain_pending()?;
        // Muxer parameter order is `(handle, pts, frames)`; the public
        // pipeline API mirrors `send_video` / `send_klv` (data first).
        self.muxer.push_audio_to(handle, pts_90khz, frames)?;
        self.drain_muxer()
    }

    fn send_subtitle(&mut self, payload: &[u8], pts_90khz: i64) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(MuxSenderError::Transport(TransportError::Closed));
        }
        self.drain_pending()?;
        // Muxer parameter order is `(pts, payload)`; we present
        // `(payload, pts)` for symmetry with `send_video` / `send_klv`.
        self.muxer.push_subtitle(pts_90khz, payload)?;
        self.drain_muxer()
    }

    fn send_subtitle_to(
        &mut self,
        handle: SubtitleStreamHandle,
        payload: &[u8],
        pts_90khz: i64,
    ) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(MuxSenderError::Transport(TransportError::Closed));
        }
        self.drain_pending()?;
        self.muxer.push_subtitle_to(handle, pts_90khz, payload)?;
        self.drain_muxer()
    }

    /// Drain the muxer's internal buffer and forward each chunk to the
    /// transport. On transport error, captures any unsent chunks into
    /// `pending_bytes` and returns the error.
    fn drain_muxer(&mut self) -> Result<(), MuxSenderError> {
        let max = self.transport.max_payload();
        let mut buf = vec![0u8; max];
        loop {
            let n = self.muxer.pull(&mut buf);
            if n == 0 {
                return Ok(());
            }
            let chunk = buf[..n].to_vec();
            match self.transport.send_bytes(&chunk) {
                Ok(()) => {
                    self.bytes_sent += chunk.len() as u64;
                    self.packets_sent += 1;
                }
                Err(e) => {
                    // Transport rejected the chunk — buffer it; do NOT count as sent.
                    self.pending_bytes.push_back(chunk);
                    // Drain any further muxer output into pending_bytes too,
                    // so the muxer's internal buffer doesn't fill up while
                    // transport is unavailable.
                    loop {
                        let n2 = self.muxer.pull(&mut buf);
                        if n2 == 0 {
                            break;
                        }
                        self.pending_bytes.push_back(buf[..n2].to_vec());
                    }
                    return Err(MuxSenderError::Transport(e));
                }
            }
        }
    }

    fn drain_pending(&mut self) -> Result<(), MuxSenderError> {
        while let Some(chunk) = self.pending_bytes.front() {
            let len = chunk.len() as u64;
            self.transport
                .send_bytes(chunk)
                .map_err(MuxSenderError::Transport)?;
            // Only count after successful send.
            self.bytes_sent += len;
            self.packets_sent += 1;
            self.pending_bytes.pop_front();
        }
        Ok(())
    }
}

/// Errors from `MuxSender::send_video` / `send_klv`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MuxSenderError {
    #[error(transparent)]
    Mux(#[from] MuxError),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

#[cfg(test)]
mod multi_stream_tests {
    use super::*;
    use tst_core::mpegts::mux::{AudioCodec, KlvStreamType, StreamKind, SubtitleCodec, VideoCodec};
    use tst_core::transport::{Transport, TransportError};

    /// In-memory transport that records every byte sent.
    struct MemTransport {
        bytes: std::sync::Mutex<Vec<u8>>,
        alive: std::sync::atomic::AtomicBool,
    }
    impl MemTransport {
        fn new() -> Self {
            Self {
                bytes: std::sync::Mutex::new(Vec::new()),
                alive: std::sync::atomic::AtomicBool::new(true),
            }
        }
    }
    impl Transport for MemTransport {
        fn send_bytes(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
            self.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(())
        }
        fn max_payload(&self) -> usize {
            1316
        }
        fn close(&mut self) {
            self.alive.store(false, std::sync::atomic::Ordering::SeqCst);
        }
        fn is_alive(&self) -> bool {
            self.alive.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[test]
    fn sender_video_handles_returns_one_per_configured_video_stream() {
        let cfg = MuxerConfig::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_video(0x1021, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .end_program()
            .build()
            .unwrap();
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        assert_eq!(s.video_handles().len(), 2);
        assert_eq!(s.klv_handles().len(), 1);
    }

    #[test]
    fn sender_send_video_to_routes_through() {
        let cfg = MuxerConfig::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_video(0x1021, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .pcr_pid(0x1011)
            .end_program()
            .build()
            .unwrap();
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let ir = s.video_handles()[1];
        let nal = [0x00, 0x00, 0x00, 0x01, 0x67, 0xBB];
        s.send_video_to(ir, &nal, 0, true).unwrap();
        // We can't read the transport bytes directly from outside the lock,
        // but we can confirm the call returns Ok and the sender is alive.
        assert!(s.is_alive());
    }

    #[test]
    fn stats_starts_with_per_stream_entries_for_configured_streams() {
        let cfg = MuxerConfig::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .end_program()
            .build()
            .unwrap();
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let st = s.stats();
        assert_eq!(st.bytes_sent, 0);
        assert_eq!(st.packets_sent, 0);
        assert_eq!(st.pending_bytes_queued, 0);
        assert_eq!(st.pending_chunks_queued, 0);
        assert_eq!(st.per_stream.len(), 2);
        assert!(st.per_stream.contains_key(&0x100));
    }

    #[test]
    fn stats_count_video_pushes() {
        let cfg = MuxerConfig::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .end_program()
            .build()
            .unwrap();
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let nal: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67, 0xBB];
        s.send_video(nal, 0, true).unwrap();
        let st = s.stats();
        assert_eq!(st.per_stream[&0x100].items, 1);
        assert_eq!(st.per_stream[&0x100].bytes, nal.len() as u64);
        assert!(st.bytes_sent > 0);
        assert!(st.packets_sent > 0);
    }

    #[test]
    fn reset_stats_zeros_counters_keeps_per_stream() {
        let cfg = MuxerConfig::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .end_program()
            .build()
            .unwrap();
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let nal: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67, 0xBB];
        s.send_video(nal, 0, true).unwrap();
        s.reset_stats();
        let st = s.stats();
        assert_eq!(st.bytes_sent, 0);
        assert_eq!(st.packets_sent, 0);
        assert_eq!(st.per_stream.len(), 2);
        assert_eq!(st.per_stream[&0x100].items, 0);
    }

    #[test]
    fn send_audio_pushes_through_pipeline() {
        // Single program, video + one audio stream. The bare send_audio
        // shorthand resolves because total_audio == 1 across the muxer.
        let cfg = MuxerConfig::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_audio(0x200, AudioCodec::Aac)
            .end_program()
            .build()
            .unwrap();
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        // Synthetic audio frame bytes — the muxer doesn't validate the
        // codec payload here, so any non-empty buffer suffices.
        let frames = vec![0xFFu8; 64];
        s.send_audio(&frames, 90_000).unwrap();
        let st = s.stats();
        assert_eq!(st.per_stream[&0x200].items, 1);
        assert_eq!(st.per_stream[&0x200].bytes, frames.len() as u64);
        assert!(st.bytes_sent > 0);
        assert!(st.packets_sent > 0);
    }

    #[test]
    fn send_audio_to_routes_by_handle() {
        // Two audio streams — bare send_audio would reject with
        // AmbiguousTarget; send_audio_to disambiguates via handle.
        let cfg = MuxerConfig::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_audio(0x200, AudioCodec::Aac)
            .add_audio(0x201, AudioCodec::Mp2)
            .end_program()
            .build()
            .unwrap();
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let handles = s.audio_handles();
        assert_eq!(handles.len(), 2);
        let frames = vec![0xAAu8; 32];
        s.send_audio_to(handles[1], &frames, 90_000).unwrap();
        let st = s.stats();
        assert_eq!(st.per_stream[&0x201].items, 1);
        assert_eq!(st.per_stream[&0x200].items, 0);
        assert!(s.is_alive());
    }

    #[test]
    fn send_subtitle_pushes_through_pipeline() {
        let cfg = MuxerConfig::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_subtitle(0x300, SubtitleCodec::WebVttInTs)
            .end_program()
            .build()
            .unwrap();
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        // A minimal WebVTT-in-TS cue body (the muxer doesn't validate
        // contents — it just frames the bytes into a PES).
        let cue = b"WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nhello\n";
        s.send_subtitle(cue, 90_000).unwrap();
        let st = s.stats();
        assert_eq!(st.per_stream[&0x300].items, 1);
        assert_eq!(st.per_stream[&0x300].bytes, cue.len() as u64);
        assert!(st.bytes_sent > 0);
        assert!(st.packets_sent > 0);
    }

    #[test]
    fn send_subtitle_to_routes_by_handle() {
        let cfg = MuxerConfig::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_subtitle(0x300, SubtitleCodec::WebVttInTs)
            .add_subtitle(0x301, SubtitleCodec::WebVttInTs)
            .end_program()
            .build()
            .unwrap();
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let handles = s.subtitle_handles();
        assert_eq!(handles.len(), 2);
        let cue = b"WEBVTT\n\n00:00:03.000 --> 00:00:04.000\nrouted\n";
        s.send_subtitle_to(handles[1], cue, 90_000).unwrap();
        let st = s.stats();
        assert_eq!(st.per_stream[&0x301].items, 1);
        assert_eq!(st.per_stream[&0x300].items, 0);
        assert!(s.is_alive());
    }

    #[test]
    fn sender_send_video_rejects_when_multiple_video_streams_configured() {
        let cfg = MuxerConfig::builder()
            .add_program(1, 0x1000)
            .add_video(0x1011, VideoCodec::H264)
            .add_video(0x1021, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .end_program()
            .build()
            .unwrap();
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let nal = [0x00, 0x00, 0x00, 0x01, 0x67];
        let err = s.send_video(&nal, 0, true).unwrap_err();
        match err {
            MuxSenderError::Mux(MuxError::AmbiguousTarget {
                kind: StreamKind::Video,
                count: 2,
            }) => {}
            other => panic!("expected AmbiguousTarget, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod cancel_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tst_core::mpegts::mux::{KlvStreamType, VideoCodec};
    use tst_core::transport::{Transport, TransportCancel, TransportError};

    /// Mock transport whose send_bytes blocks (parks) until cancel is
    /// triggered, simulating libsrt's send buffer being full.
    struct ParkableTransport {
        cancelled: Arc<AtomicBool>,
    }
    struct ParkableCancel {
        cancelled: Arc<AtomicBool>,
    }
    impl TransportCancel for ParkableCancel {
        fn cancel(&self) {
            self.cancelled.store(true, Ordering::SeqCst);
        }
    }
    impl Transport for ParkableTransport {
        fn send_bytes(&mut self, _: &[u8]) -> Result<(), TransportError> {
            // Spin-park until cancelled, then return Broken.
            for _ in 0..1000 {
                if self.cancelled.load(Ordering::SeqCst) {
                    return Err(TransportError::Broken("cancelled".into()));
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(TransportError::Broken(
                "test timeout (cancel never fired)".into(),
            ))
        }
        fn max_payload(&self) -> usize {
            1316
        }
        fn close(&mut self) {
            self.cancelled.store(true, Ordering::SeqCst);
        }
        fn is_alive(&self) -> bool {
            !self.cancelled.load(Ordering::SeqCst)
        }
        fn cancel_handle(&self) -> Option<std::sync::Arc<dyn TransportCancel + Send + Sync>> {
            Some(std::sync::Arc::new(ParkableCancel {
                cancelled: self.cancelled.clone(),
            }))
        }
    }

    /// `close()` from another thread unblocks a sender thread parked
    /// inside `send_video()`. Without cancel-first, the close call would
    /// itself block on the inner Mutex held by the parked sender.
    #[test]
    fn close_unblocks_parked_sender_thread() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let cfg = MuxerConfig::builder()
            .add_program(1, 0x1000)
            .add_video(0x100, VideoCodec::H264)
            .add_klv(0x101, KlvStreamType::PrivateData, false)
            .end_program()
            .build()
            .unwrap();
        let s = Arc::new(
            MuxSender::new(
                ParkableTransport {
                    cancelled: cancelled.clone(),
                },
                cfg,
            )
            .unwrap(),
        );
        let s_send = s.clone();

        let nal = vec![0x00, 0x00, 0x00, 0x01, 0x67, 0xBB];
        let send_thread = std::thread::spawn(move || s_send.send_video(&nal, 0, true));

        // Give the send thread a moment to grab the lock and park.
        std::thread::sleep(std::time::Duration::from_millis(50));

        // close() must NOT itself block on the inner Mutex; it cancels
        // first, the parked send returns Broken, then close lock-acquires.
        let close_start = std::time::Instant::now();
        s.close();
        let close_elapsed = close_start.elapsed();

        // Allow generous slack: the send thread sleeps 1ms between
        // checks, so the parked send returns within ~5ms after cancel.
        assert!(
            close_elapsed < std::time::Duration::from_millis(200),
            "close() blocked for {close_elapsed:?} — should have been near-instant via cancel"
        );

        let result = send_thread.join().unwrap();
        assert!(matches!(
            result,
            Err(MuxSenderError::Transport(TransportError::Broken(_)))
        ));
    }
}
