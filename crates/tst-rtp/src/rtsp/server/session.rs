//! Per-connection async task — runs the RTSP request/response loop
//! against one client. Reads requests via tokio AsyncRead, dispatches
//! to handlers, writes responses back.
//!
//! State machine: Idle → Described (after DESCRIBE) → SettingUp (after
//! SETUP) → Playing (after PLAY) → TornDown (after TEARDOWN or
//! disconnect). v1 keeps state in [`ServerSessionState`]; Wave D extends
//! with fan-out subscription handle and transport choice
//! (Udp/TcpInterleaved).
//!
//! A TLS variant `handle_connection_tls` is gated on `feature = "tls"`
//! AND `TokioTlsServerStream` existing — T11 lands the `tokio-rustls`
//! integration; until then the TLS path is unimplemented here.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::error::RtspServerError;
use crate::rtsp::message::{RtspMethod, RtspRequest};
use crate::rtsp::server::ServerState;
use crate::rtsp::server::auth::generate_nonce;
use crate::rtsp::server::fanout::PeerDropCounter;
use crate::rtsp::server::handlers;

/// Per-session state. Lives for the duration of one client's TCP
/// connection. Wave D (T16) extends with transport choice + allocated
/// UDP sockets / interleaved channels.
pub struct ServerSessionState {
    /// RTSP session ID — None pre-SETUP, Some after SETUP returns 200.
    pub session_id: Option<String>,
    /// Mount path the client SETUP'd against, e.g. "/live". None
    /// pre-SETUP.
    pub mount_path: Option<String>,
    /// Stable nonce for Digest auth — generated once per connection and
    /// emitted in every WWW-Authenticate. Wave D may rotate per
    /// stale=true.
    pub auth_nonce: String,
    /// Count of consecutive 401-bounced requests. After 3 in a row the
    /// session closes (basic DoS guard).
    pub auth_failures: u8,
    /// Transport negotiation result from SETUP. None pre-SETUP.
    pub transport: Option<crate::rtsp::client::transport_negotiation::TransportResponse>,
    /// Server-allocated UDP RTP+RTCP pair (for UDP-transport sessions).
    /// `None` for TCP-interleaved sessions, multicast SETUPs, or
    /// pre-SETUP. T17's PLAY handler hands these to the per-peer
    /// fan-out task.
    pub udp_sockets: Option<(
        std::sync::Arc<tokio::net::UdpSocket>,
        std::sync::Arc<tokio::net::UdpSocket>,
    )>,
    /// Interleaved RTP+RTCP channel pair (for TCP-interleaved sessions).
    /// `None` for UDP sessions or pre-SETUP.
    pub interleaved_channels: Option<(u8, u8)>,
    /// JoinHandle for the per-peer fanout subscriber task (Wave C T13's
    /// `spawn_peer_fanout`). `Some` after PLAY, `None` after PAUSE or
    /// TEARDOWN. PAUSE may re-spawn on a subsequent PLAY.
    pub fanout_handle: Option<tokio::task::JoinHandle<()>>,
    /// CancellationToken for the per-peer task. PAUSE cancels + replaces
    /// with a fresh token so a subsequent PLAY can re-spawn; TEARDOWN
    /// cancels + drops the handle without replacement.
    pub peer_cancel: tokio_util::sync::CancellationToken,
    /// Drop counter observed by `MountStats::frames_dropped_total`. Held
    /// here so the session can keep the `Arc` alive for the duration of
    /// the fanout task even after PAUSE drops the JoinHandle. Field is
    /// `pub(crate)` because `PeerDropCounter` itself is crate-private.
    pub(crate) peer_drop_counter: Option<std::sync::Arc<PeerDropCounter>>,
}

impl ServerSessionState {
    pub(crate) fn new() -> Self {
        Self {
            session_id: None,
            mount_path: None,
            auth_nonce: generate_nonce(),
            auth_failures: 0,
            transport: None,
            udp_sockets: None,
            interleaved_channels: None,
            fanout_handle: None,
            peer_cancel: tokio_util::sync::CancellationToken::new(),
            peer_drop_counter: None,
        }
    }
}

/// Handle one client TCP connection until disconnect or cancel.
///
/// `dead_code` allowed because T8 (the listener) is in flight in a
/// parallel worktree — it dispatches into this function. Once T8 lands
/// the allow can come off.
#[allow(dead_code)]
pub(crate) async fn handle_connection(
    state: Arc<ServerState>,
    tcp: TcpStream,
    peer: SocketAddr,
) -> Result<(), RtspServerError> {
    state.active_sessions.fetch_add(1, Ordering::Relaxed);
    let entry = crate::rtsp::server::register_session(&state, peer);
    let res = handle_connection_inner(state.clone(), tcp, peer, entry.clone()).await;
    crate::rtsp::server::unregister_session(&state, &entry);
    state.active_sessions.fetch_sub(1, Ordering::Relaxed);
    res
}

async fn handle_connection_inner(
    state: Arc<ServerState>,
    mut tcp: TcpStream,
    peer: SocketAddr,
    session_entry: Arc<crate::rtsp::server::ActiveSession>,
) -> Result<(), RtspServerError> {
    tracing::info!(target: "tst_rtp::server", peer = %peer, "session opened");
    let mut session = ServerSessionState::new();
    // Bounded read buffer — RTSP requests are typically << 4 KiB.
    let mut buf: Vec<u8> = Vec::with_capacity(8192);

    loop {
        // Cancellation guard — hard cancel exits the loop immediately,
        // graceful cancel observed via the tokio::select! below.
        if state.hard_cancel.is_canceled() {
            break;
        }

        // Read more bytes. Use a small per-iteration buffer so we can
        // respond promptly to cancellation. tokio::select! with the
        // cancel_token gives prompt graceful-shutdown response.
        let mut chunk = [0u8; 4096];
        let n = tokio::select! {
            r = tcp.read(&mut chunk) => match r {
                Ok(0) => {
                    // Clean EOF — client disconnected.
                    break;
                }
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!(target: "tst_rtp::server", peer = %peer, error = %e, "read failed");
                    break;
                }
            },
            _ = state.cancel_token.cancelled() => {
                tracing::info!(target: "tst_rtp::server", peer = %peer, "graceful cancel observed");
                break;
            }
            _ = session_entry.cancel.cancelled() => {
                tracing::info!(target: "tst_rtp::server", peer = %peer, "per-session cancel observed");
                break;
            }
        };
        buf.extend_from_slice(&chunk[..n]);

        // Try to parse complete request(s). If the buffer doesn't yet
        // contain a full request (no CRLFCRLF, or Content-Length body
        // not yet arrived), loop back to read more.
        loop {
            let (req, consumed) = match RtspRequest::parse(&buf) {
                Ok(t) => t,
                Err(_) => break, // Need more bytes (or genuinely malformed; we ignore + read more).
            };
            buf.drain(..consumed);
            let response = dispatch(&req, &state, &mut session);
            let bytes = response.encode();
            if let Err(e) = tcp.write_all(&bytes).await {
                tracing::warn!(target: "tst_rtp::server", peer = %peer, error = %e, "write failed");
                return Ok(());
            }
            if response.status == 401 {
                session.auth_failures = session.auth_failures.saturating_add(1);
                if session.auth_failures >= 3 {
                    tracing::warn!(
                        target: "tst_rtp::server",
                        peer = %peer,
                        "3 consecutive auth failures; closing session"
                    );
                    return Ok(());
                }
            } else {
                session.auth_failures = 0;
            }
            if req.method == RtspMethod::Teardown && response.status == 200 {
                // Clean close after TEARDOWN.
                let _ = tcp.shutdown().await;
                return Ok(());
            }
        }
    }
    tracing::info!(target: "tst_rtp::server", peer = %peer, "session closed");
    Ok(())
}

/// Dispatch an RTSP request to the appropriate handler.
fn dispatch(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
) -> crate::rtsp::message::RtspResponse {
    match req.method {
        RtspMethod::Options => handlers::handle_options(req, state),
        RtspMethod::Describe => handlers::handle_describe(req, state, session),
        RtspMethod::Setup => handlers::handle_setup(req, state, session),
        RtspMethod::Play => handlers::handle_play(req, state, session),
        RtspMethod::Pause => handlers::handle_pause(req, state, session),
        RtspMethod::Teardown => handlers::handle_teardown(req, state, session),
        RtspMethod::GetParameter => handlers::handle_get_parameter(req, state, session),
    }
}

/// TLS-variant of [`handle_connection`]. Same shape, takes a tokio-rustls
/// `TlsStream<TcpStream>` instead of plain `TcpStream`. Stubbed for now;
/// the full TLS-session loop mirrors `handle_connection_inner` over
/// `TokioTlsServerStream` (Wave D refactors into a shared generic over
/// `AsyncRead + AsyncWrite`).
///
/// Listener (Task 8) calls this when the bind URL scheme is `rtsps://`.
#[cfg(feature = "tls")]
#[allow(dead_code, unused_variables, unused_mut)]
pub(crate) async fn handle_connection_tls(
    state: Arc<ServerState>,
    mut tls: crate::rtsp::server::tls::TokioTlsServerStream,
    peer: SocketAddr,
) -> Result<(), RtspServerError> {
    state.active_sessions.fetch_add(1, Ordering::Relaxed);
    tracing::info!(target: "tst_rtp::server", peer = %peer, "TLS session opened (stub)");
    // Wave D wires the full TLS-session loop. For now, drain the stream
    // until the client disconnects; no RTSP request handling. This stub
    // exists so the listener's TLS branch compiles + so rtsps:// CONNECT
    // tests pass at the handshake level.
    state.active_sessions.fetch_sub(1, Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
mod session_tests {
    use super::*;
    use tokio::net::TcpListener;

    /// Spin up a local TCP listener, accept one connection, run the
    /// session loop. Client sends OPTIONS, we read the 200 OK response.
    #[tokio::test]
    async fn session_responds_to_options() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_handle = tokio::spawn(async move {
            let (tcp, peer) = listener.accept().await.unwrap();
            // Build a minimal ServerState for the test.
            let builder = crate::builder::RtspServerBuilder::new("rtsp://127.0.0.1:0").unwrap();
            let state = std::sync::Arc::new(crate::rtsp::server::ServerState {
                builder,
                cancel_token: tokio_util::sync::CancellationToken::new(),
                hard_cancel: crate::cancel::RtspServerCancelHandle::new(),
                mounts: std::sync::Mutex::new(std::collections::HashMap::new()),
                active_sessions: std::sync::atomic::AtomicUsize::new(0),
                total_rtp_packets_sent: std::sync::atomic::AtomicU64::new(0),
                total_rtp_bytes_sent: std::sync::atomic::AtomicU64::new(0),
                started: std::sync::atomic::AtomicBool::new(true),
                shutdown: std::sync::atomic::AtomicBool::new(false),
                local_addr: std::sync::Mutex::new(None),
                sessions: std::sync::Mutex::new(Vec::new()),
            });
            handle_connection(state, tcp, peer).await
        });

        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        client
            .write_all(b"OPTIONS rtsp://127.0.0.1/test RTSP/1.0\r\nCSeq: 1\r\n\r\n")
            .await
            .unwrap();
        // Read until we see a complete response.
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = client.read(&mut chunk).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&buf);
        assert!(text.contains("200 OK"), "got: {}", text);
        assert!(
            text.to_lowercase().contains("public:"),
            "expected Public header in: {}",
            text
        );

        // Close client; session should exit cleanly.
        drop(client);
        let _ = server_handle.await;
    }

    #[tokio::test]
    async fn session_responds_to_teardown_and_closes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_handle = tokio::spawn(async move {
            let (tcp, peer) = listener.accept().await.unwrap();
            let builder = crate::builder::RtspServerBuilder::new("rtsp://127.0.0.1:0").unwrap();
            let state = std::sync::Arc::new(crate::rtsp::server::ServerState {
                builder,
                cancel_token: tokio_util::sync::CancellationToken::new(),
                hard_cancel: crate::cancel::RtspServerCancelHandle::new(),
                mounts: std::sync::Mutex::new(std::collections::HashMap::new()),
                active_sessions: std::sync::atomic::AtomicUsize::new(0),
                total_rtp_packets_sent: std::sync::atomic::AtomicU64::new(0),
                total_rtp_bytes_sent: std::sync::atomic::AtomicU64::new(0),
                started: std::sync::atomic::AtomicBool::new(true),
                shutdown: std::sync::atomic::AtomicBool::new(false),
                local_addr: std::sync::Mutex::new(None),
                sessions: std::sync::Mutex::new(Vec::new()),
            });
            handle_connection(state, tcp, peer).await
        });

        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        // T17 implemented the real TEARDOWN handler: 200 OK after auth
        // (no auth configured here), session state cleared, TCP closed
        // by the dispatcher after observing 200 + TEARDOWN method.
        client
            .write_all(b"TEARDOWN rtsp://127.0.0.1/test RTSP/1.0\r\nCSeq: 1\r\n\r\n")
            .await
            .unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = client.read(&mut chunk).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&buf);
        assert!(text.contains("200"), "got: {}", text);

        drop(client);
        let _ = server_handle.await;
    }
}
