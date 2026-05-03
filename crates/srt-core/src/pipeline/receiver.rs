// crates/srt-core/src/pipeline/receiver.rs
//! `Receiver<R>` — full receive: RecvTransport → TsReceiver → Demuxer.
//!
//! Mirrors the `Sender` shape on the receive side. Includes
//! `add_byte_sink` for fan-out: callbacks see every 188-byte TS packet
//! pulled from the transport, in registration order, before the demuxer
//! parses them. Useful for "write to disk + forward via RTP + demux for
//! KLV" workflows where multiple consumers tee off the same byte stream.
//!
//! # Byte-sink contract
//!
//! - Sinks are called once per TS packet (188 bytes), in the order they were
//!   registered via [`Receiver::add_byte_sink`].
//! - The slice passed to the sink is valid only for the duration of the call.
//!   Copy bytes into an owned buffer if they need to outlive the callback.
//! - Sinks must not panic. A panicking sink will unwind through `recv_event`.
//!
//! # Stream-end flush
//!
//! When the transport closes (`TransportError::Closed`), `Receiver` calls
//! `Demuxer::flush` before returning `Ok(None)`. This surfaces any partial
//! PES sitting in reassembly state (typically the final video AU, whose PES
//! packet length is 0 — length-unknown — and is only flushed when the next
//! PES starts or the stream ends). In normal live streams the flush emits
//! nothing; for finite test data it recovers the last sample.

use crate::error::DemuxError;
use crate::mpegts::demux::{DemuxEvent, Demuxer, DemuxerOptions};
use crate::pipeline::recv_transport::RecvTransport;
use crate::pipeline::transport::TransportError;
use crate::pipeline::ts_receiver::TsReceiver;

/// Type alias for a boxed byte-fanout callback registered via
/// [`Receiver::add_byte_sink`]. The callback receives one TS packet (188
/// bytes) per call.
pub type ByteSink = Box<dyn FnMut(&[u8]) + Send>;

/// Full receive shell: `RecvTransport → TsReceiver → Demuxer`, with optional
/// byte-sink fan-out.
///
/// `R` is any [`RecvTransport`] — typically [`SrtTransport`] for live
/// connections, or a test mock (e.g. `CannedTransport` in the integration
/// tests).
///
/// # Usage
///
/// ```ignore
/// let mut rx = Receiver::new(transport);
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
/// [`SrtTransport`]: crate::pipeline::SrtTransport
pub struct Receiver<R: RecvTransport> {
    ts: TsReceiver<R>,
    demux: Demuxer,
    byte_sinks: Vec<ByteSink>,
}

impl<R: RecvTransport> Receiver<R> {
    /// Wrap a transport with default demuxer options (lenient mode).
    pub fn new(transport: R) -> Self {
        Self {
            ts: TsReceiver::new(transport),
            demux: Demuxer::new(),
            byte_sinks: Vec::new(),
        }
    }

    /// Wrap a transport with custom demuxer options (e.g. strict mode).
    pub fn with_demux_options(transport: R, options: DemuxerOptions) -> Self {
        Self {
            ts: TsReceiver::new(transport),
            demux: Demuxer::with_options(options),
            byte_sinks: Vec::new(),
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
    /// - The transport fails → `Err(ReceiverError::Transport(e))`.
    /// - The demuxer rejects a packet in strict mode → `Err(ReceiverError::Demux(e))`.
    ///
    /// # MalformedPes note
    ///
    /// `DemuxError::MalformedPes` propagates as `ReceiverError::Demux` and
    /// terminates the receive loop. This matches the plan default (fatal
    /// propagation). If a production caller wants to skip bad PES and continue,
    /// it can match on `ReceiverError::Demux(DemuxError::MalformedPes { .. })`
    /// and call `recv_event` again; but the demuxer state after a malformed PES
    /// is undefined, so re-entry is discouraged without a design change. Tracked
    /// in the deferred-features list.
    pub fn recv_event(&mut self) -> Result<Option<DemuxEvent>, ReceiverError> {
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
                Err(other) => return Err(ReceiverError::Transport(other)),
            };
            // Fan-out to byte sinks in registration order before demuxing.
            for sink in &mut self.byte_sinks {
                sink(&pkt);
            }
            // Feed to demuxer. In lenient mode this only errors on
            // Unrecoverable (bad packet length) or MalformedPes. In strict
            // mode it can also return StrictRejection.
            self.demux.feed(&pkt).map_err(ReceiverError::Demux)?;
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
}

/// `Receiver` implements `Iterator` so callers can use `for result in &mut rx`
/// or `.collect()` patterns. EOF (`Ok(None)`) terminates the iterator.
/// Errors are surfaced as `Some(Err(e))` so the caller can distinguish a
/// transport error from a clean end of stream.
impl<R: RecvTransport> Iterator for Receiver<R> {
    type Item = Result<DemuxEvent, ReceiverError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.recv_event() {
            Ok(Some(e)) => Some(Ok(e)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

/// Errors that can be returned by [`Receiver::recv_event`].
#[derive(Debug, thiserror::Error)]
pub enum ReceiverError {
    /// The underlying transport closed unexpectedly or returned a fatal error.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// The demuxer rejected a packet (strict-mode violation, unrecoverable
    /// packet malformation, or malformed PES header).
    ///
    /// Re-entry into [`Receiver::recv_event`] after this variant is
    /// discouraged for `DemuxError::MalformedPes`: the demuxer's reassembly
    /// state is undefined past a bad PES header, so subsequent events may
    /// be inconsistent. Treat it as a stream-fatal signal until the demuxer
    /// gains lenient PES recovery.
    #[error(transparent)]
    Demux(#[from] DemuxError),
}
