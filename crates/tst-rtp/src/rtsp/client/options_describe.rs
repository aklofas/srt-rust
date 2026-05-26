//! `RtspClient::options` + `RtspClient::describe`. Single-request
//! synchronous send + recv against the control connection.

use std::collections::HashMap;
use std::io::{Read, Write};

use secrecy::ExposeSecret;

use crate::error::RtspError;
use crate::rtsp::auth::{
    AuthChallenge, DigestContext, build_basic_response, build_digest_response, parse_challenges,
};
use crate::rtsp::client::RtspClient;
use crate::rtsp::message::{RtspMethod, RtspRequest, RtspResponse};
use crate::sdp::Sdp;

/// Response shape returned by [`RtspClient::options`]. Exposes the
/// server's advertised methods (parsed from the `Public:` header) plus
/// the raw header map for callers that need to look at other fields.
#[derive(Debug, Clone)]
pub struct OptionsResponse {
    /// Methods advertised in the `Public:` header, split on `,` and
    /// trimmed. Empty if the server didn't return the header.
    pub public_methods: Vec<String>,
    /// All response headers (lowercase keys), for callers needing
    /// e.g. `Server:` or `Date:`.
    pub headers: HashMap<String, String>,
}

impl RtspClient {
    /// Send an OPTIONS request; parse the `Public:` header into the
    /// list of methods the server supports.
    ///
    /// # Errors
    ///
    /// - [`RtspError::Io`] on socket-level failure.
    /// - [`RtspError::BadResponse`] on malformed response bytes.
    /// - [`RtspError::Protocol`] if the server returns non-200.
    /// - [`RtspError::LocalCancel`] if the cancel handle was triggered
    ///   mid-read.
    pub fn options(&mut self) -> Result<OptionsResponse, RtspError> {
        let cseq = self.bump_cseq();
        let mut req = RtspRequest::new(
            RtspMethod::Options,
            self.url.render_no_credentials(),
            self.url.rtsp_version,
        )
        .header("cseq", cseq.to_string())
        .header("user-agent", "tst-rtp/0.1");
        if let Some(sid) = &self.session_id {
            req = req.header("session", sid.clone());
        }
        let bytes = req.encode();
        let resp = self.send_and_read(&bytes)?;
        self.last_server_version = resp.version;
        if resp.status != 200 {
            return Err(RtspError::Protocol {
                code: resp.status,
                reason: resp.reason,
            });
        }
        let public_methods = resp
            .headers
            .get("public")
            .map(|s| s.split(',').map(|p| p.trim().to_string()).collect())
            .unwrap_or_default();
        Ok(OptionsResponse {
            public_methods,
            headers: resp.headers,
        })
    }

    /// Send a DESCRIBE request and parse the SDP body.
    ///
    /// # Errors
    ///
    /// - [`RtspError::Io`] on socket-level failure.
    /// - [`RtspError::BadResponse`] on malformed response bytes.
    /// - [`RtspError::AuthFailed`] on a 401 response (the full
    ///   challenge-retry path lands in a later task; for now we
    ///   surface a clean error).
    /// - [`RtspError::Protocol`] on any other non-200 status.
    /// - [`RtspError::BadSdp`] if the response body isn't parseable SDP.
    /// - [`RtspError::LocalCancel`] if the cancel handle was triggered
    ///   mid-read.
    pub fn describe(&mut self) -> Result<Sdp, RtspError> {
        // First attempt: no Authorization header.
        let resp = self.send_request_with_optional_auth(RtspMethod::Describe, None)?;
        let resp = if resp.status == 401 {
            // Server demanded auth — extract challenge, build credentials, retry.
            self.handle_auth_challenge_and_retry(RtspMethod::Describe, &resp)?
        } else {
            resp
        };
        self.last_server_version = resp.version;
        if resp.status != 200 {
            return Err(RtspError::Protocol {
                code: resp.status,
                reason: resp.reason,
            });
        }
        Sdp::parse(&resp.body)
    }

    /// Send a request without (or with) a pre-built Authorization
    /// header. Used as the first attempt + the retry leg of the
    /// challenge-response flow.
    pub(crate) fn send_request_with_optional_auth(
        &mut self,
        method: RtspMethod,
        authorization: Option<String>,
    ) -> Result<RtspResponse, RtspError> {
        let cseq = self.bump_cseq();
        let mut req = RtspRequest::new(
            method,
            self.url.render_no_credentials(),
            self.url.rtsp_version,
        )
        .header("cseq", cseq.to_string())
        .header("accept", "application/sdp")
        .header("user-agent", "tst-rtp/0.1");
        if let Some(sid) = &self.session_id {
            req = req.header("session", sid.clone());
        }
        if let Some(auth) = authorization {
            req = req.header("authorization", auth);
        }
        let bytes = req.encode();
        self.send_and_read(&bytes)
    }

    /// Parse WWW-Authenticate from a 401 response, build Authorization
    /// from URL credentials, retry. Returns the retry response.
    pub(crate) fn handle_auth_challenge_and_retry(
        &mut self,
        method: RtspMethod,
        first_resp: &RtspResponse,
    ) -> Result<RtspResponse, RtspError> {
        let username = self.url.username.clone().ok_or(RtspError::AuthFailed)?;
        let password = self.url.password.clone().ok_or(RtspError::AuthFailed)?;
        let www_auth =
            first_resp
                .headers
                .get("www-authenticate")
                .ok_or(RtspError::BadResponse {
                    detail: "401 without WWW-Authenticate header",
                })?;
        let challenges = parse_challenges(www_auth);
        // Prefer Digest over Basic when both are offered.
        let challenge = challenges
            .iter()
            .find(|c| matches!(c, AuthChallenge::Digest(_)))
            .or_else(|| {
                challenges
                    .iter()
                    .find(|c| matches!(c, AuthChallenge::Basic { .. }))
            })
            .ok_or_else(|| RtspError::AuthUnsupported {
                scheme: "(no recognized scheme in WWW-Authenticate)".into(),
            })?;

        let method_str = match method {
            RtspMethod::Options => "OPTIONS",
            RtspMethod::Describe => "DESCRIBE",
            RtspMethod::Setup => "SETUP",
            RtspMethod::Play => "PLAY",
            RtspMethod::Pause => "PAUSE",
            RtspMethod::Teardown => "TEARDOWN",
            RtspMethod::GetParameter => "GET_PARAMETER",
        };
        let uri = self.url.render_no_credentials();
        let authorization = match challenge {
            AuthChallenge::Basic { .. } => build_basic_response(&username, &password),
            AuthChallenge::Digest(d) => {
                // Generate a random cnonce.
                let mut cnonce_bytes = [0u8; 16];
                getrandom::getrandom(&mut cnonce_bytes)
                    .map_err(|_| RtspError::Io(std::io::ErrorKind::Other))?;
                let mut cnonce = String::with_capacity(32);
                for b in &cnonce_bytes {
                    use std::fmt::Write as _;
                    let _ = write!(cnonce, "{:02x}", b);
                }
                let _ = password.expose_secret(); // touch to ensure non-zero
                let ctx = DigestContext {
                    username: &username,
                    password: &password,
                    method: method_str,
                    uri: &uri,
                    nc: 1,
                    cnonce: &cnonce,
                    challenge: d,
                };
                build_digest_response(&ctx)
            }
        };

        let retry = self.send_request_with_optional_auth(method, Some(authorization))?;
        if retry.status == 401 {
            return Err(RtspError::AuthFailed);
        }
        Ok(retry)
    }

    /// Write a serialized RTSP request, then read one complete response.
    ///
    /// Two read paths:
    ///
    /// 1. **Pump inactive** (UDP transport or pre-SETUP): writes + reads
    ///    happen under a single stream-mutex acquisition. The cancel
    ///    flag is checked between read polls; the underlying stream has
    ///    a short read timeout (100 ms, set in
    ///    [`RtspClient::connect_with`]), so each
    ///    `WouldBlock`/`TimedOut` round-trip is a cancel-check
    ///    opportunity. Holding the lock through the whole exchange
    ///    means the keepalive thread waits if a request is in flight —
    ///    correct since RTSP isn't pipelined.
    ///
    /// 2. **Pump active** (TCP-interleaved post-SETUP): writes happen
    ///    under the stream mutex (briefly, then released). The response
    ///    is read from `pump_state.ctrl_rx` matched by CSeq, since the
    ///    background pump thread owns reads in this mode (reading the
    ///    stream directly here would race with the pump). Responses
    ///    with CSeq >= 1_000_000 are silently discarded — those are
    ///    keepalive-thread OPTIONS responses (see
    ///    `keepalive::spawn`'s `cseq = 1_000_000u32` starting value).
    pub(crate) fn send_and_read(
        &mut self,
        request_bytes: &[u8],
    ) -> Result<RtspResponse, RtspError> {
        if self.pump_state.is_some() {
            return self.send_and_read_via_pump(request_bytes);
        }
        let mut s = self.stream.lock().expect("stream mutex poisoned");
        s.write_all(request_bytes)
            .map_err(|e| RtspError::Io(e.kind()))?;
        let mut buf = Vec::with_capacity(4096);
        let mut chunk = [0u8; 4096];
        loop {
            if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(RtspError::LocalCancel);
            }
            match s.read(&mut chunk) {
                Ok(0) => return Err(RtspError::Io(std::io::ErrorKind::UnexpectedEof)),
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if let Ok((resp, _consumed)) = RtspResponse::parse(&buf) {
                        return Ok(resp);
                    }
                    // not enough bytes yet; loop
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    // read timeout; check cancel + retry
                    continue;
                }
                Err(e) => return Err(RtspError::Io(e.kind())),
            }
        }
    }

    /// Pump-active variant of [`Self::send_and_read`]. Write under the
    /// stream mutex (brief), then poll `ctrl_rx` matching by CSeq.
    fn send_and_read_via_pump(&mut self, request_bytes: &[u8]) -> Result<RtspResponse, RtspError> {
        // Parse the outbound request's CSeq so we can match it on the
        // way back. Cheap — request_bytes is small.
        let req_cseq = parse_cseq_from_request(request_bytes);
        // Write under the mutex; release immediately.
        {
            let mut s = self.stream.lock().expect("stream mutex poisoned");
            s.write_all(request_bytes)
                .map_err(|e| RtspError::Io(e.kind()))?;
        }
        // Now poll ctrl_rx. Use a short timeout so we can re-check the
        // cancel flag.
        let pump = self
            .pump_state
            .as_ref()
            .expect("pump_state is Some — checked by caller");
        loop {
            if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(RtspError::LocalCancel);
            }
            match pump
                .ctrl_rx
                .recv_timeout(std::time::Duration::from_millis(100))
            {
                Ok(msg_bytes) => {
                    let (resp, _consumed) = match RtspResponse::parse(&msg_bytes) {
                        Ok(p) => p,
                        Err(_) => continue, // malformed; drop + keep polling
                    };
                    // Discard keepalive-thread responses (CSeq >= 1_000_000)
                    // and any other response whose CSeq doesn't match
                    // the request we just sent.
                    match (req_cseq, resp.cseq()) {
                        (Some(req), Some(got)) if req == got => return Ok(resp),
                        _ => continue,
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    // Pump exited (EOF or fatal read error).
                    return Err(RtspError::Io(std::io::ErrorKind::UnexpectedEof));
                }
            }
        }
    }
}

/// Best-effort scan for the `CSeq:` header in a serialized request.
/// Returns the parsed integer when found. Used only by the pump-active
/// read path to match responses by CSeq.
fn parse_cseq_from_request(bytes: &[u8]) -> Option<u32> {
    // Only look at the header section (up to the first CRLFCRLF).
    let end = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or(bytes.len());
    let header_text = std::str::from_utf8(&bytes[..end]).ok()?;
    for line in header_text.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("cseq:") {
            return v.trim().parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;

    /// Spawn a one-shot mock RTSP server that serves a single response.
    fn mock_server(canned_response: &'static [u8]) -> (u16, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            if let Ok((mut sock, _)) = listener.accept() {
                // Read whatever the client sent (ignore content)
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf);
                sock.write_all(canned_response).unwrap();
            }
        });
        (port, h)
    }

    #[test]
    fn options_parses_public_header() {
        let (port, h) = mock_server(
            b"RTSP/1.0 200 OK\r\nCSeq: 1\r\nPublic: OPTIONS, DESCRIBE, SETUP, PLAY, TEARDOWN\r\n\r\n",
        );
        let mut client = RtspClient::connect(&format!("rtsp://127.0.0.1:{}/test", port)).unwrap();
        let opts = client.options().unwrap();
        assert!(opts.public_methods.contains(&"DESCRIBE".to_string()));
        assert!(opts.public_methods.contains(&"PLAY".to_string()));
        h.join().unwrap();
    }

    #[test]
    fn describe_parses_sdp() {
        let body =
            b"v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=test\r\nt=0 0\r\nm=application 0 RTP/AVP 33\r\n";
        let resp = format!(
            "RTSP/1.0 200 OK\r\nCSeq: 1\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let mut full = resp.into_bytes();
        full.extend_from_slice(body);
        let leaked: &'static [u8] = Box::leak(full.into_boxed_slice());
        let (port, h) = mock_server(leaked);
        let mut client = RtspClient::connect(&format!("rtsp://127.0.0.1:{}/test", port)).unwrap();
        let sdp = client.describe().unwrap();
        assert_eq!(sdp.media.len(), 1);
        assert_eq!(sdp.media[0].payload_types, vec![33]);
        h.join().unwrap();
    }
}
