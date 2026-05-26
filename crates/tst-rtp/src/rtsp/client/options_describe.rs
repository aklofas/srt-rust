//! `RtspClient::options` + `RtspClient::describe`. Single-request
//! synchronous send + recv against the control connection.

use std::collections::HashMap;
use std::io::{Read, Write};

use crate::error::RtspError;
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
        self.stream
            .write_all(&bytes)
            .map_err(|e| RtspError::Io(e.kind()))?;
        let resp = self.read_response()?;
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
        let cseq = self.bump_cseq();
        let req = RtspRequest::new(
            RtspMethod::Describe,
            self.url.render_no_credentials(),
            self.url.rtsp_version,
        )
        .header("cseq", cseq.to_string())
        .header("accept", "application/sdp")
        .header("user-agent", "tst-rtp/0.1");
        let bytes = req.encode();
        self.stream
            .write_all(&bytes)
            .map_err(|e| RtspError::Io(e.kind()))?;
        let resp = self.read_response()?;
        self.last_server_version = resp.version;
        if resp.status == 401 {
            // Hand-off to auth-retry path lands in a later task; here we
            // just surface a clean error for now.
            return Err(RtspError::AuthFailed);
        }
        if resp.status != 200 {
            return Err(RtspError::Protocol {
                code: resp.status,
                reason: resp.reason,
            });
        }
        Sdp::parse(&resp.body)
    }

    /// Read one complete RTSP response from the stream. Honors the
    /// cancel flag by checking it between read attempts.
    ///
    /// The TCP stream is set up in
    /// [`RtspClient::connect_with`] with a short read timeout (100 ms),
    /// so each `WouldBlock` / `TimedOut` round-trip is a cancel-check
    /// opportunity.
    pub(crate) fn read_response(&mut self) -> Result<RtspResponse, RtspError> {
        let mut buf = Vec::with_capacity(4096);
        let mut chunk = [0u8; 4096];
        loop {
            if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(RtspError::LocalCancel);
            }
            match self.stream.read(&mut chunk) {
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
