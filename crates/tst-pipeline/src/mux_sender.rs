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
//!
//! # Input consumption and retry
//!
//! The two error sources have opposite retry semantics:
//!
//! - [`MuxSenderErrorSource::Mux`]: the push is atomic — the muxer state is
//!   unchanged and none of the input's TS packets were produced. The caller
//!   MAY retry the same input after fixing the cause.
//! - [`MuxSenderErrorSource::Transport`]: two sub-cases, distinguishable by
//!   the PREVIOUS call's outcome (a successful `send_*` always leaves the
//!   pending queue empty):
//!   - if the previous `send_*` returned `Ok`, the failure happened while
//!     sending THIS input's TS bytes: the input **was consumed** — muxed
//!     (continuity counters advanced) and retained in the pending queue,
//!     which drains first on the next `send_*` call, exactly once, in
//!     order. Do **not** push the same input again: it would be muxed a
//!     second time and the stream would carry duplicate access units.
//!   - if the previous `send_*` ALSO returned a transport error, the
//!     failure may instead have hit the still-undrained retained bytes
//!     BEFORE this call's input was pushed — in that case this input was
//!     **not** consumed, and pushing different data next would lose it.
//!     Callers that must not drop access units across repeated transport
//!     failures should wrap the transport in [`crate::ManagedTransport`] rather
//!     than hand-rolling recovery on the bare shell.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use alloc::vec::Vec;
use tracing::info_span;
use tst_core::error::MuxError;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{
    AudioStreamHandle, DataStreamHandle, KlvStreamHandle, Muxer, MuxerConfig, SubtitleStreamHandle,
    VideoStreamHandle,
};
use tst_core::transport::{Transport, TransportError};

use crate::mutex::ShellMutex;
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
/// No method panics on a poisoned inner mutex. If a prior call panicked
/// mid-mutation and poisoned the lock, each method falls back gracefully:
/// fallible methods (`send_*`, `*_handles_for_program`) return a typed
/// [`MuxSenderError`] with kind [`ShellErrorKind::TransportBroken`] (or
/// [`MuxError::ProgramNotFound`]); infallible methods (`*_handles`,
/// `stats`, `socket_stats`, `stream_codec_stats`, `reset_stats`,
/// `is_alive`) return the corresponding safe default (`Vec::new()`,
/// `MuxSenderStats::default()`, `None`, silent no-op, `false`). `close`
/// and `Drop` already used `if let Ok` before this policy was formalized.
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
/// See [`docs/reference/srt-cancel-handle.md`](https://github.com/aklofas/ts-transformer/blob/main/ts-transformer/docs/reference/srt-cancel-handle.md) for the full cancel-handle pattern.
pub struct MuxSender<T: Transport> {
    inner: ShellMutex<Inner<T>>,
    /// Cancel handle snapshot, taken from the transport at construction
    /// time. Held outside the inner Mutex so `close()` can fire it
    /// without competing with a concurrent `send_*` for the lock.
    cancel: Option<Arc<dyn tst_core::transport::TransportCancel + Send + Sync>>,
    /// Lifetime span — see [`crate::shell_error::ShellSpan`] for the
    /// unwind-safe rationale. Private; never exposed publicly.
    _span: crate::shell_error::ShellSpan,
}

impl<T: Transport> core::fmt::Debug for MuxSender<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
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
                .field("transport_kind", &core::any::type_name::<T>())
                .finish(),
            Err(_) => f
                .debug_struct("MuxSender")
                .field("inner", &"<poisoned>")
                .field("transport_kind", &core::any::type_name::<T>())
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
    /// Reusable scratch buffer for `drain_muxer`. Sized to
    /// `transport.max_payload()` and grown lazily. Avoids a fresh
    /// heap allocation on every muxer drain call.
    scratch: Vec<u8>,
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

/// Build the poisoned-lock error for a named `send_*` site.
///
/// Called when `self.inner.lock()` returns `Err` (the Mutex is poisoned
/// because a previous `send_*` call panicked mid-mutation). Routes to
/// `MuxSenderError` with kind `TransportBroken` and a site-specific message
/// so the C ABI surfaces a useful diagnostic via `tst_get_last_error_str()`.
fn lock_poisoned(site: &'static str) -> MuxSenderError {
    MuxSenderError::from(TransportError::Broken {
        msg: alloc::format!("mux_sender: inner lock poisoned during {site}"),
        errno_code: None,
    })
}

impl<T: Transport> MuxSender<T> {
    pub fn new(transport: T, config: MuxerConfig) -> Result<Self, MuxError> {
        let span = info_span!(
            target: "tst_pipeline::mux_sender",
            "mux_sender",
            program_count = config.programs.len(),
            transport_kind = core::any::type_name::<T>(),
        );
        let _enter = span.enter();
        let muxer = Muxer::new(config)?;
        let cancel = transport.cancel_handle();
        tracing::info!("MuxSender opened");
        drop(_enter);
        Ok(Self {
            inner: ShellMutex::new(Inner {
                muxer,
                transport,
                pending_bytes: VecDeque::new(),
                closed: false,
                bytes_sent: 0,
                packets_sent: 0,
                last_backpressure_state: BackpressureState::Ok,
                scratch: Vec::new(),
            }),
            cancel,
            _span: core::panic::AssertUnwindSafe(span),
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
    /// `tst_mux_sender_send_video` — see `bindings/c/include/tstrans.h`.
    ///
    /// # Typed PTS
    ///
    /// `pts: Pts90khz` is a newtype around the raw 90 kHz tick count. Construct
    /// from raw ticks with [`Pts90khz::new`] or from milliseconds with
    /// [`Pts90khz::from_millis`]. Internal arithmetic across the workspace still
    /// uses raw `i64`; a follow-up plan tracked in `docs/project/deferred-features.md`
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
    ///   Whether the input was consumed depends on the failure point — see
    ///   the module-level *Input consumption and retry* section before
    ///   deciding whether to resend.
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
        // Mutex-poisoning policy (recoverable path): poisoned inner lock means a
        // previous panic happened mid-mutation. `lock_poisoned` routes to
        // `MuxSenderError` with kind `TransportBroken` and a site-specific
        // message so the C ABI surfaces a useful diagnostic via
        // `tst_get_last_error_str()`. Precedent: the gap-buffer policy in
        // `ManagedTransport::send_managed` / `drain_gap_if_alive` in reconnect/mod.rs.
        let mut inner = self.inner.lock().map_err(|_| lock_poisoned("send_video"))?;
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
    /// `tst_mux_sender_send_klv` — see `bindings/c/include/tstrans.h`.
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
    ///   Whether the input was consumed depends on the failure point — see
    ///   the module-level *Input consumption and retry* section before
    ///   deciding whether to resend.
    pub fn send_klv(
        &self,
        klv: &[u8],
        pts: Pts90khz,
        metadata_service_id: u8,
    ) -> Result<(), MuxSenderError> {
        // Mutex-poisoning policy — see send_video for rationale.
        let mut inner = self.inner.lock().map_err(|_| lock_poisoned("send_klv"))?;
        inner.send_klv(klv, pts.as_ticks(), metadata_service_id)
    }

    /// Send one video access unit to a specific configured video stream.
    /// `handle` is obtained from [`Self::video_handles`]; passing a handle
    /// from a different sender / muxer surfaces as
    /// [`MuxError::InvalidStreamHandle`] inside [`MuxSenderErrorSource::Mux`].
    ///
    /// # C ABI
    ///
    /// `tst_mux_sender_send_video_to` — see `bindings/c/include/tstrans.h`.
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
    ///   Whether the input was consumed depends on the failure point — see
    ///   the module-level *Input consumption and retry* section before
    ///   deciding whether to resend.
    pub fn send_video_to(
        &self,
        handle: VideoStreamHandle,
        nal: &[u8],
        pts: Pts90khz,
        key_frame: bool,
    ) -> Result<(), MuxSenderError> {
        // Mutex-poisoning policy — see send_video for rationale.
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| lock_poisoned("send_video_to"))?;
        inner.send_video_to(handle, nal, pts.as_ticks(), key_frame)
    }

    /// Send one access unit with explicit composition (PTS) and decode (DTS)
    /// timestamps. Required for reordered codecs (H.264/H.265/H.266/AV1
    /// streams with B-frames).
    ///
    /// Mirrors [`tst_core::mpegts::mux::Muxer::push_video_to_with_dts`]:
    /// the muxer emits PES with `PTS_DTS_flags = '11'` per
    /// ISO/IEC 13818-1 §2.4.3.6, carrying both timestamps. When
    /// `pts == dts`, prefer [`Self::send_video_to`] for the smaller
    /// 5-byte PTS-only PES encoding.
    ///
    /// **Caller invariant:** `dts <= pts` per §2.4.3.6. The muxer does
    /// not enforce this; receivers will reject inverted timestamps.
    ///
    /// # C ABI
    ///
    /// Not yet exposed via the C ABI. Callers needing B-frame support
    /// from C should bridge through the Rust API or open an issue
    /// requesting the C entry.
    ///
    /// # Errors
    /// - [`MuxSenderErrorSource::Mux`] wraps [`MuxError`] from the inner
    ///   muxer (same variants as [`Self::send_video_to`]).
    /// - [`MuxSenderErrorSource::Transport`] wraps a [`TransportError`].
    pub fn send_video_to_with_dts(
        &self,
        handle: VideoStreamHandle,
        nal: &[u8],
        pts: Pts90khz,
        dts: Pts90khz,
        key_frame: bool,
    ) -> Result<(), MuxSenderError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| lock_poisoned("send_video_to_with_dts"))?;
        inner.send_video_to_with_dts(handle, nal, pts.as_ticks(), dts.as_ticks(), key_frame)
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
    /// `tst_mux_sender_send_klv_to` — see `bindings/c/include/tstrans.h`.
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
        // Mutex-poisoning policy — see send_video for rationale.
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| lock_poisoned("send_klv_to"))?;
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
    /// `tst_mux_sender_send_audio` — see `bindings/c/include/tstrans.h`.
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
    ///   Whether the input was consumed depends on the failure point — see
    ///   the module-level *Input consumption and retry* section before
    ///   deciding whether to resend.
    pub fn send_audio(&self, frames: &[u8], pts: Pts90khz) -> Result<(), MuxSenderError> {
        // Mutex-poisoning policy — see send_video for rationale.
        let mut inner = self.inner.lock().map_err(|_| lock_poisoned("send_audio"))?;
        inner.send_audio(frames, pts.as_ticks())
    }

    /// Send one audio frame buffer to a specific configured audio stream.
    /// `handle` is obtained from [`Self::audio_handles`]; passing a handle
    /// from a different sender / muxer surfaces as
    /// [`MuxError::InvalidStreamHandle`] inside [`MuxSenderErrorSource::Mux`].
    ///
    /// # C ABI
    ///
    /// `tst_mux_sender_send_audio_to` — see `bindings/c/include/tstrans.h`.
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
    ///   Whether the input was consumed depends on the failure point — see
    ///   the module-level *Input consumption and retry* section before
    ///   deciding whether to resend.
    pub fn send_audio_to(
        &self,
        handle: AudioStreamHandle,
        frames: &[u8],
        pts: Pts90khz,
    ) -> Result<(), MuxSenderError> {
        // Mutex-poisoning policy — see send_video for rationale.
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| lock_poisoned("send_audio_to"))?;
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
    /// `tst_mux_sender_send_subtitle` — see `bindings/c/include/tstrans.h`.
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
    ///   Whether the input was consumed depends on the failure point — see
    ///   the module-level *Input consumption and retry* section before
    ///   deciding whether to resend.
    pub fn send_subtitle(&self, payload: &[u8], pts: Pts90khz) -> Result<(), MuxSenderError> {
        // Mutex-poisoning policy — see send_video for rationale.
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| lock_poisoned("send_subtitle"))?;
        inner.send_subtitle(payload, pts.as_ticks())
    }

    /// Send one subtitle PES unit to a specific configured subtitle stream.
    /// `handle` is obtained from [`Self::subtitle_handles`]; passing a
    /// handle from a different sender / muxer surfaces as
    /// [`MuxError::InvalidStreamHandle`] inside [`MuxSenderErrorSource::Mux`].
    ///
    /// # C ABI
    ///
    /// `tst_mux_sender_send_subtitle_to` — see `bindings/c/include/tstrans.h`.
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
    ///   Whether the input was consumed depends on the failure point — see
    ///   the module-level *Input consumption and retry* section before
    ///   deciding whether to resend.
    pub fn send_subtitle_to(
        &self,
        handle: SubtitleStreamHandle,
        payload: &[u8],
        pts: Pts90khz,
    ) -> Result<(), MuxSenderError> {
        // Mutex-poisoning policy — see send_video for rationale.
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| lock_poisoned("send_subtitle_to"))?;
        inner.send_subtitle_to(handle, payload, pts.as_ticks())
    }

    /// Send one data payload on the muxer's single data stream. `pts` is
    /// in 90 kHz units (the TS clock); written into the PES header only
    /// when the stream was configured with `carries_pts: true`, and
    /// always used for PSI/PCR pacing decisions.
    ///
    /// Data streams are a PES **pass-through** — no AU-cell wrap, no
    /// framing, no payload inspection.
    /// [`tst_core::mpegts::mux::Muxer::push_data_to`] is the contract
    /// holder; see its docs for the full pass-through guarantees and the
    /// no-PTS-stream behavior.
    ///
    /// # C ABI
    ///
    /// `tst_mux_sender_send_data` — see `bindings/c/include/tstrans.h`.
    ///
    /// # Errors
    /// - [`MuxSenderErrorSource::Mux`] wraps [`MuxError`] from the inner muxer:
    ///   [`MuxError::NoDataStreamsConfigured`] if no data streams exist;
    ///   [`MuxError::AmbiguousTarget`] when more than one is configured
    ///   (use [`Self::send_data_to`]); [`MuxError::DataTooLarge`] if the
    ///   payload would overflow `PES_packet_length`;
    ///   [`MuxError::BufferFull`] if the muxer's outbound queue is at
    ///   `MuxerConfig::buffer_packets`.
    /// - [`MuxSenderErrorSource::Transport`] wraps a [`TransportError`]; on
    ///   transport flap the unsent TS chunks are retained for a later
    ///   `send_*` call to drain.
    ///   Whether the input was consumed depends on the failure point — see
    ///   the module-level *Input consumption and retry* section before
    ///   deciding whether to resend.
    pub fn send_data(&self, data: &[u8], pts: Pts90khz) -> Result<(), MuxSenderError> {
        // Mutex-poisoning policy — see send_video for rationale.
        let mut inner = self.inner.lock().map_err(|_| lock_poisoned("send_data"))?;
        inner.send_data(data, pts.as_ticks())
    }

    /// Send one data payload to a specific configured data stream.
    /// `handle` is obtained from [`Self::data_handles`]; passing a handle
    /// from a different sender / muxer surfaces as
    /// [`MuxError::InvalidStreamHandle`] inside [`MuxSenderErrorSource::Mux`].
    ///
    /// Data streams are a PES **pass-through** — no AU-cell wrap, no
    /// framing, no payload inspection.
    /// [`tst_core::mpegts::mux::Muxer::push_data_to`] is the contract
    /// holder; see its docs for the full pass-through guarantees and the
    /// no-PTS-stream behavior.
    ///
    /// # C ABI
    ///
    /// `tst_mux_sender_send_data_to` — see `bindings/c/include/tstrans.h`.
    ///
    /// # Errors
    /// - [`MuxSenderErrorSource::Mux`] wraps [`MuxError`] from the inner muxer:
    ///   [`MuxError::InvalidStreamHandle`] if `handle`'s index is out of
    ///   range for this muxer's data streams; [`MuxError::DataTooLarge`]
    ///   if the payload would overflow `PES_packet_length`;
    ///   [`MuxError::BufferFull`] if the muxer's outbound queue is at
    ///   `MuxerConfig::buffer_packets`.
    /// - [`MuxSenderErrorSource::Transport`] wraps a [`TransportError`]; on
    ///   transport flap the unsent TS chunks are retained for a later
    ///   `send_*` call to drain.
    ///   Whether the input was consumed depends on the failure point — see
    ///   the module-level *Input consumption and retry* section before
    ///   deciding whether to resend.
    pub fn send_data_to(
        &self,
        handle: DataStreamHandle,
        data: &[u8],
        pts: Pts90khz,
    ) -> Result<(), MuxSenderError> {
        // Mutex-poisoning policy — see send_video for rationale.
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| lock_poisoned("send_data_to"))?;
        inner.send_data_to(handle, data, pts.as_ticks())
    }

    /// Snapshot all video stream handles for this sender's muxer, in
    /// declaration order. Allocates an owned Vec so callers don't need
    /// to hold the lock.
    pub fn video_handles(&self) -> Vec<VideoStreamHandle> {
        // Safe-default on poison: a poisoned inner lock returns an empty
        // handle list — matches the "no live muxer state" answer.
        // Precedent: `ManagedTransport::socket_stats` → None on poison (reconnect/mod.rs).
        if let Ok(inner) = self.inner.lock() {
            inner.muxer.video_handles()
        } else {
            Vec::new()
        }
    }

    /// Snapshot all KLV stream handles for this sender's muxer.
    pub fn klv_handles(&self) -> Vec<KlvStreamHandle> {
        // Mutex-poisoning policy (safe-default on poison) — see video_handles for rationale.
        if let Ok(inner) = self.inner.lock() {
            inner.muxer.klv_handles()
        } else {
            Vec::new()
        }
    }

    /// Snapshot all audio stream handles for this sender's muxer, in
    /// declaration order.
    pub fn audio_handles(&self) -> Vec<AudioStreamHandle> {
        // Mutex-poisoning policy (safe-default on poison) — see video_handles for rationale.
        if let Ok(inner) = self.inner.lock() {
            inner.muxer.audio_handles()
        } else {
            Vec::new()
        }
    }

    /// Audio stream handles for the named program, in declaration order.
    /// Returns `Err(MuxError::ProgramNotFound)` if no program with the
    /// given number exists in this sender's muxer configuration.
    pub fn audio_handles_for_program(
        &self,
        program_number: u16,
    ) -> Result<Vec<AudioStreamHandle>, MuxError> {
        // Mutex-poisoning policy (recoverable path / closest-semantic mapping):
        // poisoned inner lock returns ProgramNotFound since the muxer state is
        // unreachable — the closest existing semantic for "no programs available"
        // given the function's narrow MuxError surface. The alternative — a new
        // MuxError::LockPoisoned variant — was rejected due to the public-api
        // baseline bump + binding-surface ripple.
        self.inner
            .lock()
            .map_err(|_| MuxError::ProgramNotFound { program_number })?
            .muxer
            .audio_handles_for_program(program_number)
    }

    /// Snapshot all subtitle stream handles for this sender's muxer.
    pub fn subtitle_handles(&self) -> Vec<SubtitleStreamHandle> {
        // Mutex-poisoning policy (safe-default on poison) — see video_handles for rationale.
        if let Ok(inner) = self.inner.lock() {
            inner.muxer.subtitle_handles()
        } else {
            Vec::new()
        }
    }

    /// Subtitle stream handles for the named program, in declaration
    /// order. Returns `Err(MuxError::ProgramNotFound)` if no program
    /// with the given number exists in this sender's muxer
    /// configuration.
    pub fn subtitle_handles_for_program(
        &self,
        program_number: u16,
    ) -> Result<Vec<SubtitleStreamHandle>, MuxError> {
        // Mutex-poisoning policy (recoverable path / closest-semantic mapping) —
        // see audio_handles_for_program for rationale.
        self.inner
            .lock()
            .map_err(|_| MuxError::ProgramNotFound { program_number })?
            .muxer
            .subtitle_handles_for_program(program_number)
    }

    /// Snapshot all data stream handles for this sender's muxer.
    pub fn data_handles(&self) -> Vec<DataStreamHandle> {
        // Mutex-poisoning policy (safe-default on poison) — see video_handles for rationale.
        if let Ok(inner) = self.inner.lock() {
            inner.muxer.data_handles()
        } else {
            Vec::new()
        }
    }

    /// Return a point-in-time stats snapshot. `per_stream` is delegated from
    /// the inner `Muxer`; `pending_*` fields are live gauges.
    pub fn stats(&self) -> MuxSenderStats {
        // Mutex-poisoning policy (safe-default on poison): zeroed stats matches
        // "no live state available."
        let Ok(inner) = self.inner.lock() else {
            return MuxSenderStats::default();
        };
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
    /// `bindings/c/include/tstrans.h`.
    pub fn socket_stats(&self) -> Option<tst_core::transport::SocketStats> {
        // Mutex-poisoning policy (safe-default on poison): mirrors reconnect/mod.rs
        // verbatim — None on poison, indistinguishable from "no live socket."
        // C ABI surfaces this as TST_E_NOT_AVAILABLE (-13).
        self.inner
            .lock()
            .ok()
            .and_then(|i| i.transport.socket_stats())
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
    /// see `bindings/c/include/tstrans.h`.
    pub fn stream_codec_stats(
        &self,
        pid: u16,
    ) -> Option<tst_core::mpegts::stats::StreamCodecStats> {
        // Mutex-poisoning policy (safe-default on poison): None on poison —
        // same shape as socket_stats.
        self.inner
            .lock()
            .ok()
            .and_then(|i| i.muxer.stream_codec_stats(pid))
    }

    /// Zero all flow counters and delegate to `Muxer::reset_stats`.
    /// `pending_bytes_queued` / `pending_chunks_queued` are live gauges and
    /// are NOT cleared.
    pub fn reset_stats(&self) {
        // Mutex-poisoning policy (silent no-op on poison): reset_stats on a
        // poisoned state is naturally a no-op since the stats are already
        // lost. Matches close() + Drop shape verbatim.
        if let Ok(mut inner) = self.inner.lock() {
            inner.bytes_sent = 0;
            inner.packets_sent = 0;
            inner.muxer.reset_stats();
        }
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
    /// `tst_mux_sender_cancel` — see `bindings/c/include/tstrans.h`.
    pub fn cancel_handle(
        &self,
    ) -> Option<Arc<dyn tst_core::transport::TransportCancel + Send + Sync>> {
        self.cancel.clone()
    }

    #[must_use]
    pub fn is_alive(&self) -> bool {
        // Mutex-poisoning policy (safe-default on poison): poisoned state is not
        // alive. False matches the "wrapper unusable" answer.
        if let Ok(inner) = self.inner.lock() {
            !inner.closed && inner.transport.is_alive()
        } else {
            false
        }
    }
}

impl<T: Transport> Drop for MuxSender<T> {
    fn drop(&mut self) {
        let _enter = self._span.0.enter();
        // Best-effort drain of pending_bytes on drop; if transport rejects,
        // they're discarded. Gate on `!inner.closed` to mirror Sender::Drop —
        // a prior explicit close() already drained + closed, so close-then-drop
        // would otherwise call transport.close() twice. Idempotent in practice,
        // but the gate keeps the contract consistent across the three shells.
        if let Ok(mut inner) = self.inner.lock() {
            if !inner.closed {
                let _ = inner.drain_pending();
                inner.transport.close();
            }
        }
        tracing::info!("MuxSender closed");
    }
}

/// Type alias for [`MuxSender`] with a boxed [`Transport`] trait object.
///
/// Bindings code (`tst-jni`, `tst-uniffi`, `tst-pyo3`) targets this single
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
    /// Shared body for every `send_*` path: closed-check → drain pending →
    /// push (via the caller-supplied closure) → back-pressure sample →
    /// drain muxer. Called with a closure so the push arguments (handles,
    /// PTS, data slices) don't need to be marshalled into a common enum.
    fn push_then_drain(
        &mut self,
        push: impl FnOnce(&mut Muxer) -> Result<(), MuxError>,
    ) -> Result<(), MuxSenderError> {
        if self.closed {
            return Err(TransportError::Closed.into());
        }
        // Drain any leftover from a previous failed call first.
        self.drain_pending()?;
        // Push new content. Sample back-pressure between the push (queue at
        // peak) and the drain (queue back to zero).
        let push_result = push(&mut self.muxer);
        self.maybe_warn_backpressure(matches!(push_result, Err(MuxError::BufferFull { .. })));
        push_result?;
        self.drain_muxer()
    }

    fn send_video(
        &mut self,
        nal: &[u8],
        pts_90khz: i64,
        key_frame: bool,
    ) -> Result<(), MuxSenderError> {
        self.push_then_drain(|m| m.push_video(nal, Pts90khz::new(pts_90khz), key_frame))
    }

    fn send_klv(
        &mut self,
        klv: &[u8],
        pts_90khz: i64,
        metadata_service_id: u8,
    ) -> Result<(), MuxSenderError> {
        self.push_then_drain(|m| m.push_klv(klv, Pts90khz::new(pts_90khz), metadata_service_id))
    }

    fn send_video_to(
        &mut self,
        handle: VideoStreamHandle,
        nal: &[u8],
        pts_90khz: i64,
        key_frame: bool,
    ) -> Result<(), MuxSenderError> {
        self.push_then_drain(|m| m.push_video_to(handle, nal, Pts90khz::new(pts_90khz), key_frame))
    }

    fn send_video_to_with_dts(
        &mut self,
        handle: VideoStreamHandle,
        nal: &[u8],
        pts_90khz: i64,
        dts_90khz: i64,
        key_frame: bool,
    ) -> Result<(), MuxSenderError> {
        self.push_then_drain(|m| {
            m.push_video_to_with_dts(
                handle,
                nal,
                Pts90khz::new(pts_90khz),
                Pts90khz::new(dts_90khz),
                key_frame,
            )
        })
    }

    fn send_klv_to(
        &mut self,
        handle: KlvStreamHandle,
        klv: &[u8],
        pts_90khz: i64,
        metadata_service_id: u8,
    ) -> Result<(), MuxSenderError> {
        self.push_then_drain(|m| {
            m.push_klv_to(handle, klv, Pts90khz::new(pts_90khz), metadata_service_id)
        })
    }

    fn send_audio(&mut self, frames: &[u8], pts_90khz: i64) -> Result<(), MuxSenderError> {
        self.push_then_drain(|m| m.push_audio(frames, Pts90khz::new(pts_90khz)))
    }

    fn send_audio_to(
        &mut self,
        handle: AudioStreamHandle,
        frames: &[u8],
        pts_90khz: i64,
    ) -> Result<(), MuxSenderError> {
        // Muxer parameter order is `(handle, pts, frames)`; the public
        // pipeline API mirrors `send_video` / `send_klv` (data first).
        self.push_then_drain(|m| m.push_audio_to(handle, Pts90khz::new(pts_90khz), frames))
    }

    fn send_subtitle(&mut self, payload: &[u8], pts_90khz: i64) -> Result<(), MuxSenderError> {
        // Muxer parameter order is `(pts, payload)`; we present
        // `(payload, pts)` for symmetry with `send_video` / `send_klv`.
        self.push_then_drain(|m| m.push_subtitle(Pts90khz::new(pts_90khz), payload))
    }

    fn send_subtitle_to(
        &mut self,
        handle: SubtitleStreamHandle,
        payload: &[u8],
        pts_90khz: i64,
    ) -> Result<(), MuxSenderError> {
        self.push_then_drain(|m| m.push_subtitle_to(handle, Pts90khz::new(pts_90khz), payload))
    }

    fn send_data(&mut self, data: &[u8], pts_90khz: i64) -> Result<(), MuxSenderError> {
        self.push_then_drain(|m| m.push_data(data, Pts90khz::new(pts_90khz)))
    }

    fn send_data_to(
        &mut self,
        handle: DataStreamHandle,
        data: &[u8],
        pts_90khz: i64,
    ) -> Result<(), MuxSenderError> {
        self.push_then_drain(|m| m.push_data_to(handle, data, Pts90khz::new(pts_90khz)))
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
        // Grow the scratch buffer lazily. The Transport trait does not
        // guarantee a fixed max_payload() across calls, so re-check each
        // time and resize only when the current allocation is too small.
        if self.scratch.len() < max {
            self.scratch.resize(max, 0);
        }
        loop {
            // Cap the view at the CURRENT max_payload — the scratch only
            // grows, so after a max_payload shrink the full buffer would
            // let `pull` produce chunks larger than the transport accepts.
            let n = self.muxer.pull(&mut self.scratch[..max]);
            if n == 0 {
                return Ok(());
            }
            match self.transport.send_bytes(&self.scratch[..n]) {
                Ok(()) => {
                    // Happy path: no allocation needed; bytes are in flight.
                    self.bytes_sent += n as u64;
                    self.packets_sent += 1;
                }
                Err(e) => {
                    // Transport rejected the chunk — buffer it; do NOT count as sent.
                    self.pending_bytes.push_back(self.scratch[..n].to_vec());
                    // Drain any further muxer output into pending_bytes too,
                    // so the muxer's internal buffer doesn't fill up while
                    // transport is unavailable.
                    loop {
                        let n2 = self.muxer.pull(&mut self.scratch[..max]);
                        if n2 == 0 {
                            break;
                        }
                        self.pending_bytes.push_back(self.scratch[..n2].to_vec());
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

    fn errno_code(&self) -> Option<i32> {
        match &self.source {
            MuxSenderErrorSource::Transport(t) => crate::shell_error::errno_code_from_transport(t),
            MuxSenderErrorSource::Mux(_) => None,
        }
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
    fn send_data_pushes_through_pipeline() {
        // Single program, video + one data stream. The bare send_data
        // shorthand resolves because total_data == 1 across the muxer.
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x100, VideoCodec::H264);
            prog.add_data(0x1100, 0xF0, /*carries_pts=*/ true);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        // Synthetic payload bytes — data streams are a pass-through, so
        // any non-empty buffer suffices.
        let payload = vec![0x42u8; 64];
        s.send_data(&payload, Pts90khz::new(90_000)).unwrap();
        let st = s.stats();
        assert_eq!(st.per_stream[&0x1100].items, 1);
        assert_eq!(st.per_stream[&0x1100].bytes, payload.len() as u64);
        assert!(st.bytes_sent > 0);
        assert!(st.packets_sent > 0);
    }

    #[test]
    fn send_data_to_routes_by_handle() {
        // Two data streams — bare send_data would reject with
        // AmbiguousTarget; send_data_to disambiguates via handle.
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x100, VideoCodec::H264);
            prog.add_data(0x1100, 0xF0, /*carries_pts=*/ true);
            prog.add_data(0x1101, 0xF1, /*carries_pts=*/ true);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let handles = s.data_handles();
        assert_eq!(handles.len(), 2);
        let payload = vec![0x42u8; 32];
        s.send_data_to(handles[1], &payload, Pts90khz::new(90_000))
            .unwrap();
        let st = s.stats();
        assert_eq!(st.per_stream[&0x1101].items, 1);
        assert_eq!(st.per_stream[&0x1100].items, 0);
        assert!(s.is_alive());
    }

    #[test]
    fn send_data_rejects_oversized_payload() {
        // 70_000 bytes overflows PES_packet_length (ceiling 65527 with
        // PTS); the muxer's DataTooLarge must pass through the shell's
        // error wrapping intact.
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x100, VideoCodec::H264);
            prog.add_data(0x1100, 0xF0, /*carries_pts=*/ true);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().unwrap()
        };
        let s = MuxSender::new(MemTransport::new(), cfg).unwrap();
        let payload = vec![0x42u8; 70_000];
        let err = s.send_data(&payload, Pts90khz::new(0)).unwrap_err();
        match err.source {
            MuxSenderErrorSource::Mux(MuxError::DataTooLarge { size: 70_000, .. }) => {}
            other => panic!("expected DataTooLarge, got {other:?}"),
        }
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
                return Err(TransportError::Backpressure {
                    msg: "backpressure-once".to_string(),
                    errno_code: None,
                });
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
                    return Err(TransportError::Broken {
                        msg: "cancelled".into(),
                        errno_code: None,
                    });
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(TransportError::Broken {
                msg: "test timeout (cancel never fired)".into(),
                errno_code: None,
            })
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

    /// Build a `MuxerConfig` with one stream of every type under program 1.
    /// This is the canonical config shared by Tasks 2 and 3 poisoned-lock
    /// regression tests — stream handle indices are deterministic for a given
    /// config layout, so both tests can use handles snapshotted from any
    /// sender built from this config.
    fn all_streams_config() -> MuxerConfig {
        use tst_core::mpegts::mux::{AudioCodec, KlvStreamType, SubtitleCodec};
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x100, VideoCodec::H264);
        prog.add_klv(0x101, KlvStreamType::PrivateData, false);
        prog.add_audio(0x102, AudioCodec::Aac);
        prog.add_subtitle(0x103, SubtitleCodec::WebVttInTs);
        prog.add_data(0x104, 0xF0, /*carries_pts=*/ true);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().unwrap()
    }

    /// Create a `MuxSender<PanicOnSend>` whose inner mutex has been
    /// poisoned. Poison mechanism: a spawned thread calls `send_video`
    /// which reaches `transport.send_bytes`, which panics while holding
    /// the `MutexGuard`, auto-poisoning `Inner` during stack unwinding.
    ///
    /// The returned `Arc` is shared-ownership so callers can invoke
    /// methods on the already-poisoned value without taking ownership.
    fn poison_sender() -> Arc<MuxSender<PanicOnSend>> {
        let sender = Arc::new(MuxSender::new(PanicOnSend, all_streams_config()).unwrap());
        let s = sender.clone();
        let h = std::thread::spawn(move || {
            // Minimal Annex-B IDR NAL — the muxer emits TS packets into
            // drain_muxer, which calls send_bytes, which panics.
            let nal = [0x00, 0x00, 0x00, 0x01, 0x67, 0xBB];
            let _ = s.send_video(&nal, Pts90khz::new(0), true);
        });
        let _ = h.join(); // panics; inner is now poisoned
        sender
    }

    /// PIPE-02 secondary regression: explicit `close()` on a `MuxSender`
    /// whose inner mutex was poisoned by a panic-during-send must NOT
    /// itself panic — it returns silently via the `if let Ok` branch,
    /// matching `Drop`'s graceful poisoned-lock catch.
    #[test]
    fn close_does_not_panic_on_poisoned_lock() {
        // Surviving the call IS the assertion.
        poison_sender().close();
    }

    /// Regression: every fallible-return method on
    /// `MuxSender` converts a poisoned inner lock to a typed error instead
    /// of panicking. The 10 `send_*` methods must return a `MuxSenderError`
    /// whose kind is `ShellErrorKind::TransportBroken`; the 2
    /// `*_handles_for_program` methods must return `MuxError::ProgramNotFound`.
    #[test]
    fn mux_sender_inner_lock_poisoned_returns_broken_error() {
        // Snapshot handles from a fresh (unpoisoned) sender with the same
        // config — stream handles are packed indices deterministic for a given
        // config layout, so any sender built from all_streams_config() has the
        // same handles.
        let fresh = MuxSender::new(PanicOnSend, all_streams_config()).unwrap();
        let video_h = fresh.video_handles()[0];
        let klv_h = fresh.klv_handles()[0];
        let audio_h = fresh.audio_handles()[0];
        let subtitle_h = fresh.subtitle_handles()[0];
        let data_h = fresh.data_handles()[0];
        drop(fresh);

        let sender = poison_sender();

        // --- 10 send_* methods: must return TransportBroken, not panic ---

        let nal = [0x00, 0x00, 0x00, 0x01, 0x67, 0xBB];
        let pts = Pts90khz::new(0);

        let err = sender.send_video(&nal, pts, true).unwrap_err();
        assert_eq!(
            err.kind,
            ShellErrorKind::TransportBroken,
            "send_video: expected TransportBroken, got {:?}",
            err.kind
        );
        assert!(
            matches!(&err.source, MuxSenderErrorSource::Transport(TransportError::Broken { msg, .. }) if msg.contains("send_video")),
            "send_video: message should contain 'send_video', got: {err:?}"
        );

        let err = sender.send_klv(&[0xAA, 0xBB, 0xCC], pts, 0x00).unwrap_err();
        assert_eq!(err.kind, ShellErrorKind::TransportBroken, "send_klv");
        assert!(
            matches!(&err.source, MuxSenderErrorSource::Transport(TransportError::Broken { msg, .. }) if msg.contains("send_klv")),
            "send_klv: message should contain 'send_klv', got: {err:?}"
        );

        let err = sender.send_video_to(video_h, &nal, pts, true).unwrap_err();
        assert_eq!(err.kind, ShellErrorKind::TransportBroken, "send_video_to");
        assert!(
            matches!(&err.source, MuxSenderErrorSource::Transport(TransportError::Broken { msg, .. }) if msg.contains("send_video_to")),
            "send_video_to: message should contain 'send_video_to', got: {err:?}"
        );

        let err = sender
            .send_klv_to(klv_h, &[0xAA, 0xBB, 0xCC], pts, 0x00)
            .unwrap_err();
        assert_eq!(err.kind, ShellErrorKind::TransportBroken, "send_klv_to");
        assert!(
            matches!(&err.source, MuxSenderErrorSource::Transport(TransportError::Broken { msg, .. }) if msg.contains("send_klv_to")),
            "send_klv_to: message should contain 'send_klv_to', got: {err:?}"
        );

        let err = sender.send_audio(&[0xFF; 32], pts).unwrap_err();
        assert_eq!(err.kind, ShellErrorKind::TransportBroken, "send_audio");
        assert!(
            matches!(&err.source, MuxSenderErrorSource::Transport(TransportError::Broken { msg, .. }) if msg.contains("send_audio")),
            "send_audio: message should contain 'send_audio', got: {err:?}"
        );

        let err = sender.send_audio_to(audio_h, &[0xFF; 32], pts).unwrap_err();
        assert_eq!(err.kind, ShellErrorKind::TransportBroken, "send_audio_to");
        assert!(
            matches!(&err.source, MuxSenderErrorSource::Transport(TransportError::Broken { msg, .. }) if msg.contains("send_audio_to")),
            "send_audio_to: message should contain 'send_audio_to', got: {err:?}"
        );

        let err = sender.send_subtitle(b"WEBVTT cue", pts).unwrap_err();
        assert_eq!(err.kind, ShellErrorKind::TransportBroken, "send_subtitle");
        assert!(
            matches!(&err.source, MuxSenderErrorSource::Transport(TransportError::Broken { msg, .. }) if msg.contains("send_subtitle")),
            "send_subtitle: message should contain 'send_subtitle', got: {err:?}"
        );

        let err = sender
            .send_subtitle_to(subtitle_h, b"WEBVTT cue", pts)
            .unwrap_err();
        assert_eq!(
            err.kind,
            ShellErrorKind::TransportBroken,
            "send_subtitle_to"
        );
        assert!(
            matches!(&err.source, MuxSenderErrorSource::Transport(TransportError::Broken { msg, .. }) if msg.contains("send_subtitle_to")),
            "send_subtitle_to: message should contain 'send_subtitle_to', got: {err:?}"
        );

        let err = sender.send_data(&[0x42; 32], pts).unwrap_err();
        assert_eq!(err.kind, ShellErrorKind::TransportBroken, "send_data");
        assert!(
            matches!(&err.source, MuxSenderErrorSource::Transport(TransportError::Broken { msg, .. }) if msg.contains("send_data")),
            "send_data: message should contain 'send_data', got: {err:?}"
        );

        let err = sender.send_data_to(data_h, &[0x42; 32], pts).unwrap_err();
        assert_eq!(err.kind, ShellErrorKind::TransportBroken, "send_data_to");
        assert!(
            matches!(&err.source, MuxSenderErrorSource::Transport(TransportError::Broken { msg, .. }) if msg.contains("send_data_to")),
            "send_data_to: message should contain 'send_data_to', got: {err:?}"
        );

        // --- 2 *_handles_for_program methods: must return ProgramNotFound ---

        let err = sender.audio_handles_for_program(1).unwrap_err();
        assert!(
            matches!(err, MuxError::ProgramNotFound { program_number: 1 }),
            "audio_handles_for_program: expected ProgramNotFound{{1}}, got: {err:?}"
        );

        let err = sender.subtitle_handles_for_program(1).unwrap_err();
        assert!(
            matches!(err, MuxError::ProgramNotFound { program_number: 1 }),
            "subtitle_handles_for_program: expected ProgramNotFound{{1}}, got: {err:?}"
        );
    }

    /// Regression: every infallible-return method on a `MuxSender` with a
    /// poisoned inner mutex returns a safe default instead of panicking.
    /// Safe defaults match the "no live muxer state" answer.
    ///
    /// Uses the same `poison_sender()` helper as the recoverable-path test —
    /// same poison mechanism, same config layout.
    #[test]
    fn mux_sender_inner_lock_poisoned_returns_safe_default() {
        let sender = poison_sender();

        // *_handles → empty Vec
        assert!(
            sender.video_handles().is_empty(),
            "video_handles: expected empty vec on poisoned lock"
        );
        assert!(
            sender.klv_handles().is_empty(),
            "klv_handles: expected empty vec on poisoned lock"
        );
        assert!(
            sender.audio_handles().is_empty(),
            "audio_handles: expected empty vec on poisoned lock"
        );
        assert!(
            sender.subtitle_handles().is_empty(),
            "subtitle_handles: expected empty vec on poisoned lock"
        );
        assert!(
            sender.data_handles().is_empty(),
            "data_handles: expected empty vec on poisoned lock"
        );

        // stats → MuxSenderStats::default() (zeroed)
        let s = sender.stats();
        assert_eq!(
            s.bytes_sent, 0,
            "stats.bytes_sent should be 0 on poisoned lock"
        );
        assert_eq!(
            s.packets_sent, 0,
            "stats.packets_sent should be 0 on poisoned lock"
        );

        // socket_stats → None
        assert!(
            sender.socket_stats().is_none(),
            "socket_stats: expected None on poisoned lock"
        );

        // stream_codec_stats → None (any PID)
        assert!(
            sender.stream_codec_stats(0x100).is_none(),
            "stream_codec_stats: expected None on poisoned lock"
        );

        // is_alive → false
        assert!(
            !sender.is_alive(),
            "is_alive: expected false on poisoned lock"
        );

        // reset_stats → must not panic (call and proceed)
        sender.reset_stats();
    }
}
