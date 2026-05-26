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
