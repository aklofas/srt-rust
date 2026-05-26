//! Wrapper around the `rtsp-types` crate for RTSP wire format
//! parse/serialize, scoped to the subset we use:
//! OPTIONS / DESCRIBE / SETUP / PLAY / PAUSE / TEARDOWN.
//!
//! Note: we use the `rtsp-types` crate for fuzz-harness reference parsing
//! (Task 24) and as a sanity-check oracle in unit tests. Production parse
//! is the hand-rolled [`RtspResponse::parse`] below, which is simpler and
//! has fewer transitive deps. If real-camera quirks surface that our
//! parser doesn't handle, swap to `rtsp_types::Parser::next_message()`
//! per the comment in [`RtspResponse::parse`].

use std::collections::HashMap;

use bytes::Bytes;

use crate::error::RtspError;
use crate::url::RtspVersion;

/// An RTSP request we construct on the client side.
///
/// Wraps a method + URI + version + headers + body and serializes to the
/// canonical CRLF-terminated wire format via [`RtspRequest::encode`]. The
/// headers we always set on production traffic (CSeq, User-Agent,
/// Session-if-present, Authorization-if-present) are added by the caller
/// via [`RtspRequest::header`] rather than baked in here, so this type
/// stays a pure data carrier.
#[derive(Debug, Clone)]
pub struct RtspRequest {
    pub method: RtspMethod,
    pub uri: String,
    pub version: RtspVersion,
    pub headers: HashMap<String, String>,
    pub body: Bytes,
}

/// An RTSP method we send. Bound to the subset we implement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RtspMethod {
    Options,
    Describe,
    Setup,
    Play,
    Pause,
    Teardown,
    GetParameter,
}

/// An RTSP response we receive on the client side.
#[derive(Debug, Clone)]
pub struct RtspResponse {
    pub version: RtspVersion,
    pub status: u16,
    pub reason: String,
    pub headers: HashMap<String, String>,
    pub body: Bytes,
}

impl RtspRequest {
    pub fn new(method: RtspMethod, uri: impl Into<String>, version: RtspVersion) -> Self {
        Self {
            method,
            uri: uri.into(),
            version,
            headers: HashMap::new(),
            body: Bytes::new(),
        }
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.insert(name.to_ascii_lowercase(), value.into());
        self
    }

    pub fn body(mut self, body: Bytes) -> Self {
        self.body = body;
        self
    }

    /// Serialize to bytes ready to write to the TCP stream.
    pub fn encode(&self) -> Bytes {
        let method_str = match self.method {
            RtspMethod::Options => "OPTIONS",
            RtspMethod::Describe => "DESCRIBE",
            RtspMethod::Setup => "SETUP",
            RtspMethod::Play => "PLAY",
            RtspMethod::Pause => "PAUSE",
            RtspMethod::Teardown => "TEARDOWN",
            RtspMethod::GetParameter => "GET_PARAMETER",
        };
        let mut out = Vec::with_capacity(256 + self.body.len());
        out.extend_from_slice(method_str.as_bytes());
        out.push(b' ');
        out.extend_from_slice(self.uri.as_bytes());
        out.push(b' ');
        out.extend_from_slice(self.version.wire_str().as_bytes());
        out.extend_from_slice(b"\r\n");
        for (k, v) in &self.headers {
            // Capitalize first letter of each dash-separated word for canonicality.
            for (i, segment) in k.split('-').enumerate() {
                if i > 0 {
                    out.push(b'-');
                }
                let mut chars = segment.chars();
                if let Some(c) = chars.next() {
                    for ch in c.to_uppercase() {
                        out.extend_from_slice(ch.to_string().as_bytes());
                    }
                    for ch in chars {
                        out.extend_from_slice(ch.to_string().as_bytes());
                    }
                }
            }
            out.extend_from_slice(b": ");
            out.extend_from_slice(v.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        if !self.body.is_empty() && !self.headers.contains_key("content-length") {
            out.extend_from_slice(b"Content-Length: ");
            out.extend_from_slice(self.body.len().to_string().as_bytes());
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(&self.body);
        Bytes::from(out)
    }
}

impl RtspResponse {
    /// Parse an RTSP response from a byte slice. Returns the parsed
    /// response plus the number of bytes consumed, or
    /// [`RtspError::BadResponse`] if the bytes don't form a complete
    /// well-formed response.
    ///
    /// Hand-rolled parser, scoped to the OPTIONS / DESCRIBE / SETUP /
    /// PLAY / PAUSE / TEARDOWN subset. If real-camera quirks surface,
    /// swap to `rtsp_types::Parser::next_message()`.
    pub fn parse(input: &[u8]) -> Result<(Self, usize), RtspError> {
        // Find end of status line + headers (CRLFCRLF).
        let header_end =
            input
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .ok_or(RtspError::BadResponse {
                    detail: "no header terminator",
                })?;
        let header_text =
            std::str::from_utf8(&input[..header_end]).map_err(|_| RtspError::BadResponse {
                detail: "non-UTF8 headers",
            })?;
        let mut lines = header_text.split("\r\n");
        let status_line = lines.next().ok_or(RtspError::BadResponse {
            detail: "empty status line",
        })?;

        let mut parts = status_line.splitn(3, ' ');
        let version_str = parts.next().ok_or(RtspError::BadResponse {
            detail: "missing version",
        })?;
        let code_str = parts.next().ok_or(RtspError::BadResponse {
            detail: "missing status code",
        })?;
        let reason = parts.next().unwrap_or("").to_string();
        let version = match version_str {
            "RTSP/1.0" => RtspVersion::V1_0,
            "RTSP/2.0" => RtspVersion::V2_0,
            _ => {
                return Err(RtspError::BadResponse {
                    detail: "unrecognized RTSP version",
                });
            }
        };
        let status = code_str
            .parse::<u16>()
            .map_err(|_| RtspError::BadResponse {
                detail: "non-numeric status code",
            })?;

        let mut headers: HashMap<String, String> = HashMap::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let colon = line.find(':').ok_or(RtspError::BadResponse {
                detail: "missing header colon",
            })?;
            let k = line[..colon].trim().to_ascii_lowercase();
            let v = line[colon + 1..].trim().to_string();
            headers.insert(k, v);
        }

        let content_length: usize = headers
            .get("content-length")
            .map(|s| s.parse().unwrap_or(0))
            .unwrap_or(0);
        let body_start = header_end + 4;
        if input.len() < body_start + content_length {
            return Err(RtspError::BadResponse {
                detail: "truncated body (Content-Length larger than available)",
            });
        }
        let body = Bytes::copy_from_slice(&input[body_start..body_start + content_length]);
        Ok((
            Self {
                version,
                status,
                reason,
                headers,
                body,
            },
            body_start + content_length,
        ))
    }

    pub fn cseq(&self) -> Option<u32> {
        self.headers.get("cseq")?.trim().parse().ok()
    }

    pub fn session_id(&self) -> Option<&str> {
        let raw = self.headers.get("session")?;
        // Format: 12345678;timeout=60 — strip the ;timeout= suffix.
        Some(raw.split(';').next().unwrap().trim())
    }

    pub fn session_timeout_secs(&self) -> Option<u64> {
        let raw = self.headers.get("session")?;
        for part in raw.split(';').skip(1) {
            let part = part.trim();
            if let Some(v) = part.strip_prefix("timeout=") {
                return v.parse().ok();
            }
        }
        None
    }

    /// Serialize the response to wire bytes ready to write to the TCP
    /// stream. Mirrors [`RtspRequest::encode`] on the server side.
    ///
    /// Emits the status line, then headers (one per CRLF line, with the
    /// header name title-cased dash-segment-wise for cosmetic
    /// compatibility — RFC 7826 §5.1.1 declares headers case-insensitive
    /// but some IP cameras do not honor that), then a blank line, then
    /// the body (if any). Content-Length is *not* auto-injected — Task
    /// 10's DESCRIBE handler sets it explicitly when emitting an SDP
    /// body.
    pub fn encode(&self) -> Bytes {
        let mut buf = Vec::with_capacity(256 + self.body.len());
        // Status line.
        buf.extend_from_slice(self.version.wire_str().as_bytes());
        buf.push(b' ');
        buf.extend_from_slice(self.status.to_string().as_bytes());
        buf.push(b' ');
        buf.extend_from_slice(self.reason.as_bytes());
        buf.extend_from_slice(b"\r\n");
        // Headers.
        for (k, v) in &self.headers {
            for (i, segment) in k.split('-').enumerate() {
                if i > 0 {
                    buf.push(b'-');
                }
                let mut chars = segment.chars();
                if let Some(c) = chars.next() {
                    for ch in c.to_uppercase() {
                        buf.extend_from_slice(ch.to_string().as_bytes());
                    }
                    for ch in chars {
                        buf.extend_from_slice(ch.to_string().as_bytes());
                    }
                }
            }
            buf.extend_from_slice(b": ");
            buf.extend_from_slice(v.as_bytes());
            buf.extend_from_slice(b"\r\n");
        }
        buf.extend_from_slice(b"\r\n");
        if !self.body.is_empty() {
            buf.extend_from_slice(&self.body);
        }
        Bytes::from(buf)
    }
}

impl RtspRequest {
    /// Parse an RTSP request from wire bytes. Returns the parsed request
    /// plus the number of bytes consumed (including any Content-Length
    /// body). Mirrors [`RtspResponse::parse`] on the server side.
    ///
    /// # Errors
    /// - [`RtspError::BadResponse`] (reused for request-side malformed
    ///   text; we don't have a separate `BadRequest` variant in v1).
    pub fn parse(input: &[u8]) -> Result<(Self, usize), RtspError> {
        // 1. Find CRLFCRLF terminating headers.
        let header_end =
            input
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .ok_or(RtspError::BadResponse {
                    detail: "no CRLFCRLF terminating request headers",
                })?;
        let header_bytes = &input[..header_end];
        let header_text =
            std::str::from_utf8(header_bytes).map_err(|_| RtspError::BadResponse {
                detail: "non-UTF8 RTSP request",
            })?;

        // 2. Parse request line: "METHOD URI VERSION".
        let mut lines = header_text.split("\r\n");
        let request_line = lines.next().ok_or(RtspError::BadResponse {
            detail: "empty request",
        })?;
        let mut parts = request_line.splitn(3, ' ');
        let method_str = parts.next().ok_or(RtspError::BadResponse {
            detail: "no method in request line",
        })?;
        let uri = parts
            .next()
            .ok_or(RtspError::BadResponse {
                detail: "no URI in request line",
            })?
            .to_string();
        let version_str = parts.next().ok_or(RtspError::BadResponse {
            detail: "no version in request line",
        })?;

        let method = match method_str {
            "OPTIONS" => RtspMethod::Options,
            "DESCRIBE" => RtspMethod::Describe,
            "SETUP" => RtspMethod::Setup,
            "PLAY" => RtspMethod::Play,
            "PAUSE" => RtspMethod::Pause,
            "TEARDOWN" => RtspMethod::Teardown,
            "GET_PARAMETER" => RtspMethod::GetParameter,
            _ => {
                return Err(RtspError::BadResponse {
                    detail: "unsupported RTSP method",
                });
            }
        };
        let version = match version_str {
            "RTSP/1.0" => RtspVersion::V1_0,
            "RTSP/2.0" => RtspVersion::V2_0,
            _ => {
                return Err(RtspError::BadResponse {
                    detail: "unsupported RTSP version",
                });
            }
        };

        // 3. Parse headers.
        let mut headers: HashMap<String, String> = HashMap::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let colon = line.find(':').ok_or(RtspError::BadResponse {
                detail: "malformed header line",
            })?;
            let name = line[..colon].trim().to_ascii_lowercase();
            let value = line[colon + 1..].trim().to_string();
            headers.insert(name, value);
        }

        // 4. Read body per Content-Length, if any.
        let content_length: usize = headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let body_start = header_end + 4;
        let body_end = body_start + content_length;
        if input.len() < body_end {
            return Err(RtspError::BadResponse {
                detail: "truncated body",
            });
        }
        let body = Bytes::copy_from_slice(&input[body_start..body_end]);

        Ok((
            Self {
                method,
                uri,
                version,
                headers,
                body,
            },
            body_end,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_options_request() {
        let req = RtspRequest::new(RtspMethod::Options, "rtsp://cam/h264", RtspVersion::V1_0)
            .header("cseq", "1")
            .header("user-agent", "tst-rtp/0.1");
        let bytes = req.encode();
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(s.starts_with("OPTIONS rtsp://cam/h264 RTSP/1.0\r\n"));
        assert!(s.contains("Cseq: 1\r\n") || s.contains("CSeq: 1\r\n"));
        assert!(s.ends_with("\r\n\r\n"));
    }

    #[test]
    fn parse_200_options_response() {
        let raw = b"RTSP/1.0 200 OK\r\nCSeq: 1\r\nPublic: OPTIONS, DESCRIBE, SETUP, PLAY, TEARDOWN\r\n\r\n";
        let (resp, consumed) = RtspResponse::parse(raw).unwrap();
        assert_eq!(consumed, raw.len());
        assert_eq!(resp.status, 200);
        assert_eq!(resp.reason, "OK");
        assert_eq!(resp.version, RtspVersion::V1_0);
        assert_eq!(resp.cseq(), Some(1));
        assert_eq!(
            resp.headers.get("public").unwrap(),
            "OPTIONS, DESCRIBE, SETUP, PLAY, TEARDOWN"
        );
    }

    #[test]
    fn parse_session_id_and_timeout() {
        let raw = b"RTSP/1.0 200 OK\r\nCSeq: 3\r\nSession: 12345678;timeout=60\r\n\r\n";
        let (resp, _) = RtspResponse::parse(raw).unwrap();
        assert_eq!(resp.session_id(), Some("12345678"));
        assert_eq!(resp.session_timeout_secs(), Some(60));
    }

    #[test]
    fn parse_response_with_body() {
        let body = b"v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=test\r\n";
        let mut raw = format!(
            "RTSP/1.0 200 OK\r\nCSeq: 2\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        raw.extend_from_slice(body);
        let (resp, consumed) = RtspResponse::parse(&raw).unwrap();
        assert_eq!(consumed, raw.len());
        assert_eq!(resp.body.as_ref(), body);
    }

    #[test]
    fn parse_truncated_body_errors() {
        let raw = b"RTSP/1.0 200 OK\r\nContent-Length: 100\r\n\r\nshort";
        let e = RtspResponse::parse(raw).unwrap_err();
        assert!(matches!(e, RtspError::BadResponse { .. }));
    }

    #[test]
    fn parse_rejects_unknown_version() {
        let raw = b"RTSP/3.0 200 OK\r\nCSeq: 1\r\n\r\n";
        let e = RtspResponse::parse(raw).unwrap_err();
        assert!(matches!(e, RtspError::BadResponse { .. }));
    }
}

#[cfg(test)]
mod request_parse_tests {
    use super::*;

    #[test]
    fn parse_options_request() {
        let raw = b"OPTIONS rtsp://server/live RTSP/1.0\r\n\
                    CSeq: 1\r\n\
                    User-Agent: test\r\n\
                    \r\n";
        let (req, n) = RtspRequest::parse(raw).unwrap();
        assert_eq!(req.method, RtspMethod::Options);
        assert_eq!(req.uri, "rtsp://server/live");
        assert_eq!(req.headers.get("cseq").map(String::as_str), Some("1"));
        assert_eq!(n, raw.len());
    }

    #[test]
    fn parse_describe_request_no_body() {
        let raw = b"DESCRIBE rtsp://x/y RTSP/2.0\r\nCSeq: 5\r\n\r\n";
        let (req, n) = RtspRequest::parse(raw).unwrap();
        assert_eq!(req.method, RtspMethod::Describe);
        assert_eq!(req.version, RtspVersion::V2_0);
        assert_eq!(n, raw.len());
    }

    #[test]
    fn parse_setup_with_transport_header() {
        let raw = b"SETUP rtsp://x/y RTSP/1.0\r\n\
                    CSeq: 3\r\n\
                    Transport: RTP/AVP;unicast;client_port=5004-5005\r\n\
                    \r\n";
        let (req, _) = RtspRequest::parse(raw).unwrap();
        assert_eq!(req.method, RtspMethod::Setup);
        let t = req.headers.get("transport").unwrap();
        assert!(t.contains("client_port=5004-5005"));
    }

    #[test]
    fn parse_unsupported_method_errors() {
        let raw = b"FOOBAR rtsp://x RTSP/1.0\r\n\r\n";
        let e = RtspRequest::parse(raw).unwrap_err();
        assert!(matches!(e, RtspError::BadResponse { .. }));
    }

    #[test]
    fn parse_truncated_no_crlfcrlf_errors() {
        let raw = b"OPTIONS rtsp://x RTSP/1.0\r\nCSeq: 1\r\n";
        let e = RtspRequest::parse(raw).unwrap_err();
        assert!(matches!(e, RtspError::BadResponse { .. }));
    }

    #[test]
    fn parse_request_with_body() {
        let body = b"v=0\r\ns=test\r\n";
        let mut raw = format!(
            "SETUP rtsp://x/y RTSP/1.0\r\nCSeq: 2\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        raw.extend_from_slice(body);
        let (req, n) = RtspRequest::parse(&raw).unwrap();
        assert_eq!(req.method, RtspMethod::Setup);
        assert_eq!(req.body.as_ref(), body);
        assert_eq!(n, raw.len());
    }

    #[test]
    fn encode_response_round_trips_minimal() {
        let mut headers = HashMap::new();
        headers.insert("cseq".into(), "1".into());
        let resp = RtspResponse {
            version: RtspVersion::V1_0,
            status: 200,
            reason: "OK".into(),
            headers,
            body: Bytes::new(),
        };
        let wire = resp.encode();
        let text = std::str::from_utf8(&wire).unwrap();
        assert!(text.starts_with("RTSP/1.0 200 OK\r\n"));
        assert!(text.contains("Cseq: 1\r\n") || text.contains("CSeq: 1\r\n"));
        assert!(text.ends_with("\r\n\r\n"));
    }

    #[test]
    fn encode_response_with_body_appends_body() {
        let body = b"v=0\r\ns=test\r\n";
        let mut headers = HashMap::new();
        headers.insert("cseq".into(), "2".into());
        headers.insert("content-type".into(), "application/sdp".into());
        headers.insert("content-length".into(), body.len().to_string());
        let resp = RtspResponse {
            version: RtspVersion::V1_0,
            status: 200,
            reason: "OK".into(),
            headers,
            body: Bytes::copy_from_slice(body),
        };
        let wire = resp.encode();
        // Should end with the body bytes (not CRLFCRLF).
        assert!(wire.ends_with(body));
        // And the CRLFCRLF header terminator should be present somewhere inside.
        assert!(wire.windows(4).any(|w| w == b"\r\n\r\n"));
    }
}
