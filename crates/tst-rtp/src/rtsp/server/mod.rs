//! RTSP server — accepts client connections, manages sessions, fans out
//! one Muxer's TS bytes to N connected peers.
//!
//! Phase 3 — populated across Waves A through G.

pub mod auth;
pub mod builder;
pub mod fanout;
pub mod handlers;
pub mod interleaved_pump;
pub mod listener;
pub mod mount;
pub mod multicast;
pub mod runtime;
pub mod session;
#[cfg(feature = "tls")]
pub mod tls;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use tokio::runtime::Runtime;
use tokio_util::sync::CancellationToken;

use crate::builder::RtspServerBuilder;
use crate::cancel::RtspServerCancelHandle;
use crate::error::RtspServerError;

/// Internal server state shared between the listener task, per-session
/// tasks, and mount handles. `Arc<ServerState>` lives as long as any
/// task references it; cloning is cheap.
///
/// `dead_code` allowed on several fields: Wave C wires the mounts
/// hashmap; Tasks 8 + 9 wire `active_sessions` / `total_rtp_*`.
#[allow(dead_code)]
pub(crate) struct ServerState {
    pub(crate) builder: RtspServerBuilder,
    /// Graceful-shutdown signal. `stop()` flips this; per-session tasks
    /// observe via `.cancelled()` await.
    pub(crate) cancel_token: CancellationToken,
    /// Hard-cancel signal (independent from graceful). Exposed publicly
    /// via [`RtspServer::cancel_handle`].
    pub(crate) hard_cancel: RtspServerCancelHandle,
    /// Mount path → mount state. Wave C populates this via
    /// `RtspServer::add_mount`.
    pub(crate) mounts: std::sync::Mutex<
        std::collections::HashMap<String, Arc<crate::rtsp::server::mount::MountState>>,
    >,
    /// Live count of accepted (and not-yet-closed) client sessions.
    pub(crate) active_sessions: AtomicUsize,
    /// Cumulative RTP packets sent across all peers + all mounts.
    pub(crate) total_rtp_packets_sent: AtomicU64,
    /// Cumulative RTP bytes sent across all peers + all mounts.
    pub(crate) total_rtp_bytes_sent: AtomicU64,
    /// `start()` flips this once; `start()` returns AlreadyStarted on the
    /// second call.
    pub(crate) started: AtomicBool,
    /// `stop()` flips this. After this is true, public methods return
    /// `RtspServerError::Shutdown`.
    pub(crate) shutdown: AtomicBool,
    /// Bound address — set by the listener after kernel-assigns the port
    /// (when bind URL had `port = 0`). `start()` spin-waits on this.
    pub(crate) local_addr: std::sync::Mutex<Option<SocketAddr>>,
}

/// RTSP server — accepts client connections, manages sessions, fans out
/// one Muxer's TS bytes to N connected peers.
///
/// Sync facade over an internal tokio `Runtime` (constructed in
/// `bind`/`build`, dropped in `Drop`). All public methods are sync; the
/// runtime is hidden from callers.
///
/// # Closing
///
/// Three shutdown patterns:
///
/// 1. **Drop** — fires the hard-cancel path: all per-session tasks abort
///    at their next poll, the runtime is shut down with a 5 s budget.
///    Implicit; no acknowledgement to connected clients.
/// 2. **Graceful — `stop()`** — sends an RTSP Notice (5402) to each
///    active session, allows up to
///    `RtspServerBuilder::graceful_shutdown_drain` for in-flight RTP to
///    drain, then closes the listener and runtime. Returns once drain
///    is done.
/// 3. **Hard cross-thread — `cancel_handle()`** — returns an
///    [`RtspServerCancelHandle`] that can be cancelled from any thread.
///    Equivalent to Drop's hard-cancel without dropping the handle.
pub struct RtspServer {
    pub(crate) state: Arc<ServerState>,
    pub(crate) runtime: Option<Runtime>,
}

impl std::fmt::Debug for RtspServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtspServer")
            .field("bind_url", &self.state.builder.bind_url)
            .field("started", &self.state.started.load(Ordering::Relaxed))
            .field("shutdown", &self.state.shutdown.load(Ordering::Relaxed))
            .field("local_addr", &*self.state.local_addr.lock().unwrap())
            .field(
                "active_sessions",
                &self.state.active_sessions.load(Ordering::Relaxed),
            )
            .field(
                "mounts",
                &self.state.mounts.lock().map(|m| m.len()).unwrap_or(0),
            )
            .finish()
    }
}

impl RtspServer {
    /// Internal — called from [`crate::builder::RtspServerBuilder::build`].
    /// Constructs the tokio Runtime and the shared `ServerState`.
    pub(crate) fn from_builder(b: RtspServerBuilder) -> Result<Self, RtspServerError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("tst-rtp-server")
            .build()
            .map_err(|e| RtspServerError::Io(e.kind()))?;
        let state = Arc::new(ServerState {
            builder: b,
            cancel_token: CancellationToken::new(),
            hard_cancel: RtspServerCancelHandle::new(),
            mounts: std::sync::Mutex::new(std::collections::HashMap::new()),
            active_sessions: AtomicUsize::new(0),
            total_rtp_packets_sent: AtomicU64::new(0),
            total_rtp_bytes_sent: AtomicU64::new(0),
            started: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            local_addr: std::sync::Mutex::new(None),
        });
        Ok(Self {
            state,
            runtime: Some(runtime),
        })
    }

    /// Convenience: `RtspServerBuilder::new(url)?.build()`.
    pub fn bind(url: &str) -> Result<Self, RtspServerError> {
        RtspServerBuilder::new(url)?.build()
    }

    /// Convenience: `RtspServerBuilder::with_url(url).build()`.
    pub fn bind_with(url: crate::url::RtspUrl) -> Result<Self, RtspServerError> {
        RtspServerBuilder::with_url(url).build()
    }

    /// Begin accepting client connections. Spawns the listener task on
    /// the internal runtime and spin-waits up to 1 s for the listener to
    /// bind. Returns once `local_addr()` reflects the bound port.
    ///
    /// # Errors
    /// - [`RtspServerError::AlreadyStarted`] if called twice.
    /// - [`RtspServerError::Shutdown`] if called after `stop()`.
    /// - [`RtspServerError::Io`] on listener bind failure (Task 8 wires
    ///   the real bind; this stub returns Ok immediately).
    pub fn start(&self) -> Result<(), RtspServerError> {
        if self.state.shutdown.load(Ordering::Relaxed) {
            return Err(RtspServerError::Shutdown);
        }
        if self.state.started.swap(true, Ordering::AcqRel) {
            return Err(RtspServerError::AlreadyStarted);
        }
        let state = self.state.clone();
        let rt = self.runtime.as_ref().expect("runtime present until Drop");
        rt.spawn(async move {
            if let Err(e) = crate::rtsp::server::listener::run_listener(state).await {
                tracing::error!(target: "tst_rtp::server", error = ?e, "listener exited with error");
            }
        });
        // Spin-wait up to 1 s for the listener to bind + populate local_addr.
        // Once Task 8 wires the real listener, this is the synchronization
        // point that lets callers `local_addr()` immediately after `start()`.
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while std::time::Instant::now() < deadline {
            if self.state.local_addr.lock().unwrap().is_some() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // Listener stub didn't set local_addr — this is expected pre-Task 8.
        // Once Task 8 lands, this branch becomes an error rather than Ok.
        Ok(())
    }

    /// Graceful shutdown. Fires the cancel_token (per-session tasks
    /// observe), sleeps `graceful_shutdown_drain + 1s` to let them
    /// finish, then returns. Idempotent — a second call after a
    /// completed first call is a no-op.
    ///
    /// # Errors
    /// - [`RtspServerError::NotStarted`] if called before `start()`.
    pub fn stop(&self) -> Result<(), RtspServerError> {
        if !self.state.started.load(Ordering::Relaxed) {
            return Err(RtspServerError::NotStarted);
        }
        if self.state.shutdown.swap(true, Ordering::AcqRel) {
            // Idempotent: already shut down.
            return Ok(());
        }
        self.state.cancel_token.cancel();
        let drain = self.state.builder.graceful_shutdown_drain + Duration::from_secs(1);
        std::thread::sleep(drain);
        Ok(())
    }

    /// Listener's bound address, populated once `start()` returns. `None`
    /// before `start()` is called, or before the listener task gets
    /// scheduled (rare race; spin-wait in `start()` makes this
    /// observationally rare).
    pub fn local_addr(&self) -> Option<SocketAddr> {
        *self.state.local_addr.lock().unwrap()
    }

    /// Hard-cancel handle. Cloning is cheap; multiple holders can race
    /// the cancel call (idempotent).
    pub fn cancel_handle(&self) -> RtspServerCancelHandle {
        self.state.hard_cancel.clone()
    }

    /// Snapshot of aggregate server stats.
    pub fn stats(&self) -> ServerStats {
        ServerStats {
            active_sessions: self.state.active_sessions.load(Ordering::Relaxed),
            total_rtp_packets_sent: self.state.total_rtp_packets_sent.load(Ordering::Relaxed),
            total_rtp_bytes_sent: self.state.total_rtp_bytes_sent.load(Ordering::Relaxed),
            mounts: self.state.mounts.lock().map(|m| m.len()).unwrap_or(0),
        }
    }
}

impl Drop for RtspServer {
    fn drop(&mut self) {
        // Hard-cancel path on Drop — graceful shutdown blocks too long
        // for an implicit Drop.
        self.state.hard_cancel.cancel();
        self.state.cancel_token.cancel();
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(Duration::from_secs(5));
        }
    }
}

/// Aggregate server stats snapshot, returned from
/// [`RtspServer::stats`].
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServerStats {
    pub active_sessions: usize,
    pub total_rtp_packets_sent: u64,
    pub total_rtp_bytes_sent: u64,
    pub mounts: usize,
}

#[cfg(test)]
mod runtime_tests {
    use super::*;

    #[test]
    fn bind_returns_server_with_runtime() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        // local_addr not set until start()
        assert!(server.local_addr().is_none());
        let stats = server.stats();
        assert_eq!(stats.active_sessions, 0);
        assert_eq!(stats.mounts, 0);
    }

    #[test]
    fn start_returns_ok_when_listener_stub_runs() {
        // Task 7 stub: listener::run_listener returns Ok(()) immediately
        // without setting local_addr. start() spin-waits up to 1 s and
        // then returns Ok anyway. Task 8 will tighten this to fail if
        // local_addr isn't set.
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        server.start().unwrap();
        // local_addr stays None because the listener stub didn't bind.
        // Task 8's real run_listener sets it.
    }

    #[test]
    fn start_twice_errors_second_time() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        server.start().unwrap();
        let e = server.start().unwrap_err();
        assert!(matches!(e, RtspServerError::AlreadyStarted));
    }

    #[test]
    fn stop_before_start_errors() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        let e = server.stop().unwrap_err();
        assert!(matches!(e, RtspServerError::NotStarted));
    }

    #[test]
    fn stop_is_idempotent() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        // Override the drain timing for the test so it doesn't take 1+ s.
        // (We can't, because graceful_shutdown_drain is on the builder
        // not the server. We accept the ~1.1 s in this test for clarity.)
        server.start().unwrap();
        server.stop().unwrap();
        server.stop().unwrap(); // No-op the second time.
    }

    #[test]
    fn cancel_handle_clone_shares_flag() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        let h1 = server.cancel_handle();
        let h2 = server.cancel_handle();
        h1.cancel();
        assert!(h2.is_canceled());
    }

    #[test]
    fn drop_shuts_down_runtime() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        drop(server);
        // No panic / hang means shutdown_timeout completed cleanly.
    }

    #[test]
    fn server_stats_default() {
        let s = ServerStats::default();
        assert_eq!(s.active_sessions, 0);
        assert_eq!(s.total_rtp_packets_sent, 0);
    }
}
