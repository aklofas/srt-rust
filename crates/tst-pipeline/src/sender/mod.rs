//! `Sender<T: Transport>` — pre-muxed TS bytes → SRT, with framing.
//!
//! See `framing.rs` for the sync-acquisition / loss-detection state
//! machine. `Sender` composes `TsFraming` with a `Transport`.

mod framing;

pub use framing::{SenderStats, TsFraming, TsFramingError, TsFramingMode};

use std::sync::Arc;
use tracing::{Span, info_span};
use tst_core::transport::Transport;

/// Construction-time knobs for [`Sender`].
#[must_use]
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct SenderConfig {
    pub framing_mode: TsFramingMode,
    /// Threshold (bytes consumed while UNSYNCED) above which RECOVER mode
    /// flags that sync has not been acquired. Default 18,800 (≈100
    /// packets' worth).
    ///
    /// This is a diagnostic-only threshold in the current implementation:
    /// the sender does NOT stop or fail when it is exceeded — RECOVER
    /// mode keeps scanning for a sync byte indefinitely. Callers who
    /// want fail-fast on persistent no-sync should monitor
    /// [`SenderStats::bytes_skipped_for_sync`] against their own
    /// threshold and abort externally.
    ///
    /// [`TsFramingError::NoSyncAfterLimit`] is part of the public error
    /// type for forward compatibility but is not currently emitted by
    /// the sender.
    pub max_unsynced_bytes: usize,
}

impl Default for SenderConfig {
    fn default() -> Self {
        Self {
            framing_mode: TsFramingMode::Recover,
            max_unsynced_bytes: 18_800,
        }
    }
}

use crate::shell_error::ShellErrorKind;

/// Error returned by [`Sender`] methods.
///
/// # Categorization
///
/// Bindings categorize failures via [`Self::kind`] (one of 6
/// [`ShellErrorKind`] variants); power users inspect [`Self::source`]
/// for the typed inner error.
///
/// # Reachable kinds
///
/// `Sender` can produce: `InputMalformed` (STRICT-mode framing failure),
/// `Backpressure`, `TransportBroken`, `Closed`. `ConfigInvalid` and
/// `EndOfStream` are unreachable (no muxer, sender-only).
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
#[error("Sender error ({kind:?}): {source}")]
pub struct SenderError {
    pub kind: ShellErrorKind,
    #[source]
    pub source: SenderErrorSource,
}

#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum SenderErrorSource {
    #[error(transparent)]
    Framing(#[from] TsFramingError),
    #[error(transparent)]
    Transport(#[from] tst_core::transport::TransportError),
}

impl From<TsFramingError> for SenderError {
    fn from(e: TsFramingError) -> Self {
        Self {
            kind: crate::shell_error::kind_from_framing(&e),
            source: SenderErrorSource::Framing(e),
        }
    }
}

impl From<tst_core::transport::TransportError> for SenderError {
    fn from(e: tst_core::transport::TransportError) -> Self {
        Self {
            kind: crate::shell_error::kind_from_transport(&e, crate::shell_error::Direction::Send),
            source: SenderErrorSource::Transport(e),
        }
    }
}

impl crate::shell_error::ShellError for SenderError {
    fn kind(&self) -> ShellErrorKind {
        self.kind
    }

    fn errno_code(&self) -> Option<i32> {
        match &self.source {
            SenderErrorSource::Transport(t) => crate::shell_error::errno_code_from_transport(t),
            SenderErrorSource::Framing(_) => None,
        }
    }
}

/// Pre-muxed TS bytes → SRT transport with sync framing/recovery.
///
/// # Closing
///
/// `Sender` supports three shutdown patterns:
///
/// 1. **Drop** — the [`Drop`] impl best-effort flushes any buffered
///    partial bundle and closes the underlying transport. Synchronous;
///    bounded by `SRTO_LINGER` (libsrt default 30 s, configurable via
///    `SocketBuilder::linger`).
/// 2. **Explicit close** — call [`Self::close`]. Best-effort flushes
///    any buffered partial bundle (same as Drop), marks the sender
///    closed so subsequent `send_ts` / `flush` calls return
///    [`SenderErrorSource::Transport`]`(`[`tst_core::transport::TransportError::Closed`]`)`,
///    then closes the transport. Idempotent. Equivalent to Drop —
///    `AutoCloseable`/`__exit__`/`.use { }` bindings that call
///    `close()` in their cleanup path do not lose any buffered bytes.
/// 3. **Cross-thread cancel** — call [`Self::cancel_handle`] to obtain a
///    `Send + Sync` [`tst_core::transport::TransportCancel`] handle,
///    then `cancel()` from any thread. Wakes a peer thread parked in
///    `send_ts` within one libsrt I/O cycle (~3-10 ms).
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
/// | C | `tst_sender_close(sender)` (explicit; mirrors `Drop`) |
///
/// See [`docs/reference/srt-cancel-handle.md`](https://github.com/aklofas/ts-transformer/blob/main/ts-transformer/docs/reference/srt-cancel-handle.md) for the full cancel-handle pattern.
pub struct Sender<T: Transport> {
    framing: TsFraming,
    transport: T,
    closed: bool,
    mode: TsFramingMode,
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

impl<T: Transport> std::fmt::Debug for Sender<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sender")
            .field("closed", &self.closed)
            .field("mode", &self.mode)
            .field("transport_kind", &std::any::type_name::<T>())
            .finish()
    }
}

impl<T: Transport> Sender<T> {
    pub fn new(transport: T, config: SenderConfig) -> Self {
        let span = info_span!(
            target: "tst_pipeline::sender",
            "sender",
            transport_kind = std::any::type_name::<T>(),
        );
        let _enter = span.enter();
        tracing::info!("Sender opened");
        drop(_enter);
        Self {
            framing: TsFraming::new(config.max_unsynced_bytes),
            transport,
            closed: false,
            mode: config.framing_mode,
            _span: std::panic::AssertUnwindSafe(span),
        }
    }

    /// Push pre-muxed TS bytes. RECOVER mode silently skips/recovers; in
    /// STRICT mode returns an error on misalignment.
    ///
    /// `bytes` need not be a whole number of 188-byte packets — partial
    /// packets are buffered until the next call (or [`Self::flush`]).
    /// Each emitted bundle to the underlying transport is sized to fit
    /// `transport.max_payload()`.
    ///
    /// # C ABI
    ///
    /// `tst_sender_send_ts` — see `crates/tst-c/include/tstrans.h`.
    ///
    /// # Errors
    /// - [`SenderErrorSource::Framing`] in STRICT mode when the input fails
    ///   to align on a TS sync byte (`0x47`).
    /// - [`SenderErrorSource::Transport`] when the underlying [`Transport`]
    ///   returns an error (e.g. `Closed`, `Broken`).
    ///
    /// # Example
    /// ```
    /// use tst_pipeline::{Sender, SenderConfig, TsFramingMode};
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
    /// // STRICT mode rejects any input that doesn't start with sync 0x47;
    /// // RECOVER (the default) silently resyncs.
    /// let mut cfg = SenderConfig::default();
    /// cfg.framing_mode = TsFramingMode::Strict;
    /// let mut sender = Sender::new(Sink(Vec::new()), cfg);
    ///
    /// // Three whole TS packets — STRICT-mode sync-verify needs 0x47 at
    /// // offsets 0, 188, and 376 before declaring the stream synced, so
    /// // a 2-packet input would buffer without entering SYNCED.
    /// let mut bytes = Vec::new();
    /// for _ in 0..3 {
    ///     bytes.push(0x47);
    ///     bytes.extend(vec![0u8; 187]);
    /// }
    /// sender.send_ts(&bytes)?;
    /// // 3 < 7 (one bundle), so no transport send yet — flush emits the
    /// // partial bundle of 3 packets.
    /// sender.flush()?;
    /// assert_eq!(sender.stats().packets_sent, 3);
    /// # Ok(())
    /// # }
    /// ```
    pub fn send_ts(&mut self, bytes: &[u8]) -> Result<(), SenderError> {
        if self.closed {
            return Err(tst_core::transport::TransportError::Closed.into());
        }
        let bundles = if self.mode == TsFramingMode::Recover {
            let (bundles, _stats) = self.framing.push(bytes);
            bundles
        } else {
            self.framing.push_strict(bytes)?
        };
        for bundle in bundles {
            self.transport.send_bytes(&bundle)?;
        }
        Ok(())
    }

    /// Emit any buffered partial bundle.
    ///
    /// # C ABI
    ///
    /// `tst_sender_flush` — see `crates/tst-c/include/tstrans.h`.
    ///
    /// # Errors
    /// Returns [`SenderErrorSource::Transport`] when the underlying [`Transport`]
    /// rejects the flushed bundle (typically `Closed` after a prior
    /// [`Self::close`], or `Broken` on transport flap).
    pub fn flush(&mut self) -> Result<(), SenderError> {
        if self.closed {
            return Err(tst_core::transport::TransportError::Closed.into());
        }
        let bundles = self.framing.flush();
        for bundle in bundles {
            self.transport.send_bytes(&bundle)?;
        }
        Ok(())
    }

    pub fn stats(&self) -> SenderStats {
        self.framing.stats()
    }

    /// Zero all stats counters. The framing state machine is untouched —
    /// only the counters on top of it.
    pub fn reset_stats(&mut self) {
        self.framing.reset_stats();
    }

    /// Wire-level transport stats (RTT, packet loss, bandwidth, queue
    /// depths) sourced from the underlying [`Transport::socket_stats`]
    /// implementation. Returns `None` when the transport doesn't expose
    /// comparable telemetry (test mocks) or when a managed wrapper has
    /// no live inner socket.
    ///
    /// # C ABI
    ///
    /// `tst_sender_get_socket_stats` — see
    /// `crates/tst-c/include/tstrans.h`.
    pub fn socket_stats(&self) -> Option<tst_core::transport::SocketStats> {
        self.transport.socket_stats()
    }

    pub fn close(&mut self) {
        // Best-effort flush of any buffered partial bundle BEFORE marking
        // closed (otherwise the subsequent flush() would early-return on
        // the `self.closed` guard). Mirrors Drop semantics so explicit
        // close == drop for AutoCloseable / __exit__ / .use { } /
        // tst_sender_close(...) idioms.
        let _ = self.flush();
        self.closed = true;
        self.transport.close();
    }

    #[must_use]
    pub fn is_alive(&self) -> bool {
        !self.closed && self.transport.is_alive()
    }

    /// Snapshot of the underlying transport's cancel handle. See
    /// [`crate::MuxSender::cancel_handle`] for the rationale.
    ///
    /// # C ABI
    ///
    /// `tst_sender_cancel` — see `crates/tst-c/include/tstrans.h`.
    pub fn cancel_handle(
        &self,
    ) -> Option<Arc<dyn tst_core::transport::TransportCancel + Send + Sync>> {
        self.transport.cancel_handle()
    }

    /// Borrow the underlying transport. Bindings use this to reach
    /// transport-specific telemetry not surfaced through the abstract
    /// [`Transport::socket_stats`] (e.g. SRT's 17-field `Stats`).
    ///
    /// Returns a shared reference; mutation is intentionally not
    /// exposed — the shell owns the transport's send-side lifecycle.
    pub fn transport(&self) -> &T {
        &self.transport
    }
}

/// Type alias for [`Sender`] with a boxed [`Transport`] trait object.
///
/// See [`BoxedMuxSender`](crate::mux_sender::BoxedMuxSender) for rationale and the per-binding pattern.
///
/// # Example — opaque sender from a runtime-chosen transport
/// ```no_run
/// use tst_pipeline::sender::BoxedSender;
/// use tst_pipeline::Sender;
/// use tst_core::Transport;
///
/// fn open(transport: Box<dyn Transport>) -> BoxedSender {
///     Sender::new(transport, Default::default())
/// }
/// ```
pub type BoxedSender = Sender<Box<dyn crate::Transport>>;

/// Drop flushes any buffered partial bundle (best-effort), then closes
/// the transport. Equivalent to calling [`Sender::close`] explicitly —
/// FFI bindings may use either idiom.
impl<T: Transport> Drop for Sender<T> {
    fn drop(&mut self) {
        if !self.closed {
            // Best-effort flush; ignore errors.
            let _ = self.flush();
            self.transport.close();
        }
        let _enter = self._span.0.enter();
        tracing::info!("Sender closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tst_core::transport::{Transport, TransportError};

    struct Mem;
    impl Transport for Mem {
        fn send_bytes(&mut self, _: &[u8]) -> Result<(), TransportError> {
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
    fn reset_stats_zeros_counters_in_ts_sender() {
        let mut s = Sender::new(Mem, SenderConfig::default());
        // One 188-byte TS packet starting with the sync byte.
        let mut pkt = vec![0x47u8];
        pkt.extend(vec![0u8; 187]);
        s.send_ts(&pkt).unwrap();
        assert!(s.stats().bytes_pushed > 0);
        s.reset_stats();
        let st = s.stats();
        assert_eq!(st.bytes_pushed, 0);
        assert_eq!(st.bytes_skipped_for_sync, 0);
        assert_eq!(st.resync_events, 0);
        assert_eq!(st.packets_sent, 0);
    }

    /// Recording transport that captures every byte sent. Used to assert
    /// what was emitted across the lifecycle boundary.
    struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl Transport for Recorder {
        fn send_bytes(&mut self, b: &[u8]) -> Result<(), TransportError> {
            self.0.lock().unwrap().extend_from_slice(b);
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
    fn close_flushes_buffered_partial_packets() {
        // Reproduces PIPE-01: Sender::close must flush the framing buffer
        // before marking the sender closed; otherwise 1-6 partial TS packets
        // that fit inside TsFraming::buffer are silently dropped.
        //
        // Setup: push 3 whole TS packets (564 bytes) which is less than one
        // bundle (7 packets = 1316 bytes), so they remain buffered inside
        // TsFraming until flush or close.
        let bytes_sink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorder = Recorder(bytes_sink.clone());
        let mut sender = Sender::new(recorder, SenderConfig::default());

        let mut input = Vec::new();
        for _ in 0..3 {
            input.push(0x47);
            input.extend(vec![0u8; 187]);
        }
        sender.send_ts(&input).unwrap();
        // Nothing flushed yet (3 < 7-packet bundle).
        assert_eq!(bytes_sink.lock().unwrap().len(), 0);

        sender.close();

        // Pre-fix: 0 bytes captured (partial bundle dropped).
        // Post-fix: 564 bytes captured (flush ran on close).
        assert_eq!(
            bytes_sink.lock().unwrap().len(),
            3 * 188,
            "Sender::close must flush buffered partial TS packets (parity with Drop)"
        );
    }
}
