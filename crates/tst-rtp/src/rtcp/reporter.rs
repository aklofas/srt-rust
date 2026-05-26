//! Background thread emitting RR (receiver-side) or SR (sender-side)
//! at randomized `RTCP_INTERVAL` per RFC 3550 §6.2 + §6.3.1.
//!
//! v1 simplification: we don't run the full RFC 3550 transmission-interval
//! algorithm (which scales with session size); we use a fixed-base 5 s
//! interval with ±50% randomization per §6.3.1. Single-source point-to-point
//! flows don't benefit from the full algorithm.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

/// Base RTCP transmission interval, RFC 3550 §6.2 says 5 s for the
/// reduced minimum (after the initial RTCP_BANDWIDTH backoff). We
/// don't scale by session size in v1.
pub const RTCP_BASE_INTERVAL: Duration = Duration::from_secs(5);

/// Compute a randomized interval per RFC 3550 §6.3.1 — `interval *
/// uniform(0.5, 1.5)`.
pub fn jitter_interval(base: Duration, urandom: u32) -> Duration {
    // urandom is treated as 0..u32::MAX; scale to 0.5..1.5
    let factor = 0.5 + (urandom as f64 / u32::MAX as f64);
    Duration::from_secs_f64(base.as_secs_f64() * factor)
}

/// A handle to a running RTCP reporter thread. Dropping this handle
/// signals cancel and joins the thread.
pub struct RtcpReporterHandle {
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl RtcpReporterHandle {
    /// Spawn a reporter thread that calls `emit` at randomized intervals.
    /// The closure receives no args; the caller (the RTP sender or
    /// receiver transport) snapshots its own state inside the closure.
    pub fn spawn(emit: impl FnMut() + Send + 'static) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_thread = cancel.clone();
        let mut emit = emit;
        let thread = std::thread::Builder::new()
            .name("rtcp-reporter".to_string())
            .spawn(move || {
                let mut last_emit = std::time::Instant::now();
                while !cancel_thread.load(Ordering::Relaxed) {
                    let mut urandom = [0u8; 4];
                    getrandom::getrandom(&mut urandom).expect("getrandom failed");
                    let interval = jitter_interval(RTCP_BASE_INTERVAL, u32::from_le_bytes(urandom));
                    // Wake every 100 ms to check the cancel flag — same shape as
                    // the transport's cancel-handle pattern.
                    let deadline = last_emit + interval;
                    while std::time::Instant::now() < deadline
                        && !cancel_thread.load(Ordering::Relaxed)
                    {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    if cancel_thread.load(Ordering::Relaxed) {
                        break;
                    }
                    emit();
                    last_emit = std::time::Instant::now();
                }
            })
            .expect("failed to spawn rtcp-reporter thread");
        Self {
            cancel,
            thread: Some(thread),
        }
    }

    /// Signal the reporter thread to stop. Does not wait for the thread
    /// to exit — drop the handle to join.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for RtcpReporterHandle {
    fn drop(&mut self) {
        self.cancel();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_interval_min_max_bounds() {
        let base = Duration::from_secs(10);
        // urandom=0 → factor=0.5 → 5 s
        assert_eq!(jitter_interval(base, 0), Duration::from_secs_f64(5.0));
        // urandom=u32::MAX → factor=1.5 → 15 s
        let v = jitter_interval(base, u32::MAX);
        assert!(
            v >= Duration::from_secs_f64(14.999) && v <= Duration::from_secs_f64(15.001),
            "got {:?}",
            v
        );
    }

    #[test]
    fn spawn_emits_at_least_once() {
        use std::sync::atomic::AtomicUsize;
        let counter = Arc::new(AtomicUsize::new(0));
        let c = counter.clone();
        let handle = RtcpReporterHandle::spawn(move || {
            c.fetch_add(1, Ordering::Relaxed);
        });
        // Wait up to 10 s for the first emission (base = 5 s + ±50%, so worst-case 7.5 s).
        let start = std::time::Instant::now();
        while counter.load(Ordering::Relaxed) == 0 && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(counter.load(Ordering::Relaxed) >= 1);
        drop(handle);
    }

    #[test]
    fn drop_cancels_promptly() {
        let handle = RtcpReporterHandle::spawn(|| {});
        let start = std::time::Instant::now();
        drop(handle);
        // Cancel + join should be < 500 ms (100 ms wake + slack).
        assert!(start.elapsed() < Duration::from_millis(500));
    }
}
