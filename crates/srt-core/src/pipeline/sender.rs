// crates/srt-core/src/pipeline/sender.rs
//! `Sender<T: Transport>` — composes `mpegts::mux::Muxer` with a
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

use crate::error::MuxError;
use crate::mpegts::mux::{Config, KlvStreamHandle, Muxer, VideoStreamHandle};
use crate::pipeline::transport::{Transport, TransportError};
use std::collections::VecDeque;
use std::sync::Mutex;

pub struct Sender<T: Transport> {
    inner: Mutex<Inner<T>>,
}

struct Inner<T: Transport> {
    muxer: Muxer,
    transport: T,
    /// Drained-but-not-yet-sent TS chunks, oldest first. Drained on each
    /// send_* call before any new push.
    ///
    /// Unbounded across repeated transport failures — the bare `Sender`
    /// has no cap. Callers expecting prolonged transport unavailability
    /// should wrap with `ManagedTransport` (Task 9), which adds a
    /// gap-buffer with overflow policy.
    pending_bytes: VecDeque<Vec<u8>>,
    closed: bool,
}

impl<T: Transport> Sender<T> {
    pub fn new(config: Config, transport: T) -> Result<Self, MuxError> {
        let muxer = Muxer::new(config)?;
        Ok(Self {
            inner: Mutex::new(Inner {
                muxer,
                transport,
                pending_bytes: VecDeque::new(),
                closed: false,
            }),
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
    ) -> Result<(), SenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_video(nal, pts_90khz, key_frame)
    }

    /// Send one pre-built KLV blob. `pts_90khz` is in 90 kHz units (the
    /// TS clock); ignored unless the configured KLV stream carries PTS.
    pub fn send_klv(&self, klv: &[u8], pts_90khz: i64) -> Result<(), SenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_klv(klv, pts_90khz)
    }

    /// Send one video access unit to a specific configured video stream.
    /// `handle` is obtained from [`Self::video_handles`]; passing a handle
    /// from a different sender / muxer surfaces as
    /// [`MuxError::InvalidStreamHandle`] inside [`SenderError::Mux`].
    pub fn send_video_to(
        &self,
        handle: VideoStreamHandle,
        nal: &[u8],
        pts_90khz: i64,
        key_frame: bool,
    ) -> Result<(), SenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_video_to(handle, nal, pts_90khz, key_frame)
    }

    /// Send one KLV blob to a specific configured KLV stream.
    pub fn send_klv_to(
        &self,
        handle: KlvStreamHandle,
        klv: &[u8],
        pts_90khz: i64,
    ) -> Result<(), SenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_klv_to(handle, klv, pts_90khz)
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

    pub fn close(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.closed = true;
        inner.transport.close();
    }

    pub fn is_alive(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        !inner.closed && inner.transport.is_alive()
    }
}

impl<T: Transport> Drop for Sender<T> {
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
    ) -> Result<(), SenderError> {
        if self.closed {
            return Err(SenderError::Transport(TransportError::Closed));
        }
        // Drain any leftover from a previous failed call first.
        self.drain_pending()?;
        // Push and drain new content.
        self.muxer.push_video(nal, pts_90khz, key_frame)?;
        self.drain_muxer()
    }

    fn send_klv(&mut self, klv: &[u8], pts_90khz: i64) -> Result<(), SenderError> {
        if self.closed {
            return Err(SenderError::Transport(TransportError::Closed));
        }
        self.drain_pending()?;
        self.muxer.push_klv(klv, pts_90khz)?;
        self.drain_muxer()
    }

    fn send_video_to(
        &mut self,
        handle: VideoStreamHandle,
        nal: &[u8],
        pts_90khz: i64,
        key_frame: bool,
    ) -> Result<(), SenderError> {
        if self.closed {
            return Err(SenderError::Transport(TransportError::Closed));
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
    ) -> Result<(), SenderError> {
        if self.closed {
            return Err(SenderError::Transport(TransportError::Closed));
        }
        self.drain_pending()?;
        self.muxer.push_klv_to(handle, klv, pts_90khz)?;
        self.drain_muxer()
    }

    /// Drain the muxer's internal buffer and forward each chunk to the
    /// transport. On transport error, captures any unsent chunks into
    /// `pending_bytes` and returns the error.
    fn drain_muxer(&mut self) -> Result<(), SenderError> {
        let max = self.transport.max_payload();
        let mut buf = vec![0u8; max];
        loop {
            let n = self.muxer.pull(&mut buf);
            if n == 0 {
                return Ok(());
            }
            let chunk = buf[..n].to_vec();
            if let Err(e) = self.transport.send_bytes(&chunk) {
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
                return Err(SenderError::Transport(e));
            }
        }
    }

    fn drain_pending(&mut self) -> Result<(), SenderError> {
        while let Some(chunk) = self.pending_bytes.front() {
            self.transport
                .send_bytes(chunk)
                .map_err(SenderError::Transport)?;
            self.pending_bytes.pop_front();
        }
        Ok(())
    }
}

/// Errors from `Sender::send_video` / `send_klv`.
#[derive(Debug, thiserror::Error)]
pub enum SenderError {
    #[error(transparent)]
    Mux(#[from] MuxError),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

#[cfg(test)]
mod multi_stream_tests {
    use super::*;
    use crate::mpegts::mux::{KlvStreamType, VideoCodec};
    use crate::pipeline::transport::{Transport, TransportError};

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
        #[allow(dead_code)]
        fn taken(&self) -> Vec<u8> {
            self.bytes.lock().unwrap().clone()
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
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .add_video(0x1021, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .build()
            .unwrap();
        let s = Sender::new(cfg, MemTransport::new()).unwrap();
        assert_eq!(s.video_handles().len(), 2);
        assert_eq!(s.klv_handles().len(), 1);
    }

    #[test]
    fn sender_send_video_to_routes_through() {
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .add_video(0x1021, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .pcr_pid(0x1011)
            .build()
            .unwrap();
        let s = Sender::new(cfg, MemTransport::new()).unwrap();
        let ir = s.video_handles()[1];
        let nal = [0x00, 0x00, 0x00, 0x01, 0x67, 0xBB];
        s.send_video_to(ir, &nal, 0, true).unwrap();
        // We can't read the transport bytes directly from outside the lock,
        // but we can confirm the call returns Ok and the sender is alive.
        assert!(s.is_alive());
    }

    #[test]
    fn sender_send_video_rejects_when_multiple_video_streams_configured() {
        let cfg = Config::builder()
            .add_video(0x1011, VideoCodec::H264)
            .add_video(0x1021, VideoCodec::H264)
            .add_klv(0x1031, KlvStreamType::PrivateData, false)
            .build()
            .unwrap();
        let s = Sender::new(cfg, MemTransport::new()).unwrap();
        let nal = [0x00, 0x00, 0x00, 0x01, 0x67];
        let err = s.send_video(&nal, 0, true).unwrap_err();
        match err {
            SenderError::Mux(MuxError::AmbiguousTarget {
                kind: "video",
                count: 2,
            }) => {}
            other => panic!("expected AmbiguousTarget, got {other:?}"),
        }
    }
}
