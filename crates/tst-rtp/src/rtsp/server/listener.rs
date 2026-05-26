//! Server accept loop — populated by Task 8 (Wave B).

use std::sync::Arc;

use crate::error::RtspServerError;
use crate::rtsp::server::ServerState;

/// Task 7 stub: returns Ok immediately. Task 8 replaces with the real
/// tokio TcpListener accept loop that spawns per-connection tasks.
#[allow(unused_variables)]
pub(crate) async fn run_listener(state: Arc<ServerState>) -> Result<(), RtspServerError> {
    // Task 8 implementation. Stub is no-op so Task 7's RtspServer::start
    // compiles + tests can verify the lifecycle shape independently.
    Ok(())
}
