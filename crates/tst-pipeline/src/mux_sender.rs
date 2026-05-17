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
use tracing::{Span, info_span};
use tst_core::error::MuxError;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{
    AudioStreamHandle, KlvStreamHandle, Muxer, MuxerConfig, SubtitleStreamHandle, VideoStreamHandle,
};
use tst_core::transport::{Transport, TransportError};

use crate::shell_error::ShellErrorKind;

/// Stats snapshot for [`MuxSender`].
#[must_use]
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

/// Composes [`Muxer`] with a [`Transport`] for the canonical NAL+KLV → TS →
/// transport send path. See the module docs for shape and back-pressure
/// behavior.
///
/// # Panics
///
/// All `&self` methods (`send_*`, `*_handles`, `stats`, `reset_stats`,
/// `close`, `is_alive`) acquire an internal [`Mutex`] and panic if the
/// lock has been poisoned by a previous panic in another thread inside
/// the same `MuxSender`. This is the standard Rust `Mutex` behavior;
/// a poisoned lock signals that the muxer state may be inconsistent and
/// the `MuxSender` should be discarded.
///
/// # Closing
///
/// `MuxSender` is `Send + Sync` (when `T: Transport + Send + Sync`) and
/// supports three shutdown patterns:
///
/// 1. **Drop** — the [`Drop`] impl best-effort drains `pending_bytes` and
///    closes the underlying transport. Synchronous; bounded by
///    `SRTO_LINGER` (libsrt default 30 s, configurable via
///    `SocketBuilder::linger`).
/// 2. **Explicit close** — call [`Self::close`]. Cancels the transport
///    *before* taking the inner lock, so a peer thread parked in
///    `send_video` / `send_klv` returns
///    [`MuxSenderErrorSource::Transport`]`(`[`TransportError::Broken`]`)` within
///    one libsrt I/O cycle (~3-10 ms). Idempotent.
/// 3. **Cross-thread cancel** — call [`Self::cancel_handle`] to obtain a
///    `Send + Sync` [`tst_core::transport::TransportCancel`] handle,
///    then `cancel()` from any thread. Wakes a parked send without
///    closing the `MuxSender` itself; equivalent to what `close()`
///    fires internally.
///
/// ## Per-language idiom
///
/// | Language | Idiom |
/// |----------|-------|
/// | Rust | `let _ = sender;` (Drop) or `sender.cancel_handle().map(\|c\| c.cancel());` (cross-thread) |
/// | Java | Wrap as `AutoCloseable`; `try-with-resources` calls `close()` on exit |
/// | Kotlin | Wrap as `AutoCloseable`; `.use { }` calls `close()` on exit |
/// | Swift | `deinit` calls drop; `defer { handle.cancel() }` for explicit cross-thread |
/// | Python | Wrap as `__enter__`/`__exit__`; `with ... as sender:` calls `close()` on exit |
/// | C | `tst_mux_sender_close(sender)` (explicit; mirrors `Drop`) |
///
/// See [`docs/cancel-handle.md`](https://github.com/aklofas/ts-transformer/blob/main/ts-transformer/docs/cancel-handle.md) for the full cancel-handle pattern.
pub struct MuxSender<T: Transport> {
    inner: Mutex<Inner<T>>,
    /// Cancel handle snapshot, taken from the transport at construction
    /// time. Held outside the inner Mutex so `close()` can fire it
    /// without competing with a concurrent `send_*` for the lock.
    cancel: Option<Arc<dyn tst_core::transport::TransportCancel + Send + Sync>>,
    /// Lifetime [`tracing::Span`] opened in [`Self::new`] and entered
    /// from [`Drop`] to bracket open/close events. Private — must NOT
    /// be exposed publicly (see CI public-API ratchet).
    ///
    /// Wrapped in [`std::panic::AssertUnwindSafe`] because `Span`
    /// internally holds a `Mutex` which would otherwise flip this shell
    /// from `UnwindSafe`/`RefUnwindSafe` to `!UnwindSafe`/`!RefUnwindSafe`.
    /// `Span` is only entered in `new()` and `Drop`, never on hot paths,
    /// so asserting unwind safety is correct here.
    _span: std::panic::AssertUnwindSafe<Span>,
}

impl<T: Transport> std::fmt::Debug for MuxSender<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Acquire the inner Mutex briefly to read identity + lifecycle.
        // If poisoned (a panic happened mid-send), report poisoned-state
        // rather than panicking the formatter.
        match self.inner.lock() {
            Ok(inner) => f
                .debug_struct("MuxSender")
                .field("closed", &inner.closed)
                .field("video_streams", &inner.muxer.video_handles().len())
                .field("klv_streams", &inner.muxer.klv_handles().len())
                .field("audio_streams", &inner.muxer.audio_handles().len())
                .field("subtitle_streams", &inner.muxer.subtitle_handles().len())
                .field("pending_chunks", &inner.pending_bytes.len())
                .field("transport_kind", &std::any::type_name::<T>())
                .finish(),
            Err(_) => f
                .debug_struct("MuxSender")
                .field("inner", &"<poisoned>")
                .field("transport_kind", &std::any::type_name::<T>())
                .finish(),
        }
    }
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
    /// Last back-pressure state sampled by `maybe_warn_backpressure`.
    /// Used to fire `tracing::warn!` only on threshold-crossing
    /// (Ok→Warn or Warn→Overflow), not on every `send_*` call. Recovery
    /// transitions (Warn→Ok / Overflow→Warn) are silent.
    last_backpressure_state: BackpressureState,
}

/// Back-pressure tier on the muxer's internal packet queue. Ordering
/// matters: `Ok < Warn < Overflow`, so a strictly-greater comparison
/// (`new > last`) gives the threshold-crossing semantics that suppress
/// log spam at high pps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BackpressureState {
    /// `pending / cap < 0.8`.
    Ok,
    /// `0.8 <= pending / cap < 1.0` — one warn fires on entry.
    Warn,
    /// `pending / cap >= 1.0` — one warn fires on entry; the next
    /// `push_*` will return `MuxError::BufferFull`.
    Overflow,
}

impl<T: Transport> MuxSender<T> {
    pub fn new(transport: T, config: MuxerConfig) -> Result<Self, MuxError> {
        let span = info_span!(
            target: "tst_pipeline::mux_sender",
            "mux_sender",
            program_count = config.programs.len(),
            transport_kind = std::any::type_name::<T>(),
        );
        let _enter = span.enter();
        let muxer = Muxer::new(config)?;
        let cancel = transport.cancel_handle();
        tracing::info!("MuxSender opened");
        drop(_enter);
        Ok(Self {
            inner: Mutex::new(Inner {
                muxer,
                transport,
                pending_bytes: VecDeque::new(),
                closed: false,
                bytes_sent: 0,
                packets_sent: 0,
                last_backpressure_state: BackpressureState::Ok,
            }),
            cancel,
            _span: std::panic::AssertUnwindSafe(span),
        })
    }

    /// Send one video access unit. Annex-B framing is required.
    /// `pts` is in 90 kHz ticks (the TS clock); `key_frame` should
    /// be true for IDR.
    ///
    /// Resolves only when exactly one video stream is configured; with
    /// multiple video streams the muxer surfaces
    /// [`MuxError::AmbiguousTarget`] inside [`MuxSenderErrorSource::Mux`] —
    /// use [`Self::send_video_to`] in that case.
    ///
    /// # C ABI
    ///
    /// `tst_mux_sender_send_video` — see `crates/tst-c/include/tstrans.h`.
    ///
    /// # Typed PTS
    ///
    /// `pts: Pts90khz` is a newtype around the raw 90 kHz tick count. Construct
    /// from raw ticks with [`Pts90khz::new`] or from milliseconds with
    /// [`Pts90khz::from_millis`]. Internal arithmetic across the workspace still
    /// uses raw `i64`; a follow-up plan tracked in `docs/deferred-features.md`
    /// (landing later in this same plan) will design wrap-vs-saturate semantics
    /// on `Pts90khz` and do the full internal sweep.
    ///
    /// [`Pts90khz::new`]: tst_core::mpegts::common::Pts90khz::new
    /// [`Pts90khz::from_millis`]: tst_core::mpegts::common::Pts90khz::from_millis
    ///
    /// # Errors
    /// - [`MuxSenderErrorSource::Mux`] wraps [`MuxError`] from the underlying
    ///   muxer (e.g. `AmbiguousTarget` when more than one video stream
    ///   is configured, `InvalidStreamHandle` from `send_video_to`).
    /// - [`MuxSenderErrorSource::Transport`] wraps a [`TransportError`]; on
    ///   transport flap the unsent TS chunks are retained for a later
    ///   `send_*` call to drain.
    ///
    /// # Example
    /// ```
    /// use tst_pipeline::MuxSender;
    /// use tst_core::mpegts::common::Pts90khz;
    /// use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};
    /// use tst_core::transport::{Transport, TransportError};
    ///
    /// // In-memory sink; real callers plug in `tst_srt::SrtTransport`.
    /// struct Sink(Vec<u8>);
    /// impl Transport for Sink {
    ///     fn send_bytes(&mut self, b: &[u8]) -> Result<(), TransportError> {
    ///         self.0.extend_from_slice(b);
    ///         Ok(())
    ///     }
    ///     fn max_payload(&self) -> usize { 1316 }
    ///     fn close(&mut self) {}
    ///     fn is_alive(&self) -> bool { true }
    /// }
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
    /// prog.add_video(0x1011, VideoCodec::H264);
    /// let mut b = MuxerConfig::builder();
    /// b.add_program(prog.build());
    /// let cfg = b.build()?;
    /// let sender = MuxSender::new(Sink(Vec::new()), cfg)?;
    ///
    /// // Minimal Annex-B H.264 IDR NAL (start code + nal_unit_type=5).
    /// let nal = [0x00, 0x00, 0x00, 0x01, 0x65, 0xBB];
    /// sender.send_video(&nal, Pts90khz::new(0), true)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn send_video(
        &self,
        nal: &[u8],
        pts: Pts90khz,
        key_frame: bool,
    ) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_video(nal, pts.as_ticks(), key_frame)
    }

    /// Send one pre-built KLV blob. `pts` is in 90 kHz units (the
    /// TS clock); ignored unless the configured KLV stream carries PTS.
    ///
    /// `metadata_service_id` is written into the AU cell header per
    /// ITU-T H.222.0 V9 §2.12.4.2 / ST 1402.2 App. B Table 2 only for
    /// [`tst_core::mpegts::mux::KlvStreamType::SynchronousMetadata`] streams;
    /// ignored on [`tst_core::mpegts::mux::KlvStreamType::PrivateData`]
    /// streams. The spec default is `0x00`.
    ///
    /// # C ABI
    ///
    /// `tst_mux_sender_send_klv` — see `crates/tst-c/include/tstrans.h`.
    ///
    /// # Errors
    /// - [`MuxSenderErrorSource::Mux`] wraps [`MuxError`] from the inner muxer:
    ///   [`MuxError::NoKlvStreamsConfigured`] if no KLV streams exist;
    ///   [`MuxError::AmbiguousTarget`] when more than one is configured
    ///   (use [`Self::send_klv_to`]); [`MuxError::KlvTooLarge`] if the
    ///   blob would overflow `PES_packet_length`;
    ///   [`MuxError::BufferFull`] if the muxer's outbound queue is at
    ///   `MuxerConfig::buffer_packets`.
    /// - [`MuxSenderErrorSource::Transport`] wraps a [`TransportError`]; on
    ///   transport flap the unsent TS chunks are retained for a later
    ///   `send_*` call to drain.
    pub fn send_klv(
        &self,
        klv: &[u8],
        pts: Pts90khz,
        metadata_service_id: u8,
    ) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_klv(klv, pts.as_ticks(), metadata_service_id)
    }

    /// Send one video access unit to a specific configured video stream.
    /// `handle` is obtained from [`Self::video_handles`]; passing a handle
    /// from a different sender / muxer surfaces as
    /// [`MuxError::InvalidStreamHandle`] inside [`MuxSenderErrorSource::Mux`].
    ///
    /// # C ABI
    ///
    /// `tst_mux_sender_send_video_to` — see `crates/tst-c/include/tstrans.h`.
    ///
    /// # Errors
    /// - [`MuxSenderErrorSource::Mux`] wraps [`MuxError`] from the inner muxer:
    ///   [`MuxError::InvalidStreamHandle`] if `handle`'s index is out of
    ///   range for this muxer's video streams; [`MuxError::InvalidNal`]
    ///   if `nal` does not begin with an Annex-B start code (H.264 /
    ///   H.265 / H.266 only — AV1 OBU payloads skip this check);
    ///   [`MuxError::BufferFull`] if the muxer's outbound queue is at
    ///   `MuxerConfig::buffer_packets`.
    /// - [`MuxSenderErrorSource::Transport`] wraps a [`TransportError`]; on
    ///   transport flap the unsent TS chunks are retained for a later
    ///   `send_*` call to drain.
    pub fn send_video_to(
        &self,
        handle: VideoStreamHandle,
        nal: &[u8],
        pts: Pts90khz,
        key_frame: bool,
    ) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_video_to(handle, nal, pts.as_ticks(), key_frame)
    }

    /// Send one KLV blob to a specific configured KLV stream.
    ///
    /// `metadata_service_id` is written into the AU cell header per
    /// ITU-T H.222.0 V9 §2.12.4.2 / ST 1402.2 App. B Table 2 only for
    /// [`tst_core::mpegts::mux::KlvStreamType::SynchronousMetadata`] streams;
    /// ignored on [`tst_core::mpegts::mux::KlvStreamType::PrivateData`]
    /// streams. The spec default is `0x00`.
    ///
    /// # C ABI
    ///
    /// `tst_mux_sender_send_klv_to` — see `crates/tst-c/include/tstrans.h`.
    ///
    /// # Errors
    /// - [`MuxSenderErrorSource::Mux`] wraps [`MuxError`] from the inner muxer:
    ///   [`MuxError::InvalidStreamHandle`] if `handle`'s index is out of
    ///   range for this muxer's KLV streams; [`MuxError::KlvTooLarge`]
    ///   if the blob would overflow `PES_packet_length` (with a 5-byte
    ///   AU cell header reservation for `SynchronousMetadata` streams);
    ///   [`MuxError::BufferFull`] if the muxer's outbound queue is at
    ///   `MuxerConfig::buffer_packets`.
    /// - [`MuxSenderErrorSource::Transport`] wraps a [`TransportError`]; on
    ///   transport flap the unsent TS chunks are retained for a later
    ///   `send_*` call to drain.
    pub fn send_klv_to(
        &self,
        handle: KlvStreamHandle,
        klv: &[u8],
        pts: Pts90khz,
        metadata_service_id: u8,
    ) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_klv_to(handle, klv, pts.as_ticks(), metadata_service_id)
    }

    /// Send one audio frame buffer. `pts` is in 90 kHz ticks (the
    /// TS clock); audio always carries PTS (no DTS). `frames` is one or
    /// more pre-framed audio frames concatenated by the caller.
    ///
    /// Resolves only when exactly one audio stream is configured; with
    /// zero or multiple audio streams the muxer surfaces
    /// [`MuxError::AmbiguousTarget`] inside [`MuxSenderErrorSource::Mux`] — use
    /// [`Self::send_audio_to`] in that case.
    ///
    /// # C ABI
    ///
    /// No C counterpart yet (deferred to receiver-surface plan along with
    /// the audio stream-handle C surface).
    ///
    /// # Errors
    /// - [`MuxSenderErrorSource::Mux`] wraps [`MuxError`] from the inner muxer:
    ///   [`MuxError::NoAudioStreamsConfigured`] if no audio streams exist;
    ///   [`MuxError::AmbiguousTarget`] when more than one is configured;
    ///   [`MuxError::AudioTooLarge`] if `frames.len()` would overflow
    ///   `PES_packet_length`; [`MuxError::BufferFull`] if the muxer's
    ///   outbound queue is at `MuxerConfig::buffer_packets`.
    /// - [`MuxSenderErrorSource::Transport`] wraps a [`TransportError`]; on
    ///   transport flap the unsent TS chunks are retained for a later
    ///   `send_*` call to drain.
    pub fn send_audio(&self, frames: &[u8], pts: Pts90khz) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_audio(frames, pts.as_ticks())
    }

    /// Send one audio frame buffer to a specific configured audio stream.
    /// `handle` is obtained from [`Self::audio_handles`]; passing a handle
    /// from a different sender / muxer surfaces as
    /// [`MuxError::InvalidStreamHandle`] inside [`MuxSenderErrorSource::Mux`].
    ///
    /// # C ABI
    ///
    /// No C counterpart yet (deferred to receiver-surface plan along with
    /// the audio stream-handle C surface).
    ///
    /// # Errors
    /// - [`MuxSenderErrorSource::Mux`] wraps [`MuxError`] from the inner muxer:
    ///   [`MuxError::InvalidStreamHandle`] if `handle`'s index is out of
    ///   range for this muxer's audio streams;
    ///   [`MuxError::AudioTooLarge`] if `frames.len()` would overflow
    ///   `PES_packet_length`; [`MuxError::BufferFull`] if the muxer's
    ///   outbound queue is at `MuxerConfig::buffer_packets`.
    /// - [`MuxSenderErrorSource::Transport`] wraps a [`TransportError`]; on
    ///   transport flap the unsent TS chunks are retained for a later
    ///   `send_*` call to drain.
    pub fn send_audio_to(
        &self,
        handle: AudioStreamHandle,
        frames: &[u8],
        pts: Pts90khz,
    ) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_audio_to(handle, frames, pts.as_ticks())
    }

    /// Send one subtitle PES unit. `pts` is in 90 kHz ticks (the
    /// TS clock); subtitles carry PTS only. `payload` is one complete
    /// logical subtitle unit (DVB-sub composition page, teletext data
    /// field, CEA-708 service block, or WebVTT cue) — fragmentation
    /// across PES is not used.
    ///
    /// Resolves only when exactly one subtitle stream is configured;
    /// with zero or multiple subtitle streams the muxer surfaces
    /// [`MuxError::AmbiguousTarget`] inside [`MuxSenderErrorSource::Mux`] — use
    /// [`Self::send_subtitle_to`] in that case.
    ///
    /// # C ABI
    ///
    /// No C counterpart yet (deferred to receiver-surface plan along with
    /// the subtitle stream-handle C surface).
    ///
    /// # Errors
    /// - [`MuxSenderErrorSource::Mux`] wraps [`MuxError`] from the inner muxer:
    ///   [`MuxError::NoSubtitleStreamsConfigured`] if no subtitle streams
    ///   exist; [`MuxError::AmbiguousTarget`] when more than one is
    ///   configured; [`MuxError::SubtitleTooLarge`] if `payload.len()`
    ///   would overflow `PES_packet_length`; [`MuxError::BufferFull`] if
    ///   the muxer's outbound queue is at `MuxerConfig::buffer_packets`.
    /// - [`MuxSenderErrorSource::Transport`] wraps a [`TransportError`]; on
    ///   transport flap the unsent TS chunks are retained for a later
    ///   `send_*` call to drain.
    pub fn send_subtitle(&self, payload: &[u8], pts: Pts90khz) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_subtitle(payload, pts.as_ticks())
    }

    /// Send one subtitle PES unit to a specific configured subtitle stream.
    /// `handle` is obtained from [`Self::subtitle_handles`]; passing a
    /// handle from a different sender / muxer surfaces as
    /// [`MuxError::InvalidStreamHandle`] inside [`MuxSenderErrorSource::Mux`].
    ///
    /// # C ABI
    ///
    /// No C counterpart yet (deferred to receiver-surface plan along with
    /// the subtitle stream-handle C surface).
    ///
    /// # Errors
    /// - [`MuxSenderErrorSource::Mux`] wraps [`MuxError`] from the inner muxer:
    ///   [`MuxError::InvalidStreamHandle`] if `handle`'s index is out of
    ///   range for this muxer's subtitle streams;
    ///   [`MuxError::SubtitleTooLarge`] if `payload.len()` would overflow
    ///   `PES_packet_length`; [`MuxError::BufferFull`] if the muxer's
    ///   outbound queue is at `MuxerConfig::buffer_packets`.
    /// - [`MuxSenderErrorSource::Transport`] wraps a [`TransportError`]; on
    ///   transport flap the unsent TS chunks are retained for a later
    ///   `send_*` call to drain.
    pub fn send_subtitle_to(
        &self,
        handle: SubtitleStreamHandle,
        payload: &[u8],
        pts: Pts90khz,
    ) -> Result<(), MuxSenderError> {
        let mut inner = self.inner.lock().unwrap();
        inner.send_subtitle_to(handle, payload, pts.as_ticks())
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

    /// Wire-level transport stats (RTT, packet loss, bandwidth, queue
    /// depths) sourced from the underlying [`Transport::socket_stats`]
    /// implementation. Returns `None` when the transport doesn't expose
    /// comparable telemetry (test mocks) or when a managed wrapper has
    /// no live inner socket (mid-reconnect).
    ///
    /// # C ABI
    ///
    /// `tst_mux_sender_get_socket_stats` — see
    /// `crates/tst-c/include/tstrans.h`.
    pub fn socket_stats(&self) -> Option<tst_core::transport::SocketStats> {
        self.inner.lock().unwrap().transport.socket_stats()
    }

    /// Per-PID codec-specific counters. Delegates to the inner
    /// [`tst_core::mpegts::mux::Muxer::stream_codec_stats`].
    ///
    /// See [`tst_core::mpegts::stats::StreamCodecStats`] for the
    /// semantics of the return value (`None` vs `Some(Unknown)` vs
    /// typed variant).
    ///
    /// Result does NOT vary with transport reconnect state — the
    /// Muxer's per-PID state is independent of the live socket. The C
    /// ABI's `tst_managed_mux_sender_get_stream_codec_stats` returns
    /// the same values as `tst_mux_sender_get_stream_codec_stats`
    /// during reconnect; no `TST_E_NOT_AVAILABLE` is returned for
    /// codec stats.
    ///
    /// # C ABI
    ///
    /// `tst_mux_sender_get_stream_codec_stats` (plain) +
    /// `tst_managed_mux_sender_get_stream_codec_stats` (managed wrapper) —
    /// see `crates/tst-c/include/tstrans.h`.
    pub fn stream_codec_stats(
        &self,
        pid: u16,
    ) -> Option<tst_core::mpegts::stats::StreamCodecStats> {
        self.inner.lock().unwrap().muxer.stream_codec_stats(pid)
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

    /// Close the sender. Idempotent. Best-effort drains any pending
    /// bytes buffered during a prior back-pressure event, then marks
    /// the sender closed and closes the underlying transport.
    ///
    /// Wakes any thread parked inside `send_video` / `send_klv` / `send_*_to`
    /// by cancelling the underlying transport BEFORE acquiring the inner
    /// lock — so a peer thread waiting on libsrt's `srt_sendmsg` returns
    /// promptly with `TransportError::Broken`. Without this cancel-first
    /// step the close would deadlock against the parked send for the
    /// duration of `SRTO_SNDTIMEO` (or forever, on the libsrt default).
    ///
    /// Pending-bytes drain is best-effort; if the transport rejects on
    /// drain (typically because it's already broken), the bytes are
    /// silently abandoned. This matches Drop semantics.
    ///
    /// Poisoned-lock handling: if a prior panic poisoned the inner mutex,
    /// `close` silently returns rather than panic — parity with Drop.
    pub fn close(&self) {
        // Cancel-first: wake any peer thread parked inside
        // transport.send_bytes so they return TransportError::Broken and
        // release the inner Mutex. Otherwise we'd deadlock here waiting
        // for the lock. Must happen BEFORE the lock acquisition.
        if let Some(c) = &self.cancel {
            c.cancel();
        }
        // Graceful poisoned-lock handling — mirrors Drop's `if let Ok`.
        // If the lock is poisoned, the underlying transport may already
        // have closed itself via the cancel above; abandon pending.
        if let Ok(mut inner) = self.inner.lock() {
            // Best-effort drain BEFORE marking closed; otherwise drain_pending
            // bails on the `self.closed` guard inside its helpers (the
            // Inner::send_* methods short-circuit on closed; drain_pending
            // does not currently check `closed`, but matching the order
            // keeps the future-proof contract obvious).
            let _ = inner.drain_pending();
            inner.closed = true;
            inner.transport.close();
        }
    }

    /// Snapshot of the underlying transport's cancel handle, if it
    /// supports cancellation. Equivalent to what `close()` calls
    /// internally; exposed for callers who want to keep the MuxSender
    /// alive but still have an out-of-band wake-up mechanism.
    ///
    /// # C ABI
    ///
    /// `tst_mux_sender_cancel` — see `crates/tst-c/include/tstrans.h`.
    pub fn cancel_handle(
        &self,
    ) -> Option<Arc<dyn tst_core::transport::TransportCancel + Send + Sync>> {
        self.cancel.clone()
    }

    #[must_use]
    pub fn is_alive(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        !inner.closed && inner.transport.is_alive()
    }
}

impl<T: Transport> Drop for MuxSender<T> {
    fn drop(&mut self) {
        let _enter = self._span.0.enter();
        // Best-effort drain of pending_bytes on drop; if transport rejects,
        // they're discarded.
        if let Ok(mut inner) = self.inner.lock() {
            let _ = inner.drain_pending();
            inner.transport.close();
        }
        tracing::info!("MuxSender closed");
    }
}

/// Type alias for [`MuxSender`] with a boxed [`Transport`] trait object.
///
/// Bindings code (`srt-jni`, `srt-uniffi`, `tst-pyo3`) targets this single
/// concrete type instead of cubing per-`T` instantiation. Rust callers with a
/// custom transport keep the generic `MuxSender<MyTransport>` shape.
///
/// # Example — opaque sender from a runtime-chosen transport
/// ```no_run
/// use tst_pipeline::mux_sender::BoxedMuxSender;
/// use tst_pipeline::MuxSender;
/// use tst_core::Transport;
///
/// fn open(transport: Box<dyn Transport>) -> Result<BoxedMuxSender, Box<dyn std::error::Error>> {
///     Ok(MuxSender::new(transport, Default::default())?)
/// }
/// ```
pub type BoxedMuxSender = MuxSender<Box<dyn crate::Transport>>;

impl<T: Transport> Inner<T> {
    fn send_video(
        &mut self,
        nal: &[u8],
        pts_90khz: i64,
        key_frame: bool,
    ) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(TransportError::Closed.into());
        }
        // Drain any leftover from a previous failed call first.
        self.drain_pending()?;
        // Push and drain new content. Sample back-pressure between the
        // push (queue at peak) and the drain (queue back to zero).
        let push_result = self
            .muxer
            .push_video(nal, Pts90khz::new(pts_90khz), key_frame);
        self.maybe_warn_backpressure(matches!(push_result, Err(MuxError::BufferFull { .. })));
        push_result?;
        self.drain_muxer()
    }

    fn send_klv(
        &mut self,
        klv: &[u8],
        pts_90khz: i64,
        metadata_service_id: u8,
    ) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(TransportError::Closed.into());
        }
        self.drain_pending()?;
        let push_result = self
            .muxer
            .push_klv(klv, Pts90khz::new(pts_90khz), metadata_service_id);
        self.maybe_warn_backpressure(matches!(push_result, Err(MuxError::BufferFull { .. })));
        push_result?;
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
            return Err(TransportError::Closed.into());
        }
        self.drain_pending()?;
        let push_result =
            self.muxer
                .push_video_to(handle, nal, Pts90khz::new(pts_90khz), key_frame);
        self.maybe_warn_backpressure(matches!(push_result, Err(MuxError::BufferFull { .. })));
        push_result?;
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
            return Err(TransportError::Closed.into());
        }
        self.drain_pending()?;
        let push_result =
            self.muxer
                .push_klv_to(handle, klv, Pts90khz::new(pts_90khz), metadata_service_id);
        self.maybe_warn_backpressure(matches!(push_result, Err(MuxError::BufferFull { .. })));
        push_result?;
        self.drain_muxer()
    }

    fn send_audio(&mut self, frames: &[u8], pts_90khz: i64) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(TransportError::Closed.into());
        }
        self.drain_pending()?;
        let push_result = self.muxer.push_audio(frames, Pts90khz::new(pts_90khz));
        self.maybe_warn_backpressure(matches!(push_result, Err(MuxError::BufferFull { .. })));
        push_result?;
        self.drain_muxer()
    }

    fn send_audio_to(
        &mut self,
        handle: AudioStreamHandle,
        frames: &[u8],
        pts_90khz: i64,
    ) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(TransportError::Closed.into());
        }
        self.drain_pending()?;
        // Muxer parameter order is `(handle, pts, frames)`; the public
        // pipeline API mirrors `send_video` / `send_klv` (data first).
        let push_result = self
            .muxer
            .push_audio_to(handle, Pts90khz::new(pts_90khz), frames);
        self.maybe_warn_backpressure(matches!(push_result, Err(MuxError::BufferFull { .. })));
        push_result?;
        self.drain_muxer()
    }

    fn send_subtitle(&mut self, payload: &[u8], pts_90khz: i64) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(TransportError::Closed.into());
        }
        self.drain_pending()?;
        // Muxer parameter order is `(pts, payload)`; we present
        // `(payload, pts)` for symmetry with `send_video` / `send_klv`.
        let push_result = self.muxer.push_subtitle(Pts90khz::new(pts_90khz), payload);
        self.maybe_warn_backpressure(matches!(push_result, Err(MuxError::BufferFull { .. })));
        push_result?;
        self.drain_muxer()
    }

    fn send_subtitle_to(
        &mut self,
        handle: SubtitleStreamHandle,
        payload: &[u8],
        pts_90khz: i64,
    ) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(TransportError::Closed.into());
        }
        self.drain_pending()?;
        let push_result = self
            .muxer
            .push_subtitle_to(handle, Pts90khz::new(pts_90khz), payload);
        self.maybe_warn_backpressure(matches!(push_result, Err(MuxError::BufferFull { .. })));
        push_result?;
        self.drain_muxer()
    }

    /// Sample muxer queue depth and emit `tracing::warn!` once when the
    /// back-pressure tier transitions UP (`Ok→Warn` at >=80% of cap, or
    /// `Warn→Overflow` at >=100% of cap). Recovery transitions are
    /// silent. Called between `push_*` and `drain_muxer`, when the
    /// queue is at its peak depth for this `send_*` cycle.
    ///
    /// `push_was_buffer_full` flags the case where the just-attempted
    /// `push_*` returned [`MuxError::BufferFull`]: the queue depth is
    /// unchanged from before the failed push (so it may read below the
    /// cap), but the user-observable signal — a push that didn't fit —
    /// is the overflow transition.
    fn maybe_warn_backpressure(&mut self, push_was_buffer_full: bool) {
        let cap = self.muxer.capacity_packets();
        if cap == 0 {
            return;
        }
        let pending = self.muxer.pending_packets();
        // Integer arithmetic: `pending * 5 >= cap * 4` is exactly
        // `pending / cap >= 0.8`, no f64 dependency, no boundary rounding.
        let new_state = if push_was_buffer_full || pending >= cap {
            BackpressureState::Overflow
        } else if pending.saturating_mul(5) >= cap.saturating_mul(4) {
            BackpressureState::Warn
        } else {
            BackpressureState::Ok
        };
        if new_state > self.last_backpressure_state {
            match new_state {
                BackpressureState::Warn => tracing::warn!(
                    target: "tst_pipeline::mux_sender",
                    pending,
                    cap,
                    "back-pressure approaching cap (>=80%)",
                ),
                BackpressureState::Overflow => tracing::warn!(
                    target: "tst_pipeline::mux_sender",
                    pending,
                    cap,
                    "back-pressure at cap — sends will block or fail",
                ),
                BackpressureState::Ok => {}
            }
        }
        self.last_backpressure_state = new_state;
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
                    return Err(e.into());
                }
            }
        }
    }

    fn drain_pending(&mut self) -> Result<(), MuxSenderError> {
        while let Some(chunk) = self.pending_bytes.front() {
            let len = chunk.len() as u64;
            self.transport.send_bytes(chunk)?;
            // Only count after successful send.
            self.bytes_sent += len;
            self.packets_sent += 1;
            self.pending_bytes.pop_front();
        }
        Ok(())
    }
}

/// Error returned by [`MuxSender`] methods.
///
/// # Categorization
///
/// Bindings categorize failures via [`Self::kind`] (one of 6
/// [`ShellErrorKind`] variants); power users inspect [`Self::source`]
/// for the typed inner error.
///
/// # Reachable kinds
///
/// `MuxSender` can produce: `ConfigInvalid`, `InputMalformed`,
/// `Backpressure`, `TransportBroken`, `Closed`. `EndOfStream` is
/// receiver-only.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
#[error("MuxSender error ({kind:?}): {source}")]
pub struct MuxSenderError {
    /// Categorical reason for this failure.
    pub kind: ShellErrorKind,
    /// Typed inner error (the actual `MuxError` or `TransportError`
    /// instance produced by the underlying muxer / transport).
    #[source]
    pub source: MuxSenderErrorSource,
}

/// Typed source enum for [`MuxSenderError`]. One variant per error type
/// the underlying `MuxSender` internals can produce.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum MuxSenderErrorSource {
    #[error(transparent)]
    Mux(#[from] MuxError),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

impl From<MuxError> for MuxSenderError {
    fn from(e: MuxError) -> Self {
        Self {
            kind: crate::shell_error::kind_from_mux(&e),
            source: MuxSenderErrorSource::Mux(e),
        }
    }
}

impl From<TransportError> for MuxSenderError {
    fn from(e: TransportError) -> Self {
        Self {
            kind: crate::shell_error::kind_from_transport(&e, crate::shell_error::Direction::Send),
            source: MuxSenderErrorSource::Transport(e),
        }
    }
}

impl crate::shell_error::ShellError for MuxSenderError {
    fn kind(&self) -> ShellErrorKind {
        self.kind
    }
}

#[cfg(test)]
mod multi_stream_tests {
    use super::*;
    use tst_core::mpegts::mux::{
        AudioCodec, KlvStreamType, MuxerProgramConfigBuilder, StreamKind, SubtitleCodec, VideoCodec,
    };
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
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x1011, VideoCodec::H264);
            prog.add_video(0x1021, VideoCodec::H264);
            prog.add_klv(0x1031, KlvStreamType::PrivateData, false);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        assert_eq!(s.video_handles().len(), 2);
        assert_eq!(s.klv_handles().len(), 1);
    }

    #[test]
    fn sender_send_video_to_routes_through() {
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x1011, VideoCodec::H264);
            prog.add_video(0x1021, VideoCodec::H264);
            prog.add_klv(0x1031, KlvStreamType::PrivateData, false);
            prog.pcr_pid(0x1011);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let ir = s.video_handles()[1];
        let nal = [0x00, 0x00, 0x00, 0x01, 0x67, 0xBB];
        s.send_video_to(ir, &nal, Pts90khz::new(0), true).unwrap();
        // We can't read the transport bytes directly from outside the lock,
        // but we can confirm the call returns Ok and the sender is alive.
        assert!(s.is_alive());
    }

    #[test]
    fn stats_starts_with_per_stream_entries_for_configured_streams() {
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x100, VideoCodec::H264);
            prog.add_klv(0x101, KlvStreamType::PrivateData, false);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
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
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x100, VideoCodec::H264);
            prog.add_klv(0x101, KlvStreamType::PrivateData, false);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let nal: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67, 0xBB];
        s.send_video(nal, Pts90khz::new(0), true).unwrap();
        let st = s.stats();
        assert_eq!(st.per_stream[&0x100].items, 1);
        assert_eq!(st.per_stream[&0x100].bytes, nal.len() as u64);
        assert!(st.bytes_sent > 0);
        assert!(st.packets_sent > 0);
    }

    #[test]
    fn reset_stats_zeros_counters_keeps_per_stream() {
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x100, VideoCodec::H264);
            prog.add_klv(0x101, KlvStreamType::PrivateData, false);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let nal: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67, 0xBB];
        s.send_video(nal, Pts90khz::new(0), true).unwrap();
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
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x100, VideoCodec::H264);
            prog.add_audio(0x200, AudioCodec::Aac);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        // Synthetic audio frame bytes — the muxer doesn't validate the
        // codec payload here, so any non-empty buffer suffices.
        let frames = vec![0xFFu8; 64];
        s.send_audio(&frames, Pts90khz::new(90_000)).unwrap();
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
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x100, VideoCodec::H264);
            prog.add_audio(0x200, AudioCodec::Aac);
            prog.add_audio(0x201, AudioCodec::Mp2);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let handles = s.audio_handles();
        assert_eq!(handles.len(), 2);
        let frames = vec![0xAAu8; 32];
        s.send_audio_to(handles[1], &frames, Pts90khz::new(90_000))
            .unwrap();
        let st = s.stats();
        assert_eq!(st.per_stream[&0x201].items, 1);
        assert_eq!(st.per_stream[&0x200].items, 0);
        assert!(s.is_alive());
    }

    #[test]
    fn send_subtitle_pushes_through_pipeline() {
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x100, VideoCodec::H264);
            prog.add_subtitle(0x300, SubtitleCodec::WebVttInTs);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        // A minimal WebVTT-in-TS cue body (the muxer doesn't validate
        // contents — it just frames the bytes into a PES).
        let cue = b"WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nhello\n";
        s.send_subtitle(cue, Pts90khz::new(90_000)).unwrap();
        let st = s.stats();
        assert_eq!(st.per_stream[&0x300].items, 1);
        assert_eq!(st.per_stream[&0x300].bytes, cue.len() as u64);
        assert!(st.bytes_sent > 0);
        assert!(st.packets_sent > 0);
    }

    #[test]
    fn send_subtitle_to_routes_by_handle() {
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x100, VideoCodec::H264);
            prog.add_subtitle(0x300, SubtitleCodec::WebVttInTs);
            prog.add_subtitle(0x301, SubtitleCodec::WebVttInTs);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let handles = s.subtitle_handles();
        assert_eq!(handles.len(), 2);
        let cue = b"WEBVTT\n\n00:00:03.000 --> 00:00:04.000\nrouted\n";
        s.send_subtitle_to(handles[1], cue, Pts90khz::new(90_000))
            .unwrap();
        let st = s.stats();
        assert_eq!(st.per_stream[&0x301].items, 1);
        assert_eq!(st.per_stream[&0x300].items, 0);
        assert!(s.is_alive());
    }

    #[test]
    fn sender_send_video_rejects_when_multiple_video_streams_configured() {
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x1011, VideoCodec::H264);
            prog.add_video(0x1021, VideoCodec::H264);
            prog.add_klv(0x1031, KlvStreamType::PrivateData, false);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let nal = [0x00, 0x00, 0x00, 0x01, 0x67];
        let err = s.send_video(&nal, Pts90khz::new(0), true).unwrap_err();
        match err.source {
            MuxSenderErrorSource::Mux(MuxError::AmbiguousTarget {
                kind: StreamKind::Video,
                count: 2,
            }) => {}
            other => panic!("expected AmbiguousTarget, got {other:?}"),
        }
    }

    /// Transport that errors the first N send_bytes calls (back-pressure
    /// simulation), then accepts. Captured bytes are exposed via an external
    /// Arc<Mutex<Vec<u8>>> snoop slot since MuxSender takes the transport by
    /// value (MemTransport above isn't observable post-construction).
    struct BackpressureOnce {
        fail_remaining: std::sync::atomic::AtomicUsize,
        bytes: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    }
    impl BackpressureOnce {
        fn new(fail_first: usize, snoop: std::sync::Arc<std::sync::Mutex<Vec<u8>>>) -> Self {
            Self {
                fail_remaining: std::sync::atomic::AtomicUsize::new(fail_first),
                bytes: snoop,
            }
        }
    }
    impl Transport for BackpressureOnce {
        fn send_bytes(&mut self, b: &[u8]) -> Result<(), TransportError> {
            let prev = self
                .fail_remaining
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            if prev > 0 {
                return Err(TransportError::Backpressure(
                    "backpressure-once".to_string(),
                ));
            }
            self.bytes.lock().unwrap().extend_from_slice(b);
            Ok(())
        }
        fn max_payload(&self) -> usize {
            1316
        }
        fn close(&mut self) {}
        fn is_alive(&self) -> bool {
            true
        }
    }

    #[test]
    fn close_drains_pending_bytes() {
        // Reproduces PIPE-02: MuxSender::close must drain pending_bytes
        // before marking closed; otherwise queued back-pressure-buffered
        // chunks are silently abandoned on explicit close.
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x100, VideoCodec::H264);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
        // Snoop slot exposes BackpressureOnce's captured bytes externally.
        let snoop = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        // Fail first send_bytes: forces the muxer's emitted bundle to land in
        // pending_bytes (Inner::drain_muxer reacts to TransportError::Backpressure).
        let transport = BackpressureOnce::new(1, snoop.clone());
        let sender = MuxSender::new(transport, cfg).unwrap();

        // Minimal Annex-B H.264 IDR NAL.
        let nal = [0x00, 0x00, 0x00, 0x01, 0x65, 0xBB];
        // First send: muxer emits a bundle, transport rejects, bundle lands
        // in pending_bytes. send_video returns Err(Backpressure) — ignore;
        // the relevant assertion is about close's post-condition.
        let _ = sender.send_video(&nal, Pts90khz::new(0), true);

        sender.close();

        // Pre-fix: 0 bytes captured (pending abandoned by close).
        // Post-fix: > 0 bytes captured (close drained pending; transport's
        // 2nd send_bytes call succeeded with prev=0).
        let captured = snoop.lock().unwrap().len();
        assert!(
            captured > 0,
            "MuxSender::close must drain pending_bytes (parity with Drop); captured = {captured}"
        );
    }
}

#[cfg(test)]
mod cancel_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tst_core::mpegts::mux::{KlvStreamType, MuxerProgramConfigBuilder, VideoCodec};
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
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x100, VideoCodec::H264);
            prog.add_klv(0x101, KlvStreamType::PrivateData, false);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
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
        let send_thread =
            std::thread::spawn(move || s_send.send_video(&nal, Pts90khz::new(0), true));

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
            Err(ref err) if err.kind == ShellErrorKind::TransportBroken
        ));
    }

    /// Transport that panics on every `send_bytes` call. Used to poison
    /// the inner `Mutex<Inner<T>>` by triggering a panic with the lock
    /// held (the `MutexGuard` drops during unwinding, auto-poisoning).
    struct PanicOnSend;
    impl Transport for PanicOnSend {
        fn send_bytes(&mut self, _b: &[u8]) -> Result<(), TransportError> {
            panic!("intentional poison-the-lock panic for poisoned-lock test")
        }
        fn max_payload(&self) -> usize {
            1316
        }
        fn close(&mut self) {}
        fn is_alive(&self) -> bool {
            true
        }
    }

    /// PIPE-02 secondary regression: explicit `close()` on a `MuxSender`
    /// whose inner mutex was poisoned by a panic-during-send must NOT
    /// itself panic — it returns silently via the `if let Ok` branch,
    /// matching `Drop`'s graceful poisoned-lock catch.
    #[test]
    fn close_does_not_panic_on_poisoned_lock() {
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x100, VideoCodec::H264);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
        let sender = Arc::new(MuxSender::new(PanicOnSend, cfg).unwrap());

        // Poison the inner mutex: spawn a thread whose `send_video` call
        // dives into `transport.send_bytes`, which panics. The Mutex auto-
        // poisons during stack unwinding because the MutexGuard is on the
        // panicking frame.
        let s_panic = sender.clone();
        let handle = std::thread::spawn(move || {
            // Minimal Annex-B IDR NAL — the muxer will emit a bundle into
            // drain_muxer, which calls transport.send_bytes, which panics.
            let nal = [0x00, 0x00, 0x00, 0x01, 0x67, 0xBB];
            let _ = s_panic.send_video(&nal, Pts90khz::new(0), true);
        });
        let _ = handle.join(); // ignore the thread's panic payload

        // Pre-Task-5: this panics via `.unwrap()` on the poisoned lock.
        // Post-Task-5: returns silently via `if let Ok(mut inner)`.
        // Surviving the call IS the assertion.
        sender.close();
    }
}
