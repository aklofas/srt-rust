//! RTSP request handlers — dispatched from `session.rs`'s per-session
//! state machine. Task 9 ships function stubs; Task 10 (Wave B parallel)
//! implements OPTIONS + DESCRIBE; Wave D (T16-T17) adds
//! SETUP/PLAY/PAUSE/TEARDOWN.

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;

use crate::rtsp::message::{RtspRequest, RtspResponse};
use crate::rtsp::server::ServerState;
use crate::rtsp::server::session::ServerSessionState;

/// Default `Server:` header value for outbound responses.
pub(crate) fn server_header() -> String {
    format!("tst-rtp/{}", env!("CARGO_PKG_VERSION"))
}

/// Build an error response (4xx/5xx) with copied CSeq, Server, and a
/// reason phrase. Used by every handler for unhappy paths.
pub(crate) fn error_response(req: &RtspRequest, status: u16, reason: &str) -> RtspResponse {
    let mut headers = HashMap::new();
    if let Some(cseq) = req.headers.get("cseq") {
        headers.insert("cseq".into(), cseq.clone());
    }
    headers.insert("server".into(), server_header());
    RtspResponse {
        version: req.version,
        status,
        reason: reason.into(),
        headers,
        body: Bytes::new(),
    }
}

/// OPTIONS handler — Task 10 ships the real impl. Task 9 ships a
/// minimal-but-correct 200 OK with a Public header so loopback tests
/// can verify the wire path end-to-end.
#[allow(unused_variables)]
pub(crate) fn handle_options(req: &RtspRequest, state: &Arc<ServerState>) -> RtspResponse {
    let mut headers = HashMap::new();
    if let Some(cseq) = req.headers.get("cseq") {
        headers.insert("cseq".into(), cseq.clone());
    }
    headers.insert("server".into(), server_header());
    headers.insert(
        "public".into(),
        "OPTIONS, DESCRIBE, SETUP, PLAY, PAUSE, TEARDOWN, GET_PARAMETER".into(),
    );
    RtspResponse {
        version: req.version,
        status: 200,
        reason: "OK".into(),
        headers,
        body: Bytes::new(),
    }
}

/// DESCRIBE handler — Task 10 ships the real impl with auth +
/// SDP generation. Task 9 stubs with a 501 Not Implemented.
#[allow(unused_variables)]
pub(crate) fn handle_describe(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
) -> RtspResponse {
    error_response(req, 501, "Not Implemented")
}

/// SETUP handler — Wave D Task 16.
#[allow(unused_variables)]
pub(crate) fn handle_setup(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
) -> RtspResponse {
    error_response(req, 501, "Not Implemented")
}

/// PLAY handler — Wave D Task 17.
#[allow(unused_variables)]
pub(crate) fn handle_play(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
) -> RtspResponse {
    error_response(req, 501, "Not Implemented")
}

/// PAUSE handler — Wave D Task 17.
#[allow(unused_variables)]
pub(crate) fn handle_pause(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
) -> RtspResponse {
    error_response(req, 501, "Not Implemented")
}

/// TEARDOWN handler — Wave D Task 17.
#[allow(unused_variables)]
pub(crate) fn handle_teardown(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
) -> RtspResponse {
    error_response(req, 501, "Not Implemented")
}

/// GET_PARAMETER handler — server-side keepalive ping support.
/// v1 simply responds 200 OK with the session header echoed.
#[allow(unused_variables)]
pub(crate) fn handle_get_parameter(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
) -> RtspResponse {
    let mut headers = HashMap::new();
    if let Some(cseq) = req.headers.get("cseq") {
        headers.insert("cseq".into(), cseq.clone());
    }
    if let Some(sid) = req.headers.get("session") {
        headers.insert("session".into(), sid.clone());
    }
    headers.insert("server".into(), server_header());
    RtspResponse {
        version: req.version,
        status: 200,
        reason: "OK".into(),
        headers,
        body: Bytes::new(),
    }
}
