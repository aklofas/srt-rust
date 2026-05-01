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
use crate::mpegts::mux::{Config, Muxer};
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
