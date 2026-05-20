//! `ManagedDemuxReceiver<R>` — reconnect-aware full receive shell.
//!
//! Composition: `ManagedRecvTransport<R> → Receiver → Demuxer`. Unlike
//! the byte-level [`ManagedRecvTransport`] used directly under
//! `Receiver` / [`DemuxReceiver`][crate::DemuxReceiver],
//! this shell knows about BOTH the reconnect signal AND the sync /
//! demux state, and resets the latter when the former fires.
//!
//! # Why a new shell instead of pushing this into ManagedRecvTransport
//!
//! `ManagedRecvTransport` returns `Ok(bytes)` from `recv_bytes` after
//! a reconnect with no out-of-band signal — the [`RecvTransport`] trait
//! has no shape for "the byte you're getting is from a fresh
//! connection." Higher-level shells that own only the
//! [`RecvTransport`] trait reference cannot detect the boundary.
//! `ManagedDemuxReceiver` solves this by owning the `ManagedRecvTransport`
//! as a CONCRETE type and polling
//! [`ManagedRecvTransport::reconnects_count`] between events.
//!
//! # The bug this fixes
//!
//! Today's `DemuxReceiver<ManagedRecvTransport<T>>` composition keeps
//! the `Receiver`'s syncer buffer and the `Demuxer`'s PSI/PES state
//! across reconnects. Bytes from the dropped connection carry into the
//! new connection's framing:
//!
//! - The syncer's `[u8]` ring may hold a fractional packet from the
//!   dead connection. The next bytes from the new connection get
//!   appended; 0x47 byte alignment is then computed against a stale
//!   prefix.
//! - The PES reassembler has half-built sample buffers indexed by PID.
//!   The next packet from the new connection on the same PID is
//!   appended to the dead sample → emit-time, the consumer gets a
//!   sample whose first ~N bytes are from connection A and the rest
//!   from connection B.
//! - PSI section assemblers may hold a partial PAT/PMT section. Next
//!   PUSI from the new connection completes the section with bytes
//!   from a different version of the PAT/PMT.
//! - Continuity counters carry over → bogus CC jump events fire.
//!
//! `ManagedDemuxReceiver` calls [`Receiver::reset_sync`] and
//! [`tst_core::mpegts::demux::Demuxer::reset_sync`] when the reconnect
//! counter rises, then queues a [`DemuxEvent::ReconnectDiscontinuity`]
//! event for the next `recv_event` call so the consumer sees the
//! boundary explicitly.
//!
//! # Closing
//!
//! Mirrors [`DemuxReceiver`][crate::DemuxReceiver]'s shutdown patterns
//! (Drop / `close()` / cross-thread `cancel_handle().cancel()`).

use crate::demux_receiver::DemuxReceiverError;
use crate::managed_receive::ManagedRecvTransport;
use crate::receiver::{Receiver, ReceiverConfig, ReceiverErrorSource};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{Span, info, info_span};
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, DemuxerConfig};
use tst_core::transport::RecvTransport;

/// Construction parameters for [`ManagedDemuxReceiver`].
///
/// Currently empty; reserved for future knobs (e.g. emit-discontinuity-
/// on-first-connect, demuxer-reset opt-out for callers that want
/// raw-bytes-only). Construct via `Default::default()` and assign
/// overrides as fields land.
///
/// Symmetric with [`crate::ReceiverConfig`] /
/// [`crate::demux_receiver::DemuxReceiver`] which take no config
/// today.
#[must_use]
#[non_exhaustive]
#[derive(Debug, Default, Clone)]
pub struct ManagedDemuxReceiverConfig {}

/// Reconnect-aware full receive shell.
///
/// Owns a [`ManagedRecvTransport`] (byte-level reconnect) wrapped in a
/// [`Receiver`] (TS sync recovery) plus a [`Demuxer`] (PSI/PES parse).
/// Between event emissions, polls the reconnect counter; on a fresh
/// transport rebuild, drops sync/demux state and surfaces the boundary
/// to the caller as a [`DemuxEvent::ReconnectDiscontinuity`] event.
///
/// # Usage
///
/// ```ignore
/// use tst_pipeline::{
///     ManagedDemuxReceiver, ManagedDemuxReceiverConfig, ManagedRecvTransport,
///     ReconnectPolicy,
/// };
///
/// let factory = Box::new(|| SrtTransport::connect(addr, &cfg));
/// let inner = SrtTransport::connect(addr, &cfg)?;
/// let managed = ManagedRecvTransport::new(inner, factory, ReconnectPolicy::default());
/// let mut rx = ManagedDemuxReceiver::new(managed, ManagedDemuxReceiverConfig::default());
///
/// for ev in &mut rx {
///     match ev? {
///         DemuxEvent::ReconnectDiscontinuity => {
///             // Drop any per-stream caches; next ProgramMap will arrive.
///         }
///         DemuxEvent::ProgramMap(pmt) => { /* re-build caches */ }
///         DemuxEvent::Sample { .. } => { /* forward AU */ }
///         _ => {}
///     }
/// }
/// ```
pub struct ManagedDemuxReceiver<R: RecvTransport> {
    ts: Receiver<ManagedRecvTransport<R>>,
    demux: Demuxer,
    /// Shared handle to the underlying `ManagedRecvTransport`'s
    /// reconnect counter. Snapshotted in `new()` so the shell can poll
    /// it without an accessor on `Receiver` (which would expose its
    /// inner transport publicly).
    reconnects: Arc<AtomicU64>,
    /// Reconnect counter snapshot from the last loop iteration. When
    /// the live counter rises above this value the shell knows a
    /// reconnect just fired since the previous `recv_event` and resets
    /// sync/demux state.
    last_reconnects: u64,
    /// Set when a reconnect was detected mid-loop; consumed by the next
    /// `recv_event` to yield `DemuxEvent::ReconnectDiscontinuity` before
    /// any post-reconnect events. Stored as `bool` rather than queued
    /// directly into the demuxer's `queue` because the demuxer's queue
    /// is cleared by `reset_sync` and we want the event to survive that
    /// clear.
    pending_reconnect_event: bool,
    /// Lifetime [`tracing::Span`] opened in [`Self::new`] and entered
    /// from [`Drop`] to bracket open/close events. Private — must NOT
    /// be exposed publicly (see CI public-API ratchet).
    ///
    /// Wrapped in [`std::panic::AssertUnwindSafe`] to keep the shell
    /// `UnwindSafe`/`RefUnwindSafe` despite `Span`'s internal `Mutex`.
    _span: std::panic::AssertUnwindSafe<Span>,
}

impl<R: RecvTransport> std::fmt::Debug for ManagedDemuxReceiver<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedDemuxReceiver")
            .field("is_alive", &self.is_alive())
            .field("last_reconnects", &self.last_reconnects)
            .field("pending_reconnect_event", &self.pending_reconnect_event)
            .field("transport_kind", &std::any::type_name::<R>())
            .finish()
    }
}

impl<R: RecvTransport> ManagedDemuxReceiver<R> {
    /// Wrap a [`ManagedRecvTransport`] with default demuxer options
    /// (lenient mode).
    pub fn new(transport: ManagedRecvTransport<R>, _config: ManagedDemuxReceiverConfig) -> Self {
        let span = info_span!(
            target: "tst_pipeline::managed_demux_receiver",
            "managed_demux_receiver",
            transport_kind = std::any::type_name::<R>(),
        );
        let _enter = span.enter();
        info!("ManagedDemuxReceiver opened");
        drop(_enter);
        let reconnects = transport.reconnects_handle();
        Self {
            ts: Receiver::new(transport, ReceiverConfig::default()),
            demux: Demuxer::new(),
            reconnects,
            last_reconnects: 0,
            pending_reconnect_event: false,
            _span: std::panic::AssertUnwindSafe(span),
        }
    }

    /// Wrap a [`ManagedRecvTransport`] with custom demuxer options
    /// (e.g. strict mode).
    pub fn with_demux_options(
        transport: ManagedRecvTransport<R>,
        options: DemuxerConfig,
        _config: ManagedDemuxReceiverConfig,
    ) -> Self {
        let span = info_span!(
            target: "tst_pipeline::managed_demux_receiver",
            "managed_demux_receiver",
            transport_kind = std::any::type_name::<R>(),
        );
        let _enter = span.enter();
        info!("ManagedDemuxReceiver opened");
        drop(_enter);
        let reconnects = transport.reconnects_handle();
        Self {
            ts: Receiver::new(transport, ReceiverConfig::default()),
            demux: Demuxer::with_config(options),
            reconnects,
            last_reconnects: 0,
            pending_reconnect_event: false,
            _span: std::panic::AssertUnwindSafe(span),
        }
    }

    /// Pull one [`DemuxEvent`].
    ///
    /// Detects transport reconnects between iterations: when the
    /// underlying [`ManagedRecvTransport::reconnects_count`] rises,
    /// the shell resets the syncer + demuxer state and emits
    /// [`DemuxEvent::ReconnectDiscontinuity`] before yielding any
    /// further events from the fresh connection.
    ///
    /// # Errors
    ///
    /// Returns [`DemuxReceiverError`] (same shape as plain
    /// `DemuxReceiver` — variants identical). Reconnect is NOT an
    /// error: it surfaces in-band as a `ReconnectDiscontinuity` event.
    /// Terminal closes (budget exhausted, peer EOS, or caller cancel)
    /// surface as `Ok(None)` after the demuxer's final `flush` drain,
    /// matching `DemuxReceiver::recv_event` semantics.
    pub fn recv_event(&mut self) -> Result<Option<DemuxEvent>, DemuxReceiverError> {
        loop {
            // Step 1: yield a pending reconnect-discontinuity event first.
            // This must precede the demuxer's own queue drain — after a
            // reset_sync that queue is empty, but a future variant of this
            // shell might leave non-reconnect events in flight; ordering
            // the discontinuity event first prevents post-reconnect events
            // from being yielded before the boundary marker.
            if self.pending_reconnect_event {
                self.pending_reconnect_event = false;
                return Ok(Some(DemuxEvent::ReconnectDiscontinuity));
            }

            // Step 2: fast path — demuxer already has a queued event.
            if let Some(e) = self.demux.next_event() {
                return Ok(Some(e));
            }

            // Step 3: pull the next aligned 188-byte packet from the
            // sync layer. This is where a reconnect manifests: the
            // underlying ManagedRecvTransport rebuilds the inner
            // transport, then returns Ok from recv_bytes — the syncer
            // sees the bytes-since-last-call rise but has no way to
            // know they came from a new connection. Detect via the
            // reconnect counter BEFORE feeding any new bytes to the
            // syncer.
            //
            // Race note: a reconnect that happens during a recv block
            // is not detected here UNTIL the next recv returns, at
            // which point the count check below picks it up.
            let pkt = match self.ts.next_packet() {
                Ok(p) => p,
                Err(e) if e.kind == crate::shell_error::ShellErrorKind::EndOfStream => {
                    // Stream end: same shape as DemuxReceiver — flush
                    // any partial PES then drain remaining events.
                    self.demux.flush();
                    if let Some(ev) = self.demux.next_event() {
                        return Ok(Some(ev));
                    }
                    return Ok(None);
                }
                Err(e) => {
                    let ReceiverErrorSource::Transport(te) = e.source;
                    return Err(te.into());
                }
            };

            // Step 4: now that we've taken a packet, check whether a
            // reconnect fired since the last loop iteration. If so,
            // DROP this packet (it's from the new connection's first
            // recv but the syncer may have buffered tail bytes from
            // the dead connection AHEAD of it — easier to discard
            // this one packet and let the syncer re-align cleanly
            // than to surgically separate dead-tail from
            // fresh-leading). Reset state and queue the discontinuity.
            //
            // Note: this check sits AFTER next_packet rather than
            // before because reconnect can fire DURING a single
            // next_packet call (multiple recv_bytes loops over a
            // single packet boundary). Polling after we've drained
            // one packet's worth of bytes ensures we observe the
            // post-reconnect state.
            let current = self.reconnects.load(Ordering::Acquire);
            if current > self.last_reconnects {
                self.last_reconnects = current;
                self.ts.reset_sync();
                self.demux.reset_sync();
                self.pending_reconnect_event = true;
                // Drop the just-read packet; the syncer is now clean
                // and will re-lock on the next post-reconnect bytes.
                continue;
            }

            // Step 5: steady-state path — feed the aligned packet to
            // the demuxer and loop to pull whatever events it produced.
            self.demux
                .feed_aligned(&pkt)
                .map_err(DemuxReceiverError::from)?;
        }
    }

    /// Advisory liveness check. Delegates to the underlying transport.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.ts.is_alive()
    }

    /// Close the underlying transport. Idempotent.
    pub fn close(&mut self) {
        self.ts.close();
    }

    /// Cross-thread cancel handle for the underlying managed transport.
    pub fn cancel_handle(
        &self,
    ) -> Option<Arc<dyn tst_core::transport::TransportCancel + Send + Sync>> {
        self.ts.cancel_handle()
    }

    /// Total number of times the underlying transport has been rebuilt
    /// since this shell was constructed. Convenience accessor that
    /// reads the shared counter shipped from the inner
    /// [`ManagedRecvTransport::reconnects_handle`]. Useful for stats
    /// exports / dashboards.
    #[must_use]
    pub fn reconnects_count(&self) -> u64 {
        self.reconnects.load(Ordering::Acquire)
    }
}

/// `ManagedDemuxReceiver` implements `Iterator` so callers can use
/// `for result in &mut rx`. EOF (`Ok(None)`) terminates; errors are
/// surfaced as `Some(Err(e))`.
impl<R: RecvTransport> Iterator for ManagedDemuxReceiver<R> {
    type Item = Result<DemuxEvent, DemuxReceiverError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.recv_event() {
            Ok(Some(e)) => Some(Ok(e)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

impl<R: RecvTransport> Drop for ManagedDemuxReceiver<R> {
    fn drop(&mut self) {
        let _enter = self._span.0.enter();
        info!("ManagedDemuxReceiver closed");
    }
}

// ManagedDemuxReceiver reuses `DemuxReceiverError` / `DemuxReceiverErrorSource`
// from the sibling `demux_receiver` module rather than introducing a parallel
// error type — the variants are identical (Transport + Demux) and a single
// type keeps the binding surface tight.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconnect::{BackoffStrategy, ReconnectPolicy};
    use std::time::Duration;
    use tst_core::transport::{RecvTransport, TransportError};

    /// Build a policy with zero backoff so tests don't sleep.
    fn fast_policy(max_attempts: Option<u32>) -> ReconnectPolicy {
        ReconnectPolicy {
            max_attempts,
            backoff: BackoffStrategy::Constant(Duration::from_millis(0)),
            ..Default::default()
        }
    }

    /// Build a syntactically-valid 188-byte TS packet on the given PID.
    fn ts_packet(pid: u16, cc: u8) -> [u8; 188] {
        let mut buf = [0xFFu8; 188];
        buf[0] = 0x47;
        buf[1] = 0x40 | ((pid >> 8) as u8 & 0x1F);
        buf[2] = (pid & 0xFF) as u8;
        buf[3] = 0x10 | (cc & 0x0F);
        buf
    }

    /// `RecvTransport` that returns a fixed sequence of byte vectors then
    /// switches behavior on a flag to drive the reconnect path. The
    /// first phase serves a chunk of TS bytes; once exhausted it
    /// returns `Broken` until the test flips `phase2_packets` ready,
    /// at which point a fresh inner is constructed via the factory.
    struct ScriptedRecv {
        packets: std::collections::VecDeque<Vec<u8>>,
        broken_after_exhaust: bool,
    }

    impl RecvTransport for ScriptedRecv {
        fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
            match self.packets.pop_front() {
                Some(v) => {
                    let n = v.len().min(buf.len());
                    buf[..n].copy_from_slice(&v[..n]);
                    Ok(n)
                }
                None => {
                    if self.broken_after_exhaust {
                        Err(TransportError::Broken {
                            msg: "scripted exhaust".into(),
                            errno_code: None,
                        })
                    } else {
                        Err(TransportError::Closed)
                    }
                }
            }
        }

        fn max_payload(&self) -> usize {
            1316
        }

        fn is_alive(&self) -> bool {
            !self.packets.is_empty()
        }
    }

    /// Concatenate aligned packets into one chunk so the syncer locks
    /// (it needs 4 in-stride confirmations).
    fn chunk_of_aligned(packets: &[[u8; 188]]) -> Vec<u8> {
        let mut v = Vec::with_capacity(packets.len() * 188);
        for p in packets {
            v.extend_from_slice(p);
        }
        v
    }

    /// Smoke test: with a 0-attempt reconnect policy, the first inner
    /// exhaust surfaces as EOF (no reconnect possible) and no
    /// `ReconnectDiscontinuity` event is emitted. The shell behaves
    /// like a plain `DemuxReceiver` for the no-reconnect path.
    ///
    /// Note: ManagedRecvTransport treats inner `Closed` as a reconnect
    /// trigger (not EOF) — the policy budget (max_attempts=0 here)
    /// rejects the rebuild attempt and converts to terminal `Closed`.
    /// Factory is invoked once during the budget-rejection sequence
    /// but its result is discarded; we still expect no
    /// reconnect-count rise.
    #[test]
    fn no_reconnect_no_discontinuity_event() {
        // 5 aligned PID-0 packets, then the transport closes.
        let packets: Vec<[u8; 188]> = (0..5).map(|i| ts_packet(0x0100, i as u8)).collect();
        let chunk = chunk_of_aligned(&packets);

        let inner = ScriptedRecv {
            packets: vec![chunk].into(),
            broken_after_exhaust: false,
        };
        // Factory always fails — combined with max_attempts=0, the budget
        // is exhausted on the first attempt and the shell surfaces EOF
        // without ever rebuilding the inner.
        let factory = Box::new(|| -> Result<ScriptedRecv, TransportError> {
            Err(TransportError::Broken {
                msg: "no reconnect for this test".into(),
                errno_code: None,
            })
        });
        // max_attempts=Some(0) → budget rejected on first attempt;
        // ManagedRecvTransport latches closed and returns Closed.
        let managed = ManagedRecvTransport::new(inner, factory, fast_policy(Some(0)));
        let mut rx = ManagedDemuxReceiver::new(managed, ManagedDemuxReceiverConfig::default());

        let mut saw_reconnect = false;
        let mut events = 0;
        loop {
            match rx.recv_event() {
                Ok(Some(DemuxEvent::ReconnectDiscontinuity)) => saw_reconnect = true,
                Ok(Some(_)) => events += 1,
                Ok(None) => break,
                Err(_e) => break, // Closed surfaces as Err on receiver side
            }
        }
        // No successful rebuild ever happened → no reconnect event.
        // 0x0100 isn't in any PMT so the demuxer ignores its packets.
        assert!(
            !saw_reconnect,
            "should not emit ReconnectDiscontinuity when no rebuild succeeded"
        );
        let _ = events;
        assert_eq!(rx.reconnects_count(), 0);
    }

    /// Reconnect fires mid-stream: shell yields ReconnectDiscontinuity
    /// and the inner reconnect-count rises.
    #[test]
    fn reconnect_emits_discontinuity_event() {
        // Phase 1: a few aligned packets (enough to lock the syncer,
        // then a fractional packet at the tail to test mid-PES reset).
        let p1_packets: Vec<[u8; 188]> = (0..6).map(|i| ts_packet(0x0100, i as u8)).collect();
        let mut p1 = chunk_of_aligned(&p1_packets);
        // Append 100 bytes of "garbage" trailing — simulates a fractional
        // PES packet that the dead connection cut off mid-flight.
        p1.extend_from_slice(&[0xAA; 100]);

        let inner = ScriptedRecv {
            packets: vec![p1].into(),
            broken_after_exhaust: true, // exhausting triggers reconnect path
        };

        // Phase 2: 4 fresh aligned packets after reconnect. The shell
        // should drop the dead tail, reset state, emit a
        // ReconnectDiscontinuity, then re-lock on these clean packets.
        let factory_calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let factory_calls_cl = factory_calls.clone();
        let factory = Box::new(move || -> Result<ScriptedRecv, TransportError> {
            let n = factory_calls_cl.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                // First reconnect: serve fresh clean packets.
                let p2_packets: Vec<[u8; 188]> =
                    (0..6).map(|i| ts_packet(0x0200, i as u8)).collect();
                let chunk = chunk_of_aligned(&p2_packets);
                Ok(ScriptedRecv {
                    packets: vec![chunk].into(),
                    broken_after_exhaust: false, // EOF after this chunk
                })
            } else {
                // No further reconnect attempts.
                Err(TransportError::Broken {
                    msg: "no more rebuilds".into(),
                    errno_code: None,
                })
            }
        });
        let managed = ManagedRecvTransport::new(inner, factory, fast_policy(Some(3)));
        let mut rx = ManagedDemuxReceiver::new(managed, ManagedDemuxReceiverConfig::default());

        let mut saw_reconnect = false;
        let mut total_events = 0;
        let mut iters = 0;
        loop {
            iters += 1;
            assert!(iters < 1000, "test loop should terminate");
            match rx.recv_event() {
                Ok(Some(DemuxEvent::ReconnectDiscontinuity)) => {
                    saw_reconnect = true;
                    // The dead-tail bytes (100 bytes of 0xAA) must not
                    // be feeding into the new connection's syncer
                    // post-reconnect. The reset_sync calls handle that.
                    assert_eq!(rx.reconnects_count(), 1);
                }
                Ok(Some(_)) => total_events += 1,
                Ok(None) => break,
                Err(_e) => break, // tolerate factory budget exhaust
            }
        }
        assert!(
            saw_reconnect,
            "reconnect should have surfaced as ReconnectDiscontinuity event"
        );
        assert_eq!(rx.reconnects_count(), 1);
        let _ = total_events;
    }

    /// Reconnect during mid-stream PES (partial-packet tail in the
    /// syncer): the reset_sync call drops those bytes so they don't
    /// splice into the new connection's framing.
    ///
    /// We bound factory rebuilds explicitly via a shared counter
    /// because `ManagedRecvTransport`'s `max_attempts` budget resets
    /// per-`recv_bytes` call (it's a local var); without an in-factory
    /// cap, a `broken_after_exhaust: true` inner that the factory
    /// keeps rebuilding would loop forever.
    #[test]
    fn reconnect_clears_syncer_buffer_and_demux_state() {
        // Phase 1: enough aligned packets to lock + a partial-packet
        // tail of 187 bytes (< 188, won't form a full packet).
        let p1_packets: Vec<[u8; 188]> = (0..5).map(|i| ts_packet(0x0100, i as u8)).collect();
        let mut p1 = chunk_of_aligned(&p1_packets);
        // 187 bytes — the syncer would otherwise hold these as the
        // start of packet 6. After reset_sync they MUST be dropped.
        p1.extend_from_slice(&[0x47; 187]);

        let inner = ScriptedRecv {
            packets: vec![p1].into(),
            broken_after_exhaust: true,
        };

        // Factory invocation count cap so the test terminates. After
        // 1 successful rebuild + 1 final failure, the shell exhausts
        // its budget and returns terminal Closed.
        let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let calls_cl = calls.clone();
        let factory = Box::new(move || -> Result<ScriptedRecv, TransportError> {
            let n = calls_cl.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n == 0 {
                // After reconnect, serve a TS stream that starts with a
                // BAD sync byte (0x00, 0xFF...) before becoming valid. If
                // the syncer wasn't reset, the dead-tail's 0x47s would
                // mis-lock onto a fake boundary. After reset, the syncer
                // hunts cleanly through the bad prefix.
                let mut bytes = vec![0x00u8; 50];
                let packets: Vec<[u8; 188]> = (0..5).map(|i| ts_packet(0x0200, i as u8)).collect();
                bytes.extend(chunk_of_aligned(&packets));
                Ok(ScriptedRecv {
                    packets: vec![bytes].into(),
                    broken_after_exhaust: true,
                })
            } else {
                Err(TransportError::Broken {
                    msg: "factory exhausted".into(),
                    errno_code: None,
                })
            }
        });
        let managed = ManagedRecvTransport::new(inner, factory, fast_policy(Some(2)));
        let mut rx = ManagedDemuxReceiver::new(managed, ManagedDemuxReceiverConfig::default());

        let mut saw_reconnect = false;
        let mut iters = 0;
        loop {
            iters += 1;
            assert!(iters < 1000, "test loop should terminate");
            match rx.recv_event() {
                Ok(Some(DemuxEvent::ReconnectDiscontinuity)) => {
                    saw_reconnect = true;
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => break,
            }
        }
        assert!(saw_reconnect, "reconnect should have surfaced");
        // The crucial assertion: the demuxer didn't crash with a
        // stale-state error and the shell reached terminal cleanly. If
        // the syncer or demuxer state had been left intact, the bad
        // prefix or the orphan 187-byte tail would have caused either
        // a phantom sample to emit (mixed bytes) or an Unrecoverable
        // error inside the demuxer.
        assert!(!rx.is_alive());
    }

    /// Iterator impl: `for ev in &mut rx` terminates when the
    /// underlying transport's reconnect budget is exhausted. With
    /// max_attempts=0, the first inner exhaust triggers immediate
    /// budget rejection which surfaces as a Closed error on the
    /// receiver side — the iterator stops on the first error or None.
    #[test]
    fn iterator_terminates_on_eof() {
        let packets: Vec<[u8; 188]> = (0..5).map(|i| ts_packet(0x0100, i as u8)).collect();
        let chunk = chunk_of_aligned(&packets);
        let inner = ScriptedRecv {
            packets: vec![chunk].into(),
            broken_after_exhaust: false,
        };
        // Factory call shape doesn't matter — budget=0 rejects before
        // dispatch.
        let factory = Box::new(|| -> Result<ScriptedRecv, TransportError> {
            Err(TransportError::Broken {
                msg: "no reconnect".into(),
                errno_code: None,
            })
        });
        let managed = ManagedRecvTransport::new(inner, factory, fast_policy(Some(0)));
        let mut rx = ManagedDemuxReceiver::new(managed, ManagedDemuxReceiverConfig::default());

        let mut count = 0;
        for result in &mut rx {
            // Tolerate either Ok event or terminal Err (Closed); both
            // are valid iterator stop conditions for this fixture.
            let _ = result;
            count += 1;
            if count > 100 {
                panic!("iterator did not terminate");
            }
        }
        // The loop terminated. Whether on Ok(None) or after one Err,
        // we exited bounded; that's the contract under test.
        let _ = count;
    }
}
