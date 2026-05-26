//! Per-connection async task — populated by Task 9 (Wave B).

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::net::TcpStream;

use crate::error::RtspServerError;
use crate::rtsp::server::ServerState;

/// Per-session state. Minimal version — T9 (parallel sibling) extends.
///
/// Lives on the per-connection async task. Task 10 (this commit) needs
/// `auth_nonce` for the WWW-Authenticate challenge flow; the other
/// fields exist so the merge with T9's full struct is purely additive
/// rather than introducing fresh fields.
#[allow(dead_code)]
pub(crate) struct ServerSessionState {
    pub(crate) session_id: Option<String>,
    pub(crate) mount_path: Option<String>,
    pub(crate) auth_nonce: String,
    pub(crate) auth_failures: u8,
}

#[allow(dead_code)]
impl ServerSessionState {
    pub(crate) fn new() -> Self {
        Self {
            session_id: None,
            mount_path: None,
            auth_nonce: crate::rtsp::server::auth::generate_nonce(),
            auth_failures: 0,
        }
    }
}

/// Task 8 invokes this for every accepted connection. Task 7 ships only
/// the signature stub; Task 9 implements the real session state machine.
#[allow(dead_code, unused_variables)]
pub(crate) async fn handle_connection(
    state: Arc<ServerState>,
    tcp: TcpStream,
    peer: SocketAddr,
) -> Result<(), RtspServerError> {
    state
        .active_sessions
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    state
        .active_sessions
        .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}
