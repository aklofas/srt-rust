//! `DemuxReceiver<R>` — full receive: RecvTransport → Receiver → Demuxer.
//!
//! Mirrors the `MuxSender` shape on the receive side. Includes
//! `add_byte_sink` for fan-out: callbacks see every 188-byte TS packet
//! pulled from the transport, in registration order, before the demuxer
//! parses them. Useful for "write to disk + forward via RTP + demux for
//! KLV" workflows where multiple consumers tee off the same byte stream.
//!
//! # Byte-sink contract
//!
//! - Sinks are called once per TS packet (188 bytes), in the order they were
//!   registered via [`DemuxReceiver::add_byte_sink`].
//! - The slice passed to the sink is valid only for the duration of the call.
//!   Copy bytes into an owned buffer if they need to outlive the callback.
//! - Sinks must not panic. A panicking sink will unwind through `recv_event`.
//!
//! # Stream-end flush
//!
//! When the transport closes (`TransportError::Closed`), `DemuxReceiver` calls
//! `Demuxer::flush` before returning `Ok(None)`. This surfaces any partial
//! PES sitting in reassembly state (typically the final video AU, whose PES
//! packet length is 0 — length-unknown — and is only flushed when the next
//! PES starts or the stream ends). In normal live streams the flush emits
//! nothing; for finite test data it recovers the last sample.

use crate::receiver::Receiver;
use std::sync::Arc;
use tracing::{Span, info_span};
use tst_core::error::DemuxError;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, DemuxerOptions};
use tst_core::transport::RecvTransport;
use tst_core::transport::TransportError;

/// Type alias for a boxed byte-fanout callback registered via
/// [`DemuxReceiver::add_byte_sink`]. The callback receives one TS packet (188
/// bytes) per call.
pub type ByteSink = Box<dyn FnMut(&[u8]) + Send>;

/// Full receive shell: `RecvTransport → Receiver → Demuxer`, with optional
/// byte-sink fan-out.
///
/// `R` is any [`RecvTransport`] — typically an SRT-specific transport for live
/// connections, or a test mock (e.g. `CannedTransport` in the integration
/// tests).
///
/// # Usage
///
/// ```ignore
/// let mut rx = DemuxReceiver::new(transport);
/// rx.add_byte_sink(Box::new(|pkt| { /* write pkt to disk, etc. */ }));
/// for result in &mut rx {
///     match result.unwrap() {
///         DemuxEvent::ProgramMap(pmt) => { /* inspect PMT */ }
///         DemuxEvent::Sample { stream, payload, .. } => { /* forward AU */ }
///         _ => {}
///     }
/// }
/// ```
///
/// # Closing
///
/// `DemuxReceiver` supports three shutdown patterns:
///
/// 1. **Drop** — the [`Drop`] impl emits a tracing event and lets the
///    inner [`Receiver`] / transport `Drop` chain run, which closes the
///    libsrt socket. Synchronous; bounded by `SRTO_LINGER` (libsrt
///    default 30 s, configurable via `SocketBuilder::linger`).
/// 2. **Explicit close** — call [`Self::close`]. Closes the underlying
///    recv transport; the next `recv_event` flushes the demuxer and
///    returns `Ok(None)` once the queue drains. Idempotent.
/// 3. **Cross-thread cancel** — call [`Self::cancel_handle`] to obtain a
///    `Send + Sync` [`tst_core::transport::TransportCancel`] handle,
///    then `cancel()` from any thread. Wakes a peer thread parked in
///    `recv_event`'s underlying `next_packet` call within one libsrt
///    I/O cycle (~3-10 ms).
///
/// C ABI for the receiver surface (including `tst_demux_receiver_close`)
/// is on the P0 backlog and not yet shipped.
///
/// ## Per-language idiom
///
/// | Language | Idiom |
/// |----------|-------|
/// | Rust | `let _ = rx;` (Drop) or `rx.cancel_handle().map(\|c\| c.cancel());` (cross-thread) |
/// | Java | Wrap as `AutoCloseable`; `try-with-resources` calls `close()` on exit |
/// | Kotlin | Wrap as `AutoCloseable`; `.use { }` calls `close()` on exit |
/// | Swift | `deinit` calls drop; `defer { handle.cancel() }` for explicit cross-thread |
/// | Python | Wrap as `__enter__`/`__exit__`; `with ... as rx:` calls `close()` on exit |
/// | C | (deferred to per-binding plan — receiver-surface C ABI is P0) |
///
/// See [`docs/cancel-handle.md`](https://github.com/aklofas/ts-transformer/blob/main/ts-transformer/docs/cancel-handle.md) for the full cancel-handle pattern.
pub struct DemuxReceiver<R: RecvTransport> {
    ts: Receiver<R>,
    demux: Demuxer,
    byte_sinks: Vec<ByteSink>,
    /// Lifetime [`tracing::Span`] opened in [`Self::new`] /
    /// [`Self::with_demux_options`] and entered from [`Drop`] to
    /// bracket open/close events. Private — must NOT be exposed
    /// publicly (see CI public-API ratchet).
    ///
    /// Wrapped in [`std::panic::AssertUnwindSafe`] because `Span`
    /// internally holds a `Mutex` which would otherwise flip this shell
    /// from `UnwindSafe`/`RefUnwindSafe` to `!UnwindSafe`/`!RefUnwindSafe`.
    /// `Span` is only entered in `new()` and `Drop`, never on hot paths,
    /// so asserting unwind safety is correct here.
    _span: std::panic::AssertUnwindSafe<Span>,
}

impl<R: RecvTransport> DemuxReceiver<R> {
    /// Wrap a transport with default demuxer options (lenient mode).
    pub fn new(transport: R) -> Self {
        let span = info_span!(
            target: "tst_pipeline::demux_receiver",
            "demux_receiver",
            transport_kind = std::any::type_name::<R>(),
        );
        let _enter = span.enter();
        tracing::info!("DemuxReceiver opened");
        drop(_enter);
        Self {
            ts: Receiver::new(transport),
            demux: Demuxer::new(),
            byte_sinks: Vec::new(),
            _span: std::panic::AssertUnwindSafe(span),
        }
    }

    /// Wrap a transport with custom demuxer options (e.g. strict mode).
    pub fn with_demux_options(transport: R, options: DemuxerOptions) -> Self {
        let span = info_span!(
            target: "tst_pipeline::demux_receiver",
            "demux_receiver",
            transport_kind = std::any::type_name::<R>(),
        );
        let _enter = span.enter();
        tracing::info!("DemuxReceiver opened");
        drop(_enter);
        Self {
            ts: Receiver::new(transport),
            demux: Demuxer::with_options(options),
            byte_sinks: Vec::new(),
            _span: std::panic::AssertUnwindSafe(span),
        }
    }

    /// Register a byte-fanout sink.
    ///
    /// The sink is called once per 188-byte TS packet, in registration order,
    /// before the demuxer processes the packet. Multiple sinks can be
    /// registered; each sees the same bytes.
    pub fn add_byte_sink(&mut self, sink: ByteSink) {
        self.byte_sinks.push(sink);
    }

    /// Pull one [`DemuxEvent`].
    ///
    /// Blocks until either:
    /// - An event is available in the demuxer's internal queue → `Ok(Some(e))`.
    /// - The transport closes cleanly → flushes the demuxer and returns
    ///   `Ok(None)` once the queue is drained.
    /// - The transport fails → `Err(DemuxReceiverError::Transport(e))`.
    /// - The demuxer rejects a packet in strict mode → `Err(DemuxReceiverError::Demux(e))`.
    ///
    /// # MalformedPes note
    ///
    /// `DemuxError::MalformedPes` propagates as `DemuxReceiverError::Demux` and
    /// terminates the receive loop. This matches the plan default (fatal
    /// propagation). If a production caller wants to skip bad PES and continue,
    /// it can match on `DemuxReceiverError::Demux(DemuxError::MalformedPes { .. })`
    /// and call `recv_event` again; but the demuxer state after a malformed PES
    /// is undefined, so re-entry is discouraged without a design change. Tracked
    /// in the deferred-features list.
    ///
    /// # Errors
    /// - [`DemuxReceiverError::Transport`] wraps any
    ///   [`TransportError`] other than `Closed` (which is the clean-EOF
    ///   signal converted to `Ok(None)`).
    /// - [`DemuxReceiverError::Demux`] wraps a [`DemuxError`] from the
    ///   inner demuxer: strict-mode violation, unrecoverable packet
    ///   malformation, or malformed PES header.
    ///
    /// # Example
    /// ```
    /// use std::collections::VecDeque;
    /// use tst_pipeline::DemuxReceiver;
    /// use tst_core::transport::{RecvTransport, TransportError};
    ///
    /// // In-memory source; real callers plug in `tst_srt::SrtTransport`.
    /// // Returns Closed once the queue is drained — that's the EOF signal
    /// // DemuxReceiver converts to `Ok(None)` after flushing the demuxer.
    /// struct Source(VecDeque<Vec<u8>>);
    /// impl RecvTransport for Source {
    ///     fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
    ///         match self.0.pop_front() {
    ///             Some(v) => {
    ///                 let n = v.len().min(buf.len());
    ///                 buf[..n].copy_from_slice(&v[..n]);
    ///                 Ok(n)
    ///             }
    ///             None => Err(TransportError::Closed),
    ///         }
    ///     }
    ///     fn max_payload(&self) -> usize { 1316 }
    ///     fn is_alive(&self) -> bool { !self.0.is_empty() }
    /// }
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// // Empty source → first `recv_event` flushes (no partial PES) and
    /// // returns `Ok(None)`. The Iterator impl makes this idiomatic:
    /// // `for ev in &mut rx { match ev? { ... } }` exits on EOF.
    /// let mut rx = DemuxReceiver::new(Source(VecDeque::new()));
    /// assert!(rx.recv_event()?.is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn recv_event(&mut self) -> Result<Option<DemuxEvent>, DemuxReceiverError> {
        loop {
            // Fast path: demuxer already has a queued event.
            if let Some(e) = self.demux.next_event() {
                return Ok(Some(e));
            }
            // Pull the next aligned 188-byte TS packet.
            let pkt = match self.ts.next_packet() {
                Ok(p) => p,
                Err(TransportError::Closed) => {
                    // Stream end: flush any partial PES sitting in reassembly
                    // (e.g. the final video AU whose PES length field is 0).
                    self.demux.flush();
                    // Drain any events the flush produced before signaling EOF.
                    if let Some(e) = self.demux.next_event() {
                        return Ok(Some(e));
                    }
                    return Ok(None);
                }
                Err(other) => return Err(DemuxReceiverError::Transport(other)),
            };
            // Fan-out to byte sinks in registration order before demuxing.
            for sink in &mut self.byte_sinks {
                sink(&pkt);
            }
            // Feed to demuxer via the aligned fast path — the Receiver
            // transport layer already produces [u8; 188] packets so no sync
            // buffering or 0x47 hunt is needed.  In lenient mode this only
            // errors on Unrecoverable (caller violated alignment contract) or
            // MalformedPes/MalformedPsi. In strict mode it can also return
            // StrictRejection.
            self.demux
                .feed_aligned(&pkt)
                .map_err(DemuxReceiverError::Demux)?;
        }
    }

    /// Advisory liveness check. Delegates to the underlying transport.
    pub fn is_alive(&self) -> bool {
        self.ts.is_alive()
    }

    /// Close the underlying transport. Idempotent.
    ///
    /// After close, the next `recv_event` call will flush the demuxer and
    /// return `Ok(None)` once the event queue is drained.
    pub fn close(&mut self) {
        self.ts.close();
    }

    /// Snapshot of the underlying recv-transport's cancel handle. Wakes
    /// a thread parked in `recv_event()`'s `next_packet()` call.
    pub fn cancel_handle(
        &self,
    ) -> Option<Arc<dyn tst_core::transport::TransportCancel + Send + Sync>> {
        self.ts.cancel_handle()
    }
}

/// `DemuxReceiver` implements `Iterator` so callers can use `for result in &mut rx`
/// or `.collect()` patterns. EOF (`Ok(None)`) terminates the iterator.
/// Errors are surfaced as `Some(Err(e))` so the caller can distinguish a
/// transport error from a clean end of stream.
impl<R: RecvTransport> Iterator for DemuxReceiver<R> {
    type Item = Result<DemuxEvent, DemuxReceiverError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.recv_event() {
            Ok(Some(e)) => Some(Ok(e)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

impl<R: RecvTransport> Drop for DemuxReceiver<R> {
    fn drop(&mut self) {
        let _enter = self._span.0.enter();
        tracing::info!("DemuxReceiver closed");
    }
}

/// Type alias for [`DemuxReceiver`] with a boxed [`RecvTransport`] trait object.
///
/// See [`BoxedMuxSender`](crate::mux_sender::BoxedMuxSender) for rationale.
///
/// # Example
/// ```no_run
/// use tst_pipeline::demux_receiver::BoxedDemuxReceiver;
/// use tst_pipeline::DemuxReceiver;
/// use tst_core::RecvTransport;
///
/// fn open(transport: Box<dyn RecvTransport>) -> BoxedDemuxReceiver {
///     DemuxReceiver::new(transport)
/// }
/// ```
pub type BoxedDemuxReceiver = DemuxReceiver<Box<dyn crate::RecvTransport>>;

/// Errors that can be returned by [`DemuxReceiver::recv_event`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DemuxReceiverError {
    /// The underlying transport closed unexpectedly or returned a fatal error.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// The demuxer rejected a packet (strict-mode violation, unrecoverable
    /// packet malformation, or malformed PES header).
    ///
    /// Re-entry into [`DemuxReceiver::recv_event`] after this variant is
    /// discouraged for `DemuxError::MalformedPes`: the demuxer's reassembly
    /// state is undefined past a bad PES header, so subsequent events may
    /// be inconsistent. Treat it as a stream-fatal signal until the demuxer
    /// gains lenient PES recovery.
    #[error(transparent)]
    Demux(#[from] DemuxError),
}

/// Stats snapshot for [`DemuxReceiver`]. Composes the underlying
/// [`crate::receiver::ReceiverStats`] (bytes/packets received, sync-recovery
/// counters) with the [`tst_core::mpegts::demux::DemuxerStats`] (events emitted,
/// per-PID counters). Sync-recovery counters (`bytes_skipped_for_sync`,
/// `resync_events`) live only on `ReceiverStats` — call
/// `Receiver::stats()` directly to read them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DemuxReceiverStats {
    pub bytes_received: u64,
    pub packets_received: u64,
    pub program_maps_seen: u64,
    pub pmt_versions_seen: u64,
    pub discontinuities: u64,
    pub nonconformant: u64,
    pub per_stream: std::collections::BTreeMap<u16, tst_core::mpegts::stats::StreamStats>,
}

impl<R: RecvTransport> DemuxReceiver<R> {
    /// Snapshot the current counters. Composes transport-layer byte/packet
    /// counts from the inner `Receiver` with demux-layer event counts from
    /// the inner `Demuxer`.
    pub fn stats(&self) -> DemuxReceiverStats {
        let ts = self.ts.stats();
        let dx = self.demux.stats();
        DemuxReceiverStats {
            bytes_received: ts.bytes_received,
            packets_received: ts.packets_received,
            program_maps_seen: dx.program_maps_seen,
            pmt_versions_seen: dx.pmt_versions_seen,
            discontinuities: dx.discontinuities,
            nonconformant: dx.nonconformant,
            per_stream: dx.per_stream,
        }
    }

    /// Reset all counters to zero. Delegates to both the inner `Receiver`
    /// and the inner `Demuxer`.
    /// Wire-level transport stats (RTT, packet loss, bandwidth, queue
    /// depths) sourced from the underlying
    /// [`RecvTransport::socket_stats`] implementation. Delegates to the
    /// inner `Receiver`, which holds the transport. Returns `None` when
    /// the transport doesn't expose comparable telemetry or when a
    /// managed wrapper has no live inner socket.
    ///
    /// # C ABI
    ///
    /// `tst_demux_receiver_get_socket_stats` — see
    /// `crates/tst-c/include/tstrans.h`.
    pub fn socket_stats(&self) -> Option<tst_core::transport::SocketStats> {
        self.ts.socket_stats()
    }

    pub fn reset_stats(&mut self) {
        self.ts.reset_stats();
        self.demux.reset_stats();
    }
}

#[cfg(test)]
mod stats_tests {
    use super::*;
    use std::collections::VecDeque;
    use tst_core::transport::RecvTransport;
    use tst_core::transport::TransportError;

    struct CannedRecv {
        chunks: VecDeque<Vec<u8>>,
        alive: bool,
    }

    impl RecvTransport for CannedRecv {
        fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
            match self.chunks.pop_front() {
                Some(v) => {
                    let n = v.len().min(buf.len());
                    buf[..n].copy_from_slice(&v[..n]);
                    Ok(n)
                }
                None => Err(TransportError::Closed),
            }
        }

        fn max_payload(&self) -> usize {
            1316
        }

        fn is_alive(&self) -> bool {
            self.alive
        }

        fn close(&mut self) {
            self.alive = false;
        }
    }

    #[test]
    fn stats_starts_zero_with_empty_per_stream() {
        let r = DemuxReceiver::new(CannedRecv {
            chunks: VecDeque::new(),
            alive: true,
        });
        let st = r.stats();
        assert_eq!(st.bytes_received, 0);
        assert_eq!(st.packets_received, 0);
        assert_eq!(st.program_maps_seen, 0);
        assert_eq!(st.per_stream.len(), 0);
    }

    #[test]
    fn reset_stats_clears_per_stream() {
        let mut r = DemuxReceiver::new(CannedRecv {
            chunks: VecDeque::new(),
            alive: true,
        });
        r.reset_stats();
        let st = r.stats();
        assert!(st.per_stream.is_empty());
    }
}
