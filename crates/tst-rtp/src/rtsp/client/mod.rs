//! `RtspClient` sync facade.
//!
//! Holds a single byte stream (plain TCP for `rtsp://`, rustls-wrapped
//! for `rtsps://`) for the control connection, behind an
//! `Arc<Mutex<Stream>>` so the main thread and the background
//! keepalive thread share the SAME stream — request/response exchanges
//! serialize under the mutex (RTSP isn't pipelined).

pub mod interleaved_pump;
pub mod keepalive;
pub mod options_describe;
pub mod play;
pub mod session;
pub mod setup;
pub mod teardown;
pub mod tls;
pub mod transport_negotiation;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;

use crate::error::RtspError;
use crate::rtsp::client::interleaved_pump::PumpStats;
use crate::url::{RtspScheme, RtspUrl, RtspVersion};

/// The control-plane byte stream — plain TCP for `rtsp://`, or
/// rustls-wrapped TCP for `rtsps://` when the `tls` cargo feature is
/// enabled.
///
/// Hidden behind the same `Read + Write` shape so per-method code in
/// `options_describe.rs`, `setup.rs`, etc. is agnostic to which
/// transport carries the bytes.
#[derive(Debug)]
pub(crate) enum Stream {
    Plain(TcpStream),
    #[cfg(feature = "tls")]
    Tls(Box<tls::TlsStream>),
}

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.read(buf),
            #[cfg(feature = "tls")]
            Stream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Stream::Plain(s) => s.write(buf),
            #[cfg(feature = "tls")]
            Stream::Tls(s) => s.write(buf),
        }
    }
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        match self {
            Stream::Plain(s) => s.write_all(buf),
            #[cfg(feature = "tls")]
            Stream::Tls(s) => s.write_all(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Stream::Plain(s) => s.flush(),
            #[cfg(feature = "tls")]
            Stream::Tls(s) => s.flush(),
        }
    }
}

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
    /// The control-plane byte stream — plain TCP for `rtsp://`, or
    /// rustls-wrapped TCP for `rtsps://` (under the `tls` feature).
    ///
    /// Wrapped in `Arc<Mutex<...>>` so the main thread + the background
    /// keepalive thread can share the SAME stream (no `try_clone` —
    /// rustls `ClientConnection` isn't clonable, so TLS keepalive would
    /// otherwise be impossible). RTSP isn't pipelined (one in-flight
    /// request at a time), so holding the lock through each
    /// request/response exchange is correct and the contention with the
    /// keepalive thread is negligible.
    pub(crate) stream: Arc<Mutex<Stream>>,
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
    /// Interleaved-pump state — `Some` after a successful TCP-interleaved
    /// SETUP has activated the producer thread that drains the control
    /// TCP into [`mpsc`] channels (data / rtcp / ctrl). When this is
    /// `Some`, `send_and_read` writes the outbound request under the
    /// stream mutex but reads the response from
    /// [`InterleavedPumpState::ctrl_rx`] (matching by CSeq) — reading the
    /// stream directly would race against the pump.
    pub(crate) pump_state: Option<InterleavedPumpState>,
}

/// State the main thread keeps about the interleaved producer thread.
///
/// Owned by [`RtspClient`] (one pump per client, since one TCP control
/// connection per client). The pump thread is reaped in `Drop`.
#[derive(Debug)]
pub(crate) struct InterleavedPumpState {
    /// Pump-only cancel flag (separate from `RtspClient::cancel` so we
    /// can stop the pump without stopping the rest of the client; in
    /// practice they're flipped together at `Drop`).
    pub(crate) cancel: Arc<AtomicBool>,
    /// Receiver for RTSP responses parsed by the pump. The pump pushes
    /// each `CRLFCRLF`+body-bounded RTSP response here; `send_and_read`
    /// polls this matching by CSeq once pump mode is active.
    pub(crate) ctrl_rx: mpsc::Receiver<Bytes>,
    /// Pump-thread handle; joined in `Drop` after `cancel` is flipped.
    pub(crate) thread: Option<std::thread::JoinHandle<()>>,
    /// Observable counters from the pump. Held here so a future
    /// diagnostic accessor (e.g. `RtspClient::pump_stats()`) can read
    /// them without racing the pump thread. Not yet exposed publicly.
    #[allow(dead_code)]
    pub(crate) stats: Arc<PumpStats>,
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
    /// Connect to an `rtsp://` or `rtsps://` URL. The `rtsps://` scheme
    /// requires the `tls` cargo feature; otherwise it returns
    /// [`RtspError::Tls`].
    ///
    /// # Errors
    ///
    /// - [`RtspError::Url`] if the URL cannot be parsed.
    /// - [`RtspError::Io`] on socket-level failure (DNS, refused, etc.).
    /// - [`RtspError::Tls`] if the URL scheme is `rtsps://` and the
    ///   `tls` cargo feature is not enabled, or on rustls handshake
    ///   failure (server name validation, untrusted cert, etc.).
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
        Self::connect_with_roots(url, None)
    }

    /// Connect with an optional client-side TLS root-cert store.
    ///
    /// `roots = None` falls back to the platform native trust roots
    /// (loaded via `rustls-native-certs`). `roots = Some(custom)` is
    /// used by `RtspClientBuilder::tls_root_certs` callers that need
    /// to trust a self-signed cert (e.g., test fixtures).
    ///
    /// For plain `rtsp://` URLs the roots argument is ignored.
    ///
    /// # Errors
    ///
    /// See [`Self::connect`].
    pub fn connect_with_roots(
        url: &RtspUrl,
        #[cfg(feature = "tls")] roots: Option<rustls::RootCertStore>,
        #[cfg(not(feature = "tls"))] roots: Option<()>,
    ) -> Result<Self, RtspError> {
        let _ = &roots; // silence unused on non-tls builds
        let is_tls = matches!(url.scheme(), RtspScheme::Rtsps);
        #[cfg(not(feature = "tls"))]
        if is_tls {
            return Err(RtspError::Tls(
                "TLS support requires the 'tls' cargo feature".into(),
            ));
        }

        let host_port = (url.host.as_str(), url.port);
        let mut addrs = host_port
            .to_socket_addrs()
            .map_err(|e| RtspError::Io(e.kind()))?;
        let peer = addrs
            .next()
            .ok_or(RtspError::Io(std::io::ErrorKind::AddrNotAvailable))?;
        let tcp = TcpStream::connect_timeout(&peer, Duration::from_secs(10))
            .map_err(|e| RtspError::Io(e.kind()))?;
        tcp.set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(|e| RtspError::Io(e.kind()))?;
        tcp.set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| RtspError::Io(e.kind()))?;
        tcp.set_nodelay(true).ok();

        // Branch the stream construction. For `rtsps://` we hand the
        // TCP socket to the rustls handshake; the resulting TlsStream
        // exposes the same Read+Write shape the rest of the client
        // expects.
        let stream = if is_tls {
            #[cfg(feature = "tls")]
            {
                Stream::Tls(Box::new(tls::TlsStream::connect(url, tcp, roots)?))
            }
            #[cfg(not(feature = "tls"))]
            {
                // Unreachable: the early-return above already short-
                // circuited. Kept as a compile-time guard.
                unreachable!("tls feature disabled but rtsps:// reached connect path")
            }
        } else {
            Stream::Plain(tcp)
        };

        Ok(Self {
            stream: Arc::new(Mutex::new(stream)),
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
            pump_state: None,
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
        // Share the same `Arc<Mutex<Stream>>` with the keepalive thread.
        // Per-ping the thread locks the mutex, writes the OPTIONS bytes,
        // unlocks. Works uniformly for `Stream::Plain` AND `Stream::Tls`
        // — pre-T21 the Tls variant skipped keepalive entirely because
        // rustls `ClientConnection` isn't clonable.
        let write_half = self.stream.clone();
        let cancel = self.cancel.clone();
        let session_dead = Arc::new(AtomicBool::new(false));
        let session_id = Arc::new(Mutex::new(self.session_id.clone()));
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

    /// Spawn the interleaved producer thread (TCP-interleaved transport).
    ///
    /// Called from SETUP after a successful TCP-interleaved negotiation
    /// (see [`crate::rtsp::client::setup`]). The pump owns reads from
    /// the control TCP from this point on: it parses `$<ch><len><data>`
    /// frames, routes RTP payloads to `data_rx` (one of the channels
    /// returned here — the session hands it to `RtpRecvTransport`),
    /// routes RTCP payloads to `rtcp_rx` (the other channel returned
    /// here — T28 plumbs it into the `RtcpReporterHandle`), and routes
    /// RTSP responses to `InterleavedPumpState::ctrl_rx` so subsequent
    /// [`Self::send_and_read`] calls can match by CSeq.
    ///
    /// Idempotent in the sense that calling it twice produces a fresh
    /// pump and drops the previous one (the previous pump's `cancel`
    /// flips, its `data_rx` becomes unfed and the receiver-transport
    /// side will see `mpsc::RecvError`).
    ///
    /// Returns `(data_rx, rtcp_rx)`. Prior to Phase 4 Stage 3 (T27) the
    /// pump's RTCP receiver was consumed by a tiny `rtsp-rtcp-drain`
    /// std::thread that discarded everything; that drain has been
    /// removed and the receiver is now returned upward so a caller (T28)
    /// can route RTCP frames into the existing `RtcpReporterHandle`
    /// instead of black-holing them.
    pub(crate) fn activate_interleaved_pump(
        &mut self,
        channels: interleaved_pump::InterleavedChannels,
    ) -> (mpsc::Receiver<Bytes>, mpsc::Receiver<Bytes>) {
        // Reap any prior pump (replacement semantics — should not
        // happen in normal SETUP flow, but be defensive).
        if let Some(prev) = self.pump_state.take() {
            prev.cancel.store(true, Ordering::Relaxed);
            if let Some(t) = prev.thread {
                let _ = t.join();
            }
        }

        let (data_tx, data_rx) = mpsc::channel::<Bytes>();
        let (rtcp_tx, rtcp_rx) = mpsc::channel::<Bytes>();
        let (ctrl_tx, ctrl_rx) = mpsc::channel::<Bytes>();
        let pump_cancel = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(interleaved_pump::PumpStats::default());

        let reader = interleaved_pump::SharedStreamReader::new(self.stream.clone());
        let thread = interleaved_pump::spawn_client_pump(
            reader,
            data_tx,
            rtcp_tx,
            ctrl_tx,
            channels,
            pump_cancel.clone(),
            stats.clone(),
        );

        self.pump_state = Some(InterleavedPumpState {
            cancel: pump_cancel,
            ctrl_rx,
            thread: Some(thread),
            stats,
        });

        (data_rx, rtcp_rx)
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
        // Best-effort TEARDOWN if a session is still active. Done BEFORE
        // flipping cancel/pump-cancel so `send_and_read` (used by
        // teardown) still works.
        if self.session_id.is_some() {
            let _ = self.teardown();
        }
        // Flip cancel so the keepalive + pump threads break out of their
        // wake loops at the next poll. Then take + join the handles so
        // the threads are reaped before the TcpStream FD they hold is
        // closed by the main thread's `Drop`.
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(t) = self.keepalive_thread.take() {
            let _ = t.join();
        }
        if let Some(mut pump) = self.pump_state.take() {
            pump.cancel.store(true, Ordering::Relaxed);
            if let Some(t) = pump.thread.take() {
                let _ = t.join();
            }
            // The pump's RTCP `mpsc::Sender` is dropped along with the
            // pump thread that just exited; the rtcp_rx end was returned
            // upward at activate time (T27) so the receiver lives with
            // whoever consumes it (T28 plumbs it into
            // `RtcpReporterHandle`). Nothing to reap here.
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
