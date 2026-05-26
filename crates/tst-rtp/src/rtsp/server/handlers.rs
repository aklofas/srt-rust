//! RTSP request handlers — dispatched from `session.rs`'s per-session
//! state machine. Task 10 ships:
//! - Shared helpers: `server_header`, `error_response`, `challenge_response`.
//! - OPTIONS handler (200 + Public + Server).
//! - DESCRIBE handler (auth-gated, returns SDP body for the matching
//!   mount, or 404 if mount path not registered).
//!
//! Stubs for SETUP/PLAY/PAUSE/TEARDOWN remain at 501 — Wave D
//! (Tasks 16-17) wires those. GET_PARAMETER ships fully (used by
//! client-initiated keepalive pings).
//!
//! Module-level `dead_code` allow: Task 10 ships these handler functions;
//! the per-session dispatcher in Task 9 (parallel sibling) is what calls
//! them. The allow comes off when T9 merges.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;

use crate::rtsp::message::{RtspRequest, RtspResponse};
use crate::rtsp::server::ServerState;
use crate::rtsp::server::auth::{AuthVerifyError, build_challenge_header, verify_authorization};
use crate::rtsp::server::session::ServerSessionState;

/// `Server:` header value for outbound responses.
pub(crate) fn server_header() -> String {
    format!("tst-rtp/{}", env!("CARGO_PKG_VERSION"))
}

/// Build a 4xx/5xx error response with copied CSeq + Server header.
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

/// Build a 401 Unauthorized response with a fresh `WWW-Authenticate`
/// challenge for the configured auth scheme. Mutates `session.auth_nonce`
/// if `stale = true` (rotates the nonce for a stale-challenge retry per
/// RFC 7616 §3.5).
pub(crate) fn challenge_response(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
    stale: bool,
) -> RtspResponse {
    let mut response = error_response(req, 401, "Unauthorized");
    if let Some(auth_cfg) = state.builder.auth.as_ref() {
        if stale {
            // Rotate the nonce so the client's next attempt uses a
            // fresh one. Nonce only matters for Digest; Basic ignores.
            session.auth_nonce = crate::rtsp::server::auth::generate_nonce();
        }
        let challenge = build_challenge_header(auth_cfg, &session.auth_nonce);
        response
            .headers
            .insert("www-authenticate".into(), challenge);
    }
    response
}

/// Verify auth for an authenticated request. Returns `Ok(())` if auth
/// is either not required or successfully verified; returns the
/// challenge response (401) if auth is required and failed.
fn check_auth(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
    method_str: &str,
) -> Result<(), RtspResponse> {
    let Some(auth_cfg) = state.builder.auth.as_ref() else {
        return Ok(()); // No auth configured — allow.
    };
    let auth_header = req.headers.get("authorization").map(String::as_str);
    match verify_authorization(
        auth_header,
        method_str,
        &req.uri,
        auth_cfg,
        &session.auth_nonce,
    ) {
        Ok(()) => Ok(()),
        Err(AuthVerifyError::StaleNonce) => {
            // Surface stale=true so the client knows to retry with the
            // new nonce instead of treating it as a credentials error.
            Err(challenge_response(req, state, session, true))
        }
        Err(_) => Err(challenge_response(req, state, session, false)),
    }
}

/// OPTIONS handler — RFC 7826 §10.1 / RFC 2326 §10.1.
///
/// Returns the list of supported methods in the `Public:` header.
/// OPTIONS itself never requires auth (it's the connectivity probe).
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

/// DESCRIBE handler — RFC 7826 §10.2.
///
/// 1. Auth check (401 if missing/wrong/stale).
/// 2. Extract mount path from request URI; look up in state.mounts.
/// 3. 404 if mount not registered.
/// 4. 200 OK with `Content-Type: application/sdp` + SDP body from
///    `Sdp::build_for_mount`.
pub(crate) fn handle_describe(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
) -> RtspResponse {
    if let Err(challenge) = check_auth(req, state, session, "DESCRIBE") {
        return challenge;
    }

    // Extract mount path. URI may be a full rtsp://host/path or a
    // path-only "/path" form (some cameras send relative URIs).
    let mount_path = extract_mount_path(&req.uri);
    let mounts = match state.mounts.lock() {
        Ok(m) => m,
        Err(_) => return error_response(req, 500, "Internal Server Error"),
    };
    let mount = match mounts.get(&mount_path) {
        Some(m) => m.clone(),
        None => {
            // Wave C populates mounts. Pre-Wave-C, every DESCRIBE returns
            // 404 — that's fine for the initial integration test setup.
            return error_response(req, 404, "Not Found");
        }
    };
    drop(mounts);

    let (kind_is_multicast, multicast_addr) = match &mount.kind {
        crate::rtsp::server::mount::MountKind::Multicast { group, .. } => (true, Some(*group)),
        _ => (false, None),
    };

    let local_addr = match *state.local_addr.lock().unwrap() {
        Some(addr) => addr,
        None => return error_response(req, 500, "Internal Server Error"),
    };
    let connection_addr = if kind_is_multicast {
        multicast_addr.unwrap_or(local_addr)
    } else {
        local_addr
    };
    let body = crate::sdp::Sdp::build_for_mount(&mount_path, connection_addr, kind_is_multicast);

    let mut headers = HashMap::new();
    if let Some(cseq) = req.headers.get("cseq") {
        headers.insert("cseq".into(), cseq.clone());
    }
    headers.insert("server".into(), server_header());
    headers.insert("content-type".into(), "application/sdp".into());
    headers.insert("content-length".into(), body.len().to_string());
    RtspResponse {
        version: req.version,
        status: 200,
        reason: "OK".into(),
        headers,
        body,
    }
}

/// Extract the path component from a request URI. Handles both full
/// `rtsp://host:port/path` and path-only `/path` forms.
fn extract_mount_path(uri: &str) -> String {
    // Strip scheme + authority if present.
    let path_start = if let Some(after_scheme) = uri.strip_prefix("rtsp://") {
        after_scheme
            .find('/')
            .map(|idx| &after_scheme[idx..])
            .unwrap_or("/")
    } else if let Some(after_scheme) = uri.strip_prefix("rtsps://") {
        after_scheme
            .find('/')
            .map(|idx| &after_scheme[idx..])
            .unwrap_or("/")
    } else {
        uri
    };
    // Strip any trailing query string; v1 matches the bare path component
    // of the mount as registered.
    let path = path_start.split('?').next().unwrap_or(path_start);
    path.to_string()
}

/// SETUP handler stub — Wave D Task 16.
#[allow(unused_variables)]
pub(crate) fn handle_setup(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
) -> RtspResponse {
    error_response(req, 501, "Not Implemented")
}

/// PLAY handler stub — Wave D Task 17.
#[allow(unused_variables)]
pub(crate) fn handle_play(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
) -> RtspResponse {
    error_response(req, 501, "Not Implemented")
}

/// PAUSE handler stub — Wave D Task 17.
#[allow(unused_variables)]
pub(crate) fn handle_pause(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
) -> RtspResponse {
    error_response(req, 501, "Not Implemented")
}

/// TEARDOWN handler stub — Wave D Task 17.
#[allow(unused_variables)]
pub(crate) fn handle_teardown(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
) -> RtspResponse {
    error_response(req, 501, "Not Implemented")
}

/// GET_PARAMETER handler — v1 echoes 200 OK with cseq + session header
/// (used by client-initiated keepalive pings).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtsp::message::RtspMethod;
    use crate::url::RtspVersion;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
    use tokio_util::sync::CancellationToken;

    fn make_state() -> Arc<ServerState> {
        let builder = crate::builder::RtspServerBuilder::new("rtsp://127.0.0.1:0").unwrap();
        Arc::new(ServerState {
            builder,
            cancel_token: CancellationToken::new(),
            hard_cancel: crate::cancel::RtspServerCancelHandle::new(),
            mounts: std::sync::Mutex::new(std::collections::HashMap::new()),
            active_sessions: AtomicUsize::new(0),
            total_rtp_packets_sent: AtomicU64::new(0),
            total_rtp_bytes_sent: AtomicU64::new(0),
            started: AtomicBool::new(true),
            shutdown: AtomicBool::new(false),
            local_addr: std::sync::Mutex::new(Some("127.0.0.1:8554".parse().unwrap())),
        })
    }

    fn make_req(method: RtspMethod, uri: &str) -> RtspRequest {
        let mut headers = HashMap::new();
        headers.insert("cseq".into(), "1".into());
        RtspRequest {
            method,
            uri: uri.into(),
            version: RtspVersion::V1_0,
            headers,
            body: Bytes::new(),
        }
    }

    #[test]
    fn options_response_lists_public_methods() {
        let state = make_state();
        let req = make_req(RtspMethod::Options, "rtsp://127.0.0.1:8554/*");
        let resp = handle_options(&req, &state);
        assert_eq!(resp.status, 200);
        let public = resp.headers.get("public").unwrap();
        for m in [
            "OPTIONS",
            "DESCRIBE",
            "SETUP",
            "PLAY",
            "PAUSE",
            "TEARDOWN",
            "GET_PARAMETER",
        ] {
            assert!(public.contains(m), "missing method {m} in Public header");
        }
    }

    #[test]
    fn options_response_includes_cseq_and_server() {
        let state = make_state();
        let req = make_req(RtspMethod::Options, "rtsp://127.0.0.1:8554/");
        let resp = handle_options(&req, &state);
        assert_eq!(resp.headers.get("cseq").map(String::as_str), Some("1"));
        assert!(
            resp.headers
                .get("server")
                .map(|s| s.starts_with("tst-rtp/"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn describe_unknown_mount_returns_404() {
        let state = make_state();
        let req = make_req(RtspMethod::Describe, "rtsp://127.0.0.1:8554/nope");
        let mut session = ServerSessionState::new();
        let resp = handle_describe(&req, &state, &mut session);
        assert_eq!(resp.status, 404);
    }

    #[test]
    fn describe_with_auth_required_returns_401_when_no_authorization() {
        use secrecy::SecretString;
        let mut builder = crate::builder::RtspServerBuilder::new("rtsp://127.0.0.1:0").unwrap();
        builder.auth_digest_md5("tst", "admin", SecretString::new("secret".into()));
        let state = Arc::new(ServerState {
            builder,
            cancel_token: CancellationToken::new(),
            hard_cancel: crate::cancel::RtspServerCancelHandle::new(),
            mounts: std::sync::Mutex::new(std::collections::HashMap::new()),
            active_sessions: AtomicUsize::new(0),
            total_rtp_packets_sent: AtomicU64::new(0),
            total_rtp_bytes_sent: AtomicU64::new(0),
            started: AtomicBool::new(true),
            shutdown: AtomicBool::new(false),
            local_addr: std::sync::Mutex::new(Some("127.0.0.1:8554".parse().unwrap())),
        });
        let req = make_req(RtspMethod::Describe, "rtsp://127.0.0.1:8554/live");
        let mut session = ServerSessionState::new();
        let resp = handle_describe(&req, &state, &mut session);
        assert_eq!(resp.status, 401);
        let www_auth = resp.headers.get("www-authenticate").unwrap();
        assert!(www_auth.starts_with("Digest"));
        assert!(www_auth.contains("realm=\"tst\""));
    }

    #[test]
    fn extract_mount_path_strips_scheme_and_authority() {
        assert_eq!(extract_mount_path("rtsp://host:8554/live"), "/live");
        assert_eq!(extract_mount_path("rtsps://host/live"), "/live");
        assert_eq!(extract_mount_path("/live"), "/live");
        assert_eq!(extract_mount_path("rtsp://host:8554/live?x=1"), "/live");
        // No path → root.
        assert_eq!(extract_mount_path("rtsp://host:8554"), "/");
    }

    #[test]
    fn error_response_includes_cseq_and_server() {
        let req = make_req(RtspMethod::Options, "/");
        let resp = error_response(&req, 404, "Not Found");
        assert_eq!(resp.status, 404);
        assert_eq!(resp.reason, "Not Found");
        assert_eq!(resp.headers.get("cseq").map(String::as_str), Some("1"));
        assert!(resp.headers.contains_key("server"));
    }
}
