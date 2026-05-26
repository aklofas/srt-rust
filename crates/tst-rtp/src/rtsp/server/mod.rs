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

/// Random 32-bit SSRC seed for multicast mounts. Uses `getrandom`;
/// falls back to zero on the (impossible-in-practice) error path.
fn rand_ssrc() -> u32 {
    let mut buf = [0u8; 4];
    let _ = getrandom::getrandom(&mut buf);
    u32::from_be_bytes(buf)
}

/// Random initial RTP sequence per RFC 3550 §5.1.
fn rand_seq() -> u16 {
    let mut buf = [0u8; 2];
    let _ = getrandom::getrandom(&mut buf);
    u16::from_be_bytes(buf)
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

    /// Register a unicast mount under `path`. Returns a [`MountHandle`][crate::rtsp::server::mount::MountHandle]
    /// the caller pushes TS frames into. Multiple handles via
    /// [`MountHandle::clone`][crate::rtsp::server::mount::MountHandle::clone]
    /// can push from different threads.
    ///
    /// `path` must start with `/` and not contain URL-reserved characters
    /// like `?` or `#`.
    ///
    /// # Errors
    /// - [`RtspServerError::InvalidMountPath`] if `path` doesn't start with `/`
    ///   or is empty / contains URL-reserved characters.
    /// - [`RtspServerError::DuplicateMount`] if `path` is already registered.
    /// - [`RtspServerError::InvalidConfig`] if `MuxerConfig` validation fails.
    /// - [`RtspServerError::Shutdown`] if called after `stop()`.
    pub fn add_mount(
        &self,
        path: &str,
        cfg: tst_core::mpegts::mux::MuxerConfig,
    ) -> Result<crate::rtsp::server::mount::MountHandle, RtspServerError> {
        if self.state.shutdown.load(Ordering::Relaxed) {
            return Err(RtspServerError::Shutdown);
        }
        if path.is_empty() || !path.starts_with('/') {
            return Err(RtspServerError::InvalidMountPath {
                detail: format!("path must start with '/'; got '{path}'"),
            });
        }
        if path.contains('?') || path.contains('#') {
            return Err(RtspServerError::InvalidMountPath {
                detail: format!("path contains URL-reserved character: '{path}'"),
            });
        }
        let mount_state = crate::rtsp::server::mount::MountState::new(
            path,
            crate::rtsp::server::mount::MountKind::Unicast,
            cfg,
            self.state.builder.fanout_capacity,
        )?;
        let mut mounts = self.state.mounts.lock().expect("mounts mutex");
        if mounts.contains_key(path) {
            return Err(RtspServerError::DuplicateMount {
                path: path.to_string(),
            });
        }
        mounts.insert(path.to_string(), mount_state.clone());
        Ok(crate::rtsp::server::mount::MountHandle { state: mount_state })
    }

    /// Register a multicast mount. The provided `group_url` is an
    /// `rtp://<mcast-ip>:<port>?ttl=N&iface=ethN` URL pointing at the
    /// multicast group + port to publish on. A single per-mount
    /// background task drains the broadcast and sends to the group;
    /// per-client SETUP responses point clients at the group.
    ///
    /// # Errors
    /// - [`RtspServerError::InvalidMountPath`] — same rules as
    ///   [`Self::add_mount`].
    /// - [`RtspServerError::InvalidMulticastGroup`] — `group_url` is
    ///   malformed or the host isn't multicast.
    /// - [`RtspServerError::DuplicateMount`] — path already registered.
    /// - [`RtspServerError::InvalidConfig`] — `MuxerConfig` validation
    ///   failed.
    /// - [`RtspServerError::Shutdown`] — server stopped.
    ///
    /// # Panics
    ///
    /// None directly; the per-mount sender task is spawned on the
    /// runtime — if it panics during send, tracing emits a warn but
    /// the server stays up.
    pub fn add_multicast_mount(
        &self,
        path: &str,
        cfg: tst_core::mpegts::mux::MuxerConfig,
        group_url: &str,
    ) -> Result<crate::rtsp::server::mount::MountHandle, RtspServerError> {
        if self.state.shutdown.load(Ordering::Relaxed) {
            return Err(RtspServerError::Shutdown);
        }
        if path.is_empty() || !path.starts_with('/') {
            return Err(RtspServerError::InvalidMountPath {
                detail: format!("path must start with '/'; got '{path}'"),
            });
        }
        if path.contains('?') || path.contains('#') {
            return Err(RtspServerError::InvalidMountPath {
                detail: format!("path contains URL-reserved character: '{path}'"),
            });
        }
        let mcast = crate::url::MulticastGroup::parse(group_url).map_err(|e| {
            RtspServerError::InvalidMulticastGroup {
                addr: group_url.to_string(),
                detail: e.to_string(),
            }
        })?;
        let mount_state = crate::rtsp::server::mount::MountState::new(
            path,
            crate::rtsp::server::mount::MountKind::Multicast {
                group: mcast.addr,
                ttl: mcast.ttl,
                iface: mcast.iface.clone(),
            },
            cfg,
            self.state.builder.fanout_capacity,
        )?;
        let mut mounts = self.state.mounts.lock().expect("mounts mutex");
        if mounts.contains_key(path) {
            return Err(RtspServerError::DuplicateMount {
                path: path.to_string(),
            });
        }
        mounts.insert(path.to_string(), mount_state.clone());
        // Spawn the per-mount multicast sender task. The send socket is
        // built async on the runtime; we use spawn so add_multicast_mount
        // can return synchronously. If the socket build fails, the task
        // logs and exits — caller observes via tracing/stats, not via
        // the return value (matches the unicast handle pattern where
        // listener errors don't unwind to the caller).
        let rt = self.runtime.as_ref().expect("runtime present until Drop");
        let mount_clone = mount_state.clone();
        let cancel = self.state.cancel_token.clone();
        let group = mcast.addr;
        let ttl = mcast.ttl;
        let iface = mcast.iface.clone();
        let drop_counter = crate::rtsp::server::fanout::PeerDropCounter::new();
        rt.spawn(async move {
            match crate::rtsp::server::multicast::build_multicast_send_socket(
                group,
                ttl,
                iface.as_deref(),
            )
            .await
            {
                Ok(sock) => {
                    let sock = Arc::new(sock);
                    let rx = mount_clone.fanout.subscribe();
                    let _join = crate::rtsp::server::multicast::spawn_multicast_sender(
                        rx,
                        sock,
                        cancel,
                        rand_ssrc(),
                        rand_seq(),
                        drop_counter,
                    );
                    // The spawn_multicast_sender returns a JoinHandle we
                    // intentionally drop — the task lives until cancel
                    // or broadcast::Closed (which happens when MountState
                    // is dropped → fanout sender drops → all subscribers
                    // get Closed).
                }
                Err(e) => {
                    tracing::error!(
                        target: "tst_rtp::server::multicast",
                        error = ?e,
                        group = ?group,
                        "failed to build multicast send socket; mount inactive"
                    );
                }
            }
        });
        Ok(crate::rtsp::server::mount::MountHandle { state: mount_state })
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
    fn start_binds_listener_and_populates_local_addr() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        server.start().unwrap();
        // After start() returns, local_addr() should reflect the
        // kernel-assigned port.
        let addr = server.local_addr().expect("listener bound");
        assert_eq!(addr.ip().to_string(), "127.0.0.1");
        assert!(addr.port() > 0);
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

#[cfg(test)]
mod listener_tests {
    use super::*;

    #[test]
    fn double_start_errors() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        server.start().unwrap();
        let e = server.start().unwrap_err();
        assert!(matches!(e, RtspServerError::AlreadyStarted));
    }

    #[test]
    fn start_then_local_addr_returns_port() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        server.start().unwrap();
        assert!(server.local_addr().unwrap().port() > 0);
    }

    #[test]
    fn second_bind_to_same_port_fails() {
        let first = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        first.start().unwrap();
        let port = first.local_addr().unwrap().port();
        // Try to bind ANOTHER server to the same port.
        let second = RtspServer::bind(&format!("rtsp://127.0.0.1:{port}")).unwrap();
        // start() should observe the bind failure as the listener task
        // exits with an error and never sets local_addr. Spin-wait
        // in start() times out, but T7's stub returns Ok regardless;
        // post-T8, the listener fails the bind and start() returns Io.
        // Since the spin-wait returns Ok if local_addr never populates,
        // we instead poll local_addr — it should be None after start()
        // returns.
        second.start().unwrap();
        // Give the listener task a moment to fail+exit.
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(
            second.local_addr().is_none(),
            "second bind should have failed"
        );
    }
}

#[cfg(test)]
mod add_mount_tests {
    use super::*;
    use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};

    fn make_muxer_cfg() -> MuxerConfig {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x1011, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().unwrap()
    }

    #[test]
    fn add_mount_returns_handle_with_path() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        let mount = server.add_mount("/live", make_muxer_cfg()).unwrap();
        assert_eq!(mount.mount_path(), "/live");
        assert_eq!(server.stats().mounts, 1);
    }

    #[test]
    fn add_mount_rejects_empty_path() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        let e = server.add_mount("", make_muxer_cfg()).unwrap_err();
        assert!(matches!(e, RtspServerError::InvalidMountPath { .. }));
    }

    #[test]
    fn add_mount_rejects_path_without_leading_slash() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        let e = server.add_mount("live", make_muxer_cfg()).unwrap_err();
        assert!(matches!(e, RtspServerError::InvalidMountPath { .. }));
    }

    #[test]
    fn add_mount_rejects_duplicate_path() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        server.add_mount("/live", make_muxer_cfg()).unwrap();
        let e = server.add_mount("/live", make_muxer_cfg()).unwrap_err();
        assert!(matches!(e, RtspServerError::DuplicateMount { .. }));
    }

    #[test]
    fn add_mount_path_with_query_char_rejected() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        let e = server.add_mount("/live?x=1", make_muxer_cfg()).unwrap_err();
        assert!(matches!(e, RtspServerError::InvalidMountPath { .. }));
    }
}

#[cfg(test)]
mod add_multicast_mount_tests {
    use super::*;
    use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};

    fn make_muxer_cfg() -> MuxerConfig {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x1011, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().unwrap()
    }

    #[test]
    fn add_multicast_mount_returns_handle_for_v4_group() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        let mount = server
            .add_multicast_mount("/mc", make_muxer_cfg(), "rtp://239.0.0.1:5004")
            .unwrap();
        assert_eq!(mount.mount_path(), "/mc");
        assert!(matches!(
            mount.mount_kind(),
            crate::rtsp::server::mount::MountKind::Multicast { .. }
        ));
    }

    #[test]
    fn add_multicast_mount_rejects_unicast_group() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        let e = server
            .add_multicast_mount("/mc", make_muxer_cfg(), "rtp://10.0.0.1:5004")
            .unwrap_err();
        assert!(matches!(e, RtspServerError::InvalidMulticastGroup { .. }));
    }

    #[test]
    fn add_multicast_mount_rejects_malformed_url() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        let e = server
            .add_multicast_mount("/mc", make_muxer_cfg(), "not-a-url")
            .unwrap_err();
        assert!(matches!(e, RtspServerError::InvalidMulticastGroup { .. }));
    }

    #[test]
    fn add_multicast_mount_rejects_duplicate_path() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        server
            .add_multicast_mount("/mc", make_muxer_cfg(), "rtp://239.0.0.1:5004")
            .unwrap();
        let e = server
            .add_multicast_mount("/mc", make_muxer_cfg(), "rtp://239.0.0.2:5004")
            .unwrap_err();
        assert!(matches!(e, RtspServerError::DuplicateMount { .. }));
    }

    #[test]
    fn add_multicast_mount_carries_ttl_and_iface() {
        let server = RtspServer::bind("rtsp://127.0.0.1:0").unwrap();
        let mount = server
            .add_multicast_mount(
                "/mc",
                make_muxer_cfg(),
                "rtp://239.0.0.1:5004?ttl=2&iface=127.0.0.1",
            )
            .unwrap();
        match mount.mount_kind() {
            crate::rtsp::server::mount::MountKind::Multicast { ttl, iface, .. } => {
                assert_eq!(*ttl, 2);
                assert_eq!(iface.as_deref(), Some("127.0.0.1"));
            }
            _ => panic!("expected Multicast"),
        }
    }
}
