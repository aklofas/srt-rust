//! Per-connection async task — populated by Task 9 (Wave B).

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::net::TcpStream;

use crate::error::RtspServerError;
use crate::rtsp::server::ServerState;

/// Task 8 invokes this for every accepted connection. Task 7 ships only
/// the signature stub; Task 9 implements the real session state machine.
#[allow(dead_code, unused_variables)]
pub(crate) async fn handle_connection(
    state: Arc<ServerState>,
    tcp: TcpStream,
    peer: SocketAddr,
) -> Result<(), RtspServerError> {
    state.active_sessions.fetch_add(1, Ordering::Relaxed);
    state.active_sessions.fetch_sub(1, Ordering::Relaxed);
    Ok(())
}

/// TLS-variant of `handle_connection`. Same shape but takes a tokio_rustls
/// `TlsStream<TcpStream>` instead of plain `TcpStream`. T9 stubs this;
/// T11 ships the underlying TLS shape; Wave D refactors to a shared
/// generic over `AsyncRead + AsyncWrite`.
#[cfg(feature = "tls")]
#[allow(dead_code, unused_variables)]
pub(crate) async fn handle_connection_tls(
    state: Arc<ServerState>,
    tls: crate::rtsp::server::tls::TokioTlsServerStream,
    peer: SocketAddr,
) -> Result<(), RtspServerError> {
    state.active_sessions.fetch_add(1, Ordering::Relaxed);
    state.active_sessions.fetch_sub(1, Ordering::Relaxed);
    Ok(())
}
