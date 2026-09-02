//! [`RecvEndReason`] — structured, first-writer-wins record of why a
//! [`ManagedDemuxReceiver`][crate::ManagedDemuxReceiver]'s stream ended.
//!
//! Recv-side analogue of `tst-rtp`'s `StreamEndReasonHandle`
//! (`tst_rtp::rtsp::client::end_reason`) — `tst-pipeline` must not depend
//! on `tst-rtp`, so this is an independent type with the same
//! first-writer-wins-cell shape rather than a shared one.
//!
//! Today the managed-SRT recv path can only distinguish two terminal
//! conditions ([`crate::ManagedRecvTransport`]'s reconnect-budget
//! exhaustion and a caller-initiated cancel/close) — both of which
//! previously surfaced identically as `Ok(None)` / a `Closed`-kind error
//! with no further detail. [`ManagedDemuxReceiver::end_reason_handle`]
//! exposes which one actually happened, obtainable *before* the receiver
//! is moved into an opaque handle (e.g. a C binding's box) so a watchdog
//! thread can poll it after the receiver itself is gone.

use std::sync::{Arc, OnceLock};

/// Why a [`ManagedDemuxReceiver`][crate::ManagedDemuxReceiver]'s stream
/// ended.
///
/// Recorded once, first-writer-wins, at whichever site actually observed
/// the terminal condition — see [`RecvEndReasonHandle`]. `None` from
/// [`RecvEndReasonHandle::get`] means the stream is still live (or ended
/// through a path this type doesn't instrument).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecvEndReason {
    /// The underlying transport reported a genuine clean end-of-stream
    /// that was **not** [`crate::ManagedRecvTransport`]'s
    /// reconnect-budget-exhausted path.
    ///
    /// Not produced by the managed-SRT path today: on that path,
    /// `ShellErrorKind::EndOfStream` arises *only* when
    /// `ManagedRecvTransport`'s reconnect budget is exhausted — a peer
    /// FIN surfaces as `TransportError::Broken`, which the decorator
    /// retries rather than treating as a clean end (see
    /// [`RecvEndReason::ReconnectExhausted`]). The variant is kept for a
    /// future `RecvTransport` implementation that *can* signal a clean
    /// EOS distinct from budget exhaustion, and so downstream `match`
    /// arms written against this `#[non_exhaustive]` enum don't need
    /// rewriting when one arrives. Currently unused is correct, not dead
    /// code.
    EndOfStream,
    /// [`crate::ManagedRecvTransport`]'s reconnect decorator gave up
    /// after exhausting its [`crate::reconnect::ReconnectPolicy`]
    /// budget — the peer never came back within `max_attempts`.
    ReconnectExhausted,
    /// The caller explicitly closed the receiver or fired its cancel
    /// handle — not a wire-level failure.
    Cancelled,
}

/// Shared, first-writer-wins cell for a [`RecvEndReason`].
///
/// `Arc<OnceLock<_>>`-backed: cheap to clone, every clone observes the
/// same recorded value, and a value recorded on one thread becomes
/// visible to a clone read on another — the cross-thread contract a
/// watchdog thread needs. [`Self::get`] keeps working after the owning
/// [`ManagedDemuxReceiver`][crate::ManagedDemuxReceiver] has been
/// dropped, provided the handle was obtained (via
/// [`ManagedDemuxReceiver::end_reason_handle`][crate::ManagedDemuxReceiver::end_reason_handle])
/// *before* that drop — e.g. before the receiver is moved into an opaque
/// C handle.
#[derive(Debug, Clone, Default)]
pub struct RecvEndReasonHandle(Arc<OnceLock<RecvEndReason>>);

impl RecvEndReasonHandle {
    /// Record `r` iff no reason has been recorded yet. A second (or
    /// later) call is a silent no-op — first-writer-wins, so the site
    /// that actually observed the terminal condition first owns the
    /// reason and a later, less-specific signal can't clobber it.
    pub(crate) fn record(&self, r: RecvEndReason) {
        let _ = self.0.set(r);
    }

    /// The recorded reason, or `None` if the stream is still live (or
    /// ended through a path this type doesn't instrument).
    #[must_use]
    pub fn get(&self) -> Option<RecvEndReason> {
        self.0.get().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_handle_is_empty() {
        let h = RecvEndReasonHandle::default();
        assert!(h.get().is_none());
    }

    #[test]
    fn record_is_first_writer_wins() {
        let h = RecvEndReasonHandle::default();
        h.record(RecvEndReason::Cancelled);
        h.record(RecvEndReason::ReconnectExhausted);
        assert_eq!(
            h.get(),
            Some(RecvEndReason::Cancelled),
            "the SECOND record() call must not clobber the first"
        );
    }

    #[test]
    fn clone_shares_the_same_cell() {
        let a = RecvEndReasonHandle::default();
        let b = a.clone();
        a.record(RecvEndReason::ReconnectExhausted);
        assert_eq!(b.get(), Some(RecvEndReason::ReconnectExhausted));
    }

    /// A value recorded on one thread must be visible to a
    /// [`RecvEndReasonHandle`] clone held on another — the cross-thread
    /// contract `end_reason_handle()` exists for (a watchdog thread
    /// polling a receiver driven elsewhere).
    #[test]
    fn handle_observes_recording_across_threads() {
        let h = RecvEndReasonHandle::default();
        let h_writer = h.clone();
        let writer = std::thread::spawn(move || {
            h_writer.record(RecvEndReason::Cancelled);
        });
        writer.join().unwrap();
        assert_eq!(h.get(), Some(RecvEndReason::Cancelled));
    }

    /// Must be safe to hand to a different thread than the one driving
    /// I/O — bindings (JVM/Python/C) will do exactly that.
    #[test]
    fn recv_end_reason_handle_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RecvEndReasonHandle>();
    }
}
