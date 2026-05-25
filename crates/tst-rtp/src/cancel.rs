//! Cancellation primitive for the `RtpTransport` / `RtpRecvTransport`
//! send/recv UDP socket wrappers (defined in the `transport` module,
//! Task 7+).
//!
//! The transport's blocking send/recv loops use a 100 ms socket-level
//! timeout (`UdpSocket::set_read_timeout` / `set_write_timeout`); on each
//! timeout they check this `AtomicBool` and either continue or return
//! `TransportError::ExplicitClose`. This mirrors `tst-srt`'s
//! `SRTO_RCVTIMEO`/`SNDTIMEO` pattern — see `feedback_repo_standalone_guardrail.md`
//! and `crates/tst-srt/src/socket.rs` for the libsrt-side precedent.
//!
//! Cancel is `Send + Sync` so consumers can stash it in an
//! `Arc<dyn TransportCancel>` and share across worker threads.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tst_core::transport::TransportCancel;

/// Shared cancellation flag.
///
/// Construct one with [`Self::new`]; both the transport (poll path) and
/// any caller-facing cancel handle hold an `Arc<Self>`. Setting the flag
/// via [`Self::cancel`] wakes any thread parked in the transport on its
/// next ~100 ms timeout.
#[derive(Debug, Default)]
pub struct RtpCancelHandle {
    flag: AtomicBool,
}

impl RtpCancelHandle {
    /// New un-cancelled handle.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Mark this handle cancelled. Idempotent.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }

    /// Has cancellation been requested?
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

impl TransportCancel for RtpCancelHandle {
    fn cancel(&self) {
        RtpCancelHandle::cancel(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn new_is_not_cancelled() {
        let h = RtpCancelHandle::new();
        assert!(!h.is_cancelled());
    }

    #[test]
    fn cancel_sets_flag() {
        let h = RtpCancelHandle::new();
        h.cancel();
        assert!(h.is_cancelled());
    }

    #[test]
    fn cancel_is_idempotent() {
        let h = RtpCancelHandle::new();
        h.cancel();
        h.cancel();
        h.cancel();
        assert!(h.is_cancelled());
    }

    #[test]
    fn cancel_visible_across_threads() {
        let h = RtpCancelHandle::new();
        let h2 = h.clone();
        let t = thread::spawn(move || {
            for _ in 0..1000 {
                if h2.is_cancelled() {
                    return true;
                }
                std::thread::yield_now();
            }
            false
        });
        h.cancel();
        assert!(t.join().unwrap(), "child thread never observed cancel");
    }

    #[test]
    fn satisfies_transport_cancel_trait() {
        fn accept(_: &dyn TransportCancel) {}
        let h = RtpCancelHandle::new();
        accept(&*h);
    }
}
