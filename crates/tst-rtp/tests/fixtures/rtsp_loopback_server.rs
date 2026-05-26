//! Tokio-driven loopback RTSP server fixture.
//!
//! Binds 127.0.0.1:0 (kernel picks port); speaks RTSP/1.0 by default
//! (RTSP/2.0 if client sends 2.0). Supports both UDP and
//! TCP-interleaved transports. Configurable to demand `Basic`, `MD5
//! Digest`, `SHA-256 Digest`, or no auth. Configurable to return
//! `461 Unsupported Transport` on first UDP SETUP (forces TCP
//! fallback).
//!
//! Returns a `FixtureHandle` that exposes:
//! - `port()` — the TCP port to put in `rtsp://127.0.0.1:<port>/test`
//! - `set_auth_mode(...)` — configure auth requirement
//! - `force_461_on_udp(true)` — drop the first UDP SETUP with 461
//! - `shutdown()` — kill the server thread

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMode {
    None,
    Basic,
    DigestMd5,
    DigestSha256,
}

pub struct FixtureConfig {
    pub auth: AuthMode,
    pub force_461_on_udp: bool,
    pub username: String,
    pub password: String,
    pub sdp_body: Vec<u8>,
}

impl Default for FixtureConfig {
    fn default() -> Self {
        Self {
            auth: AuthMode::None,
            force_461_on_udp: false,
            username: "admin".into(),
            password: "secret".into(),
            sdp_body: br#"v=0
o=- 0 0 IN IP4 127.0.0.1
s=tst-rtp test
t=0 0
m=application 0 RTP/AVP 33
a=control:trackID=0
"#
            .to_vec(),
        }
    }
}

pub struct FixtureHandle {
    pub port: u16,
    shutdown: Arc<AtomicBool>,
    runtime: Option<tokio::runtime::Runtime>,
}

impl FixtureHandle {
    pub fn spawn(cfg: FixtureConfig) -> Self {
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let cfg_arc = Arc::new(Mutex::new(cfg));
        let (port_tx, port_rx) = std::sync::mpsc::sync_channel(1);
        runtime.spawn(async move {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            port_tx.send(listener.local_addr().unwrap().port()).unwrap();
            loop {
                if shutdown_clone.load(Ordering::Relaxed) {
                    break;
                }
                tokio::select! {
                    accept_res = listener.accept() => {
                        match accept_res {
                            Ok((sock, peer)) => {
                                let cfg = cfg_arc.lock().unwrap().clone();
                                tokio::spawn(handle_client(sock, peer, cfg, shutdown_clone.clone()));
                            }
                            Err(_) => break,
                        }
                    }
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                        // Wake to check shutdown flag
                    }
                }
            }
        });
        let port = port_rx.recv().unwrap();
        Self {
            port,
            shutdown,
            runtime: Some(runtime),
        }
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }
}

impl Drop for FixtureHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(rt) = self.runtime.take() {
            rt.shutdown_timeout(std::time::Duration::from_secs(2));
        }
    }
}

impl Clone for FixtureConfig {
    fn clone(&self) -> Self {
        Self {
            auth: self.auth,
            force_461_on_udp: self.force_461_on_udp,
            username: self.username.clone(),
            password: self.password.clone(),
            sdp_body: self.sdp_body.clone(),
        }
    }
}

async fn handle_client(
    mut sock: TcpStream,
    _peer: SocketAddr,
    cfg: FixtureConfig,
    shutdown: Arc<AtomicBool>,
) {
    let mut buf = vec![0u8; 8192];
    let mut accumulator = Vec::new();
    let mut auth_passed = matches!(cfg.auth, AuthMode::None);
    let mut session_id = String::new();
    let mut udp_setup_attempts = 0;
    loop {
        if shutdown.load(Ordering::Relaxed) {
            return;
        }
        let n = match sock.read(&mut buf).await {
            Ok(0) => return,
            Ok(n) => n,
            Err(_) => return,
        };
        accumulator.extend_from_slice(&buf[..n]);

        // Parse one RTSP message from accumulator
        let end = match find_message_end(&accumulator) {
            Some(end) => end,
            None => continue,
        };
        let request = String::from_utf8_lossy(&accumulator[..end]).into_owned();
        accumulator.drain(..end);

        let method = request.split(' ').next().unwrap_or("").to_string();
        let cseq = extract_header(&request, "CSeq").unwrap_or_else(|| "0".to_string());
        // Auth check
        if !auth_passed && method != "OPTIONS" {
            if let Some(auth_header) = extract_header(&request, "Authorization") {
                auth_passed = validate_auth(&cfg, &method, &auth_header);
            }
            if !auth_passed {
                let challenge = match cfg.auth {
                    AuthMode::Basic => r#"Basic realm="test""#.to_string(),
                    AuthMode::DigestMd5 => {
                        r#"Digest realm="test", nonce="abc123", algorithm=MD5"#.to_string()
                    }
                    AuthMode::DigestSha256 => {
                        r#"Digest realm="test", nonce="abc123", algorithm=SHA-256, qop="auth""#
                            .to_string()
                    }
                    AuthMode::None => unreachable!(),
                };
                let _ = sock
                    .write_all(
                        format!(
                            "RTSP/1.0 401 Unauthorized\r\nCSeq: {}\r\nWWW-Authenticate: {}\r\n\r\n",
                            cseq, challenge,
                        )
                        .as_bytes(),
                    )
                    .await;
                continue;
            }
        }

        // Route by method
        match method.as_str() {
            "OPTIONS" => {
                let _ = sock.write_all(format!(
                    "RTSP/1.0 200 OK\r\nCSeq: {}\r\nPublic: OPTIONS, DESCRIBE, SETUP, PLAY, PAUSE, TEARDOWN\r\n\r\n",
                    cseq,
                ).as_bytes()).await;
            }
            "DESCRIBE" => {
                let _ = sock.write_all(format!(
                    "RTSP/1.0 200 OK\r\nCSeq: {}\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n",
                    cseq, cfg.sdp_body.len(),
                ).as_bytes()).await;
                let _ = sock.write_all(&cfg.sdp_body).await;
            }
            "SETUP" => {
                let transport = extract_header(&request, "Transport").unwrap_or_default();
                let is_udp = transport.contains("RTP/AVP;") && !transport.contains("/TCP");
                if is_udp && cfg.force_461_on_udp && udp_setup_attempts == 0 {
                    udp_setup_attempts += 1;
                    let _ = sock
                        .write_all(
                            format!(
                                "RTSP/1.0 461 Unsupported Transport\r\nCSeq: {}\r\n\r\n",
                                cseq,
                            )
                            .as_bytes(),
                        )
                        .await;
                    continue;
                }
                session_id = format!("{:08X}", rand_session_id());
                let resp_transport = if is_udp {
                    let client_port = extract_client_port(&transport).unwrap_or(5004);
                    format!(
                        "RTP/AVP;unicast;client_port={}-{};server_port=6970-6971",
                        client_port,
                        client_port + 1
                    )
                } else {
                    "RTP/AVP/TCP;unicast;interleaved=0-1".to_string()
                };
                let _ = sock.write_all(format!(
                    "RTSP/1.0 200 OK\r\nCSeq: {}\r\nSession: {};timeout=60\r\nTransport: {}\r\n\r\n",
                    cseq, session_id, resp_transport,
                ).as_bytes()).await;
            }
            "PLAY" => {
                let _ = sock.write_all(format!(
                    "RTSP/1.0 200 OK\r\nCSeq: {}\r\nSession: {}\r\nRTP-Info: url=rtsp://127.0.0.1/test/streamid=0;seq=1234;rtptime=5000000\r\n\r\n",
                    cseq, session_id,
                ).as_bytes()).await;
            }
            "PAUSE" => {
                let _ = sock
                    .write_all(
                        format!(
                            "RTSP/1.0 200 OK\r\nCSeq: {}\r\nSession: {}\r\n\r\n",
                            cseq, session_id,
                        )
                        .as_bytes(),
                    )
                    .await;
            }
            "TEARDOWN" => {
                let _ = sock
                    .write_all(
                        format!(
                            "RTSP/1.0 200 OK\r\nCSeq: {}\r\nSession: {}\r\n\r\n",
                            cseq, session_id,
                        )
                        .as_bytes(),
                    )
                    .await;
                return;
            }
            _ => {
                let _ = sock
                    .write_all(
                        format!("RTSP/1.0 501 Not Implemented\r\nCSeq: {}\r\n\r\n", cseq,)
                            .as_bytes(),
                    )
                    .await;
            }
        }
    }
}

fn find_message_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

fn extract_header(req: &str, name: &str) -> Option<String> {
    let lname = name.to_ascii_lowercase();
    for line in req.split("\r\n") {
        if let Some(colon) = line.find(':') {
            if line[..colon].trim().to_ascii_lowercase() == lname {
                return Some(line[colon + 1..].trim().to_string());
            }
        }
    }
    None
}

fn extract_client_port(transport: &str) -> Option<u16> {
    transport
        .split(';')
        .find_map(|p| p.trim().strip_prefix("client_port="))
        .and_then(|v| v.split('-').next())
        .and_then(|s| s.parse().ok())
}

fn validate_auth(cfg: &FixtureConfig, _method: &str, header: &str) -> bool {
    match cfg.auth {
        AuthMode::Basic => {
            // Compare base64-encoded user:pass
            use base64::Engine;
            let expected = base64::engine::general_purpose::STANDARD
                .encode(format!("{}:{}", cfg.username, cfg.password));
            header.contains(&expected)
        }
        AuthMode::DigestMd5 | AuthMode::DigestSha256 => {
            // Lax check — just look for username= in the header
            header.contains(&format!("username=\"{}\"", cfg.username))
        }
        AuthMode::None => true,
    }
}

fn rand_session_id() -> u32 {
    let mut bytes = [0u8; 4];
    getrandom::getrandom(&mut bytes).unwrap();
    u32::from_le_bytes(bytes)
}
