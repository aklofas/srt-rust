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
use std::net::{SocketAddr, UdpSocket as StdUdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

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

/// SETUP handler — RFC 7826 §10.4 / RFC 2326 §12.10. Auth-gated.
/// Allocates per-session transport (UDP socket pair or interleaved
/// channel pair); returns 200 with Session + Transport response.
///
/// Rejection codes:
/// - 401 Unauthorized — auth check fails (via `check_auth`).
/// - 404 Not Found — mount path not registered.
/// - 400 Bad Request — Transport header missing or malformed.
/// - 461 Unsupported Transport — TCP-interleaved against a multicast
///   mount (RFC 7826 §13.3).
/// - 500 Internal Server Error — UDP socket bind failure or poisoned
///   mutex.
///
/// On 200, mutates `session` with: `session_id`, `mount_path`,
/// `transport`, and either `udp_sockets` (unicast UDP) or
/// `interleaved_channels` (TCP-interleaved). Multicast SETUP responses
/// reuse the per-mount sender task (T14) so no per-session socket pair
/// is allocated; the response just points the client at the multicast
/// group.
pub(crate) fn handle_setup(
    req: &RtspRequest,
    state: &Arc<ServerState>,
    session: &mut ServerSessionState,
) -> RtspResponse {
    if let Err(challenge) = check_auth(req, state, session, "SETUP") {
        return challenge;
    }

    // Look up the mount.
    let mount_path = extract_mount_path(&req.uri);
    let mounts = match state.mounts.lock() {
        Ok(m) => m,
        Err(_) => return error_response(req, 500, "Internal Server Error"),
    };
    let mount = match mounts.get(&mount_path) {
        Some(m) => m.clone(),
        None => return error_response(req, 404, "Not Found"),
    };
    drop(mounts);

    // Parse the Transport request header.
    let transport_str = match req.headers.get("transport") {
        Some(t) => t.as_str(),
        None => return error_response(req, 400, "Bad Request"),
    };
    let parsed =
        match crate::rtsp::client::transport_negotiation::parse_transport_response(transport_str) {
            Ok(p) => p,
            Err(_) => return error_response(req, 400, "Bad Request"),
        };

    // Per RFC 7826 §13.3: TCP-interleaved is incompatible with multicast.
    let is_multicast = matches!(
        mount.kind,
        crate::rtsp::server::mount::MountKind::Multicast { .. }
    );
    if is_multicast
        && matches!(
            parsed.kind,
            crate::rtsp::client::transport_negotiation::RtspTransportKind::TcpInterleaved
        )
    {
        return error_response(req, 461, "Unsupported Transport");
    }

    // Allocate transport-specific server state + build the response
    // header.
    use crate::rtsp::client::transport_negotiation::RtspTransportKind;
    let session_id = generate_session_id();
    let transport_response_header: String;
    match parsed.kind {
        RtspTransportKind::Udp => {
            if is_multicast {
                // Multicast SETUP: server points the client at the
                // group; the per-mount multicast sender task (T14) is
                // already publishing there.
                let (group, ttl) = match &mount.kind {
                    crate::rtsp::server::mount::MountKind::Multicast { group, ttl, .. } => {
                        (*group, *ttl)
                    }
                    _ => unreachable!("multicast path gated above"),
                };
                transport_response_header = format!(
                    "RTP/AVP;multicast;destination={};port={}-{};ttl={}",
                    group.ip(),
                    group.port(),
                    group.port() + 1,
                    ttl,
                );
            } else {
                // Unicast UDP: bind a server-side RTP+RTCP port pair.
                let local_ip = match *state.local_addr.lock().unwrap() {
                    Some(addr) => addr.ip(),
                    None => return error_response(req, 500, "Internal Server Error"),
                };
                let (rtp_sock, rtcp_sock, server_rtp_port) = match bind_server_udp_pair(local_ip) {
                    Ok(t) => t,
                    Err(_) => return error_response(req, 500, "Internal Server Error"),
                };
                let client_port = parsed.client_port.unwrap_or((0, 0));
                transport_response_header = format!(
                    "RTP/AVP;unicast;client_port={}-{};server_port={}-{}",
                    client_port.0,
                    client_port.1,
                    server_rtp_port,
                    server_rtp_port + 1,
                );
                session.udp_sockets = Some((rtp_sock, rtcp_sock));
            }
        }
        RtspTransportKind::TcpInterleaved => {
            // Allocate a fresh even/odd channel pair per session. v1
            // uses a process-global atomic counter — sufficient since
            // channels are per-session-scope on the wire (each client's
            // TCP connection has its own interleaved namespace).
            static NEXT_CHANNEL: AtomicU8 = AtomicU8::new(0);
            let base = NEXT_CHANNEL.fetch_add(2, Ordering::Relaxed);
            session.interleaved_channels = Some((base, base + 1));
            transport_response_header =
                format!("RTP/AVP/TCP;unicast;interleaved={}-{}", base, base + 1);
        }
    }

    session.session_id = Some(session_id.clone());
    session.mount_path = Some(mount_path);
    session.transport = Some(parsed);

    let mut headers = HashMap::new();
    if let Some(cseq) = req.headers.get("cseq") {
        headers.insert("cseq".into(), cseq.clone());
    }
    headers.insert("server".into(), server_header());
    headers.insert(
        "session".into(),
        format!(
            "{};timeout={}",
            session_id,
            state.builder.session_timeout.as_secs()
        ),
    );
    headers.insert("transport".into(), transport_response_header);
    RtspResponse {
        version: req.version,
        status: 200,
        reason: "OK".into(),
        headers,
        body: Bytes::new(),
    }
}

/// Generate a 16-hex-char session ID. Per RFC 7826 §17.3.2, session IDs
/// must be at least 8 characters; 16 hex is generous and avoids
/// collisions across concurrent SETUPs.
fn generate_session_id() -> String {
    let mut buf = [0u8; 8];
    if getrandom::getrandom(&mut buf).is_err() {
        // Fallback: timestamp-based. getrandom only fails on platforms
        // without an OS RNG, which we don't target.
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        buf.copy_from_slice(&nanos.to_be_bytes());
    }
    let mut s = String::with_capacity(16);
    for b in buf {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Bind a fresh server-side UDP RTP+RTCP socket pair on `bind_ip`.
///
/// RTCP MUST live on RTP+1 per RFC 3550 §11; we let the kernel pick the
/// RTP port (passing port 0) and retry up to 16 times if the
/// neighboring odd port is already in use.
///
/// Returns `(rtp_socket, rtcp_socket, rtp_port)`. Each socket is wrapped
/// as `Arc<tokio::net::UdpSocket>` so it can be cloned into the
/// per-peer fan-out task in T17.
fn bind_server_udp_pair(
    bind_ip: std::net::IpAddr,
) -> Result<
    (
        std::sync::Arc<tokio::net::UdpSocket>,
        std::sync::Arc<tokio::net::UdpSocket>,
        u16,
    ),
    std::io::Error,
> {
    for _attempt in 0..16 {
        let rtp_addr = SocketAddr::new(bind_ip, 0);
        let rtp_std = StdUdpSocket::bind(rtp_addr)?;
        rtp_std.set_nonblocking(true)?;
        let rtp_port = rtp_std.local_addr()?.port();
        // Avoid the (extremely rare) case where the kernel hands us a
        // port pair like (65535, 65536) where the +1 would wrap.
        if rtp_port == u16::MAX {
            drop(rtp_std);
            continue;
        }
        let rtcp_addr = SocketAddr::new(bind_ip, rtp_port + 1);
        match StdUdpSocket::bind(rtcp_addr) {
            Ok(rtcp_std) => {
                rtcp_std.set_nonblocking(true)?;
                let rtp = tokio::net::UdpSocket::from_std(rtp_std)?;
                let rtcp = tokio::net::UdpSocket::from_std(rtcp_std)?;
                return Ok((
                    std::sync::Arc::new(rtp),
                    std::sync::Arc::new(rtcp),
                    rtp_port,
                ));
            }
            Err(_) => {
                // Neighboring RTCP slot taken; retry with a fresh RTP
                // port.
                drop(rtp_std);
                continue;
            }
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AddrInUse,
        "could not find a free RTP+RTCP port pair",
    ))
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

    // ── SETUP handler tests (T16) ─────────────────────────────────────────

    use crate::rtsp::server::mount::{MountKind, MountState};
    use tst_core::mpegts::mux::{MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};

    fn make_muxer_cfg() -> MuxerConfig {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x1011, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().unwrap()
    }

    fn make_state_with_mount() -> Arc<ServerState> {
        let state = make_state();
        let mount_state = MountState::new("/live", MountKind::Unicast, make_muxer_cfg(), 256)
            .expect("mount state constructs");
        state
            .mounts
            .lock()
            .unwrap()
            .insert("/live".into(), mount_state);
        state
    }

    fn make_state_with_multicast_mount() -> Arc<ServerState> {
        let state = make_state();
        let mount_state = MountState::new(
            "/mc",
            MountKind::Multicast {
                group: "239.0.0.1:5004".parse().unwrap(),
                ttl: 4,
                iface: None,
            },
            make_muxer_cfg(),
            256,
        )
        .expect("multicast mount state constructs");
        state
            .mounts
            .lock()
            .unwrap()
            .insert("/mc".into(), mount_state);
        state
    }

    #[test]
    fn setup_with_no_transport_header_returns_400() {
        let state = make_state_with_mount();
        let req = make_req(RtspMethod::Setup, "rtsp://127.0.0.1:8554/live");
        // No Transport header.
        let mut session = ServerSessionState::new();
        let resp = handle_setup(&req, &state, &mut session);
        assert_eq!(resp.status, 400);
    }

    /// `tokio::net::UdpSocket::from_std` requires a tokio reactor in
    /// scope, so the UDP-path SETUP test runs under a #[tokio::test].
    #[tokio::test]
    async fn setup_with_udp_transport_returns_200_with_server_port() {
        let state = make_state_with_mount();
        let mut req = make_req(RtspMethod::Setup, "rtsp://127.0.0.1:8554/live");
        req.headers.insert(
            "transport".into(),
            "RTP/AVP;unicast;client_port=5004-5005".into(),
        );
        let mut session = ServerSessionState::new();
        let resp = handle_setup(&req, &state, &mut session);
        assert_eq!(resp.status, 200, "got: {} {}", resp.status, resp.reason);
        assert!(resp.headers.contains_key("session"));
        let transport_resp = resp.headers.get("transport").unwrap();
        assert!(
            transport_resp.contains("server_port="),
            "transport missing server_port: {transport_resp}"
        );
        assert!(
            transport_resp.contains("client_port=5004-5005"),
            "transport missing client_port echo: {transport_resp}"
        );
        assert!(session.session_id.is_some());
        assert!(session.udp_sockets.is_some());
        assert!(session.interleaved_channels.is_none());
        assert_eq!(session.mount_path.as_deref(), Some("/live"));
    }

    #[test]
    fn setup_with_tcp_interleaved_returns_200_with_channel_pair() {
        let state = make_state_with_mount();
        let mut req = make_req(RtspMethod::Setup, "rtsp://127.0.0.1:8554/live");
        req.headers.insert(
            "transport".into(),
            "RTP/AVP/TCP;unicast;interleaved=0-1".into(),
        );
        let mut session = ServerSessionState::new();
        let resp = handle_setup(&req, &state, &mut session);
        assert_eq!(resp.status, 200, "got: {} {}", resp.status, resp.reason);
        let transport_resp = resp.headers.get("transport").unwrap();
        assert!(
            transport_resp.contains("interleaved="),
            "transport missing interleaved=: {transport_resp}"
        );
        assert!(session.interleaved_channels.is_some());
        assert!(session.udp_sockets.is_none());
    }

    #[test]
    fn setup_against_multicast_mount_rejects_tcp_with_461() {
        let state = make_state_with_multicast_mount();
        let mut req = make_req(RtspMethod::Setup, "rtsp://127.0.0.1:8554/mc");
        req.headers.insert(
            "transport".into(),
            "RTP/AVP/TCP;unicast;interleaved=0-1".into(),
        );
        let mut session = ServerSessionState::new();
        let resp = handle_setup(&req, &state, &mut session);
        assert_eq!(resp.status, 461);
    }

    #[test]
    fn setup_unknown_mount_returns_404() {
        let state = make_state_with_mount();
        let mut req = make_req(RtspMethod::Setup, "rtsp://127.0.0.1:8554/nope");
        req.headers.insert(
            "transport".into(),
            "RTP/AVP;unicast;client_port=5004-5005".into(),
        );
        let mut session = ServerSessionState::new();
        let resp = handle_setup(&req, &state, &mut session);
        assert_eq!(resp.status, 404);
    }

    /// Multicast SETUP returns the multicast Transport response shape
    /// (destination + port + ttl) without binding a per-session UDP
    /// pair.
    #[test]
    fn setup_against_multicast_mount_returns_multicast_transport() {
        let state = make_state_with_multicast_mount();
        let mut req = make_req(RtspMethod::Setup, "rtsp://127.0.0.1:8554/mc");
        req.headers.insert(
            "transport".into(),
            "RTP/AVP;multicast;client_port=5004-5005".into(),
        );
        let mut session = ServerSessionState::new();
        let resp = handle_setup(&req, &state, &mut session);
        assert_eq!(resp.status, 200, "got: {} {}", resp.status, resp.reason);
        let transport_resp = resp.headers.get("transport").unwrap();
        assert!(transport_resp.contains("multicast"));
        assert!(transport_resp.contains("destination=239.0.0.1"));
        assert!(transport_resp.contains("port=5004-5005"));
        assert!(transport_resp.contains("ttl=4"));
        // Multicast SETUP doesn't bind per-session UDP sockets — the
        // per-mount sender (T14) drives the actual sends.
        assert!(session.udp_sockets.is_none());
        assert!(session.session_id.is_some());
    }

    #[test]
    fn generate_session_id_is_16_hex_chars() {
        let id = generate_session_id();
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_session_id_changes_each_call() {
        let a = generate_session_id();
        let b = generate_session_id();
        assert_ne!(a, b);
    }
}
