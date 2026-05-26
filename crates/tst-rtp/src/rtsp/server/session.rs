//! Per-connection async task — populated by Task 9 (Wave B).

use std::net::SocketAddr;
use std::sync::Arc;

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
    state
        .active_sessions
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    state
        .active_sessions
        .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}
