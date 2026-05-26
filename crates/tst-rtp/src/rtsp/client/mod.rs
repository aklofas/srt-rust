//! `RtspClient` sync facade.
//!
//! Holds one `std::net::TcpStream` for the control connection plus a
//! mutex-guarded write half (so concurrent main-thread requests and
//! background keepalive pings don't interleave bytes on the wire).

pub mod keepalive;
pub mod options_describe;
pub mod play;
pub mod session;
pub mod setup;
pub mod teardown;
pub mod tls;
pub mod transport_negotiation;

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use crate::error::RtspError;
use crate::url::{RtspScheme, RtspUrl, RtspVersion};

/// Sync RTSP client. One instance per server.
///
/// Methods (options, describe, setup, play, pause, teardown,
/// keepalive_now) are added by later tasks; this task ships only the
/// struct definition + `connect` / `connect_with`.
//
// `dead_code` allowed because several fields here are populated /
// consumed by later tasks in this plan (session lifecycle, keepalive,
// transport negotiation). The struct shape is intentionally fixed up
// front so each later task can dispatch in parallel against a stable
// surface.
#[allow(dead_code)]
#[derive(Debug)]
pub struct RtspClient {
    /// The TCP read half — used by the main thread + the
    /// `InterleavedReader` background thread when TCP-interleaved
    /// transport is in use.
    pub(crate) stream: TcpStream,
    /// Negotiated URL — caller can re-parse for re-connects.
    pub(crate) url: RtspUrl,
    /// Server's connection address as we resolved it.
    pub(crate) peer: SocketAddr,
    /// Monotonic CSeq counter; every outbound request bumps this.
    pub(crate) next_cseq: AtomicU32,
    /// Session ID from the most recent SETUP success. None before
    /// SETUP / after TEARDOWN.
    pub(crate) session_id: Option<String>,
    /// Server's `Session: ...;timeout=N` value (default 60 s if absent).
    pub(crate) session_timeout: Duration,
    /// Cancel flag — set by `RtspCancelHandle::cancel` to break out of
    /// blocking I/O loops.
    pub(crate) cancel: Arc<AtomicBool>,
    /// Last RTSP version observed in a server response.
    pub(crate) last_server_version: RtspVersion,
    /// Shared flag the [keepalive](crate::rtsp::client::keepalive) thread
    /// flips when a control-TCP write fails — the main thread polls this
    /// to detect server-side session death. `None` until
    /// [`Self::spawn_keepalive_if_needed`] runs.
    pub(crate) session_dead: Option<Arc<AtomicBool>>,
    /// Shared cell the main thread updates after SETUP so the keepalive
    /// thread can emit `Session: <id>` headers. `None` until
    /// [`Self::spawn_keepalive_if_needed`] runs.
    pub(crate) session_id_shared: Option<Arc<std::sync::Mutex<Option<String>>>>,
    /// JoinHandle for the rtsp-keepalive thread — joined in [`Drop`].
    /// `None` when keepalive is disabled or hasn't been spawned yet.
    pub(crate) keepalive_thread: Option<std::thread::JoinHandle<()>>,
}

/// Cancel handle for the RTSP client. Covers the control plane; the
/// transport plane (post-PLAY RTP data) uses its own
/// [`crate::RtpCancelHandle`] returned from the
/// [`crate::RtpRecvTransport`].
#[derive(Clone)]
pub struct RtspCancelHandle {
    cancel: Arc<AtomicBool>,
}

impl RtspCancelHandle {
    /// Signal the client to break out of blocking I/O at the next poll.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Has [`Self::cancel`] been called?
    pub fn is_canceled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

impl RtspClient {
    /// Connect to an `rtsp://` URL (plain TCP only — `rtsps://` lands
    /// in a later task gated by the `tls` cargo feature).
    ///
    /// # Errors
    ///
    /// - [`RtspError::Url`] if the URL cannot be parsed.
    /// - [`RtspError::Io`] on socket-level failure (DNS, refused, etc.).
    /// - [`RtspError::Tls`] if the URL scheme is `rtsps://` and the
    ///   `tls` cargo feature is not enabled.
    pub fn connect(url: &str) -> Result<Self, RtspError> {
        let parsed = RtspUrl::parse(url)?;
        Self::connect_with(&parsed)
    }

    /// Connect using an already-parsed URL.
    ///
    /// `rtsps://` URLs are not supported by this entry point on a build
    /// without the `tls` cargo feature; they return
    /// [`RtspError::Tls`] in that case.
    ///
    /// # Errors
    ///
    /// See [`Self::connect`].
    pub fn connect_with(url: &RtspUrl) -> Result<Self, RtspError> {
        if matches!(url.scheme(), RtspScheme::Rtsps) {
            #[cfg(not(feature = "tls"))]
            return Err(RtspError::Tls(
                "TLS support requires the 'tls' cargo feature".into(),
            ));
            // With the `tls` feature, a later task dispatches the
            // rustls handshake here.
        }
        let host_port = (url.host.as_str(), url.port);
        let mut addrs = host_port
            .to_socket_addrs()
            .map_err(|e| RtspError::Io(e.kind()))?;
        let peer = addrs
            .next()
            .ok_or(RtspError::Io(std::io::ErrorKind::AddrNotAvailable))?;
        let stream = TcpStream::connect_timeout(&peer, Duration::from_secs(10))
            .map_err(|e| RtspError::Io(e.kind()))?;
        stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(|e| RtspError::Io(e.kind()))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| RtspError::Io(e.kind()))?;
        stream.set_nodelay(true).ok();
        Ok(Self {
            stream,
            url: url.clone(),
            peer,
            next_cseq: AtomicU32::new(1),
            session_id: None,
            session_timeout: Duration::from_secs(60),
            cancel: Arc::new(AtomicBool::new(false)),
            last_server_version: RtspVersion::V1_0,
            session_dead: None,
            session_id_shared: None,
            keepalive_thread: None,
        })
    }

    /// Get a clone-able cancel handle.
    pub fn cancel_handle(&self) -> RtspCancelHandle {
        RtspCancelHandle {
            cancel: self.cancel.clone(),
        }
    }

    /// Server's reported RTSP version from the last response we parsed.
    pub fn last_server_version(&self) -> RtspVersion {
        self.last_server_version
    }

    /// Internal helper: get the next CSeq value.
    pub(crate) fn bump_cseq(&self) -> u32 {
        self.next_cseq.fetch_add(1, Ordering::Relaxed)
    }

    /// Spawn the background OPTIONS-pinger.
    ///
    /// `override_interval` lets callers force a specific cadence
    /// (typically supplied by `RtspClientBuilder::keepalive_interval`);
    /// when `None`, the cadence is `session_timeout / 2`.
    ///
    /// Idempotent — calling twice will replace the prior handle (any
    /// outstanding thread sees the `cancel` flag and exits at its next
    /// 200 ms wake).
    //
    // Exposed `#[doc(hidden)] pub` so the integration test in
    // `tests/rtsp_client_keepalive.rs` can drive it without going
    // through `RtspClientBuilder`. The builder also calls this.
    #[doc(hidden)]
    pub fn spawn_keepalive_if_needed(&mut self, override_interval: Option<Duration>) {
        let interval = override_interval.unwrap_or(self.session_timeout / 2);
        // Wrap the TcpStream's write half in an Arc<Mutex<TcpStream>>.
        // try_clone() yields a separate file descriptor pointing at the
        // same socket; we hand the clone to the keepalive thread so the
        // main thread keeps its own half for `read_response`-style reads.
        let write_clone = self.stream.try_clone().expect("TcpStream try_clone");
        let write_half = Arc::new(std::sync::Mutex::new(write_clone));
        let cancel = self.cancel.clone();
        let session_dead = Arc::new(AtomicBool::new(false));
        let session_id = Arc::new(std::sync::Mutex::new(self.session_id.clone()));
        self.session_dead = Some(session_dead.clone());
        self.session_id_shared = Some(session_id.clone());
        let handle = keepalive::spawn(
            write_half,
            cancel,
            session_dead,
            interval,
            self.url.render_no_credentials(),
            self.url.rtsp_version,
            session_id,
        );
        self.keepalive_thread = Some(handle);
    }

    /// Returns false if the background keepalive thread has flipped the
    /// session-dead flag (a control-TCP write failed). Returns true when
    /// keepalive hasn't been started or hasn't observed a failure.
    pub fn is_session_alive(&self) -> bool {
        match &self.session_dead {
            Some(flag) => !flag.load(Ordering::Relaxed),
            None => true,
        }
    }
}

impl Drop for RtspClient {
    fn drop(&mut self) {
        // Best-effort TEARDOWN if a session is still active.
        if self.session_id.is_some() {
            let _ = self.teardown();
        }
        // Flip cancel so the keepalive thread breaks out of its
        // 200 ms-wake loop at the next poll. Then take + join the
        // handle so the thread is reaped before the TcpStream FD it
        // holds is closed by the main thread's `Drop`.
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(t) = self.keepalive_thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn connect_to_loopback_listener_succeeds() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // Accept in background so connect_with doesn't hang.
        std::thread::spawn(move || {
            let _ = listener.accept();
        });
        let url = format!("rtsp://127.0.0.1:{}/test", port);
        let c = RtspClient::connect(&url).unwrap();
        assert_eq!(c.peer.port(), port);
        assert!(matches!(c.url.scheme(), RtspScheme::Rtsp));
    }

    #[test]
    #[cfg(not(feature = "tls"))]
    fn rtsps_without_tls_feature_errors() {
        let e = RtspClient::connect("rtsps://localhost:322/test").unwrap_err();
        assert!(matches!(e, RtspError::Tls(_)));
    }

    #[test]
    fn cancel_handle_toggles_flag() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let _ = listener.accept();
        });
        let url = format!("rtsp://127.0.0.1:{}/test", port);
        let c = RtspClient::connect(&url).unwrap();
        let h = c.cancel_handle();
        assert!(!h.is_canceled());
        h.cancel();
        assert!(h.is_canceled());
    }
}
