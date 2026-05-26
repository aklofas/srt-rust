//! `RtspClient::play` + `RtspClient::pause` + `RTP-Info:` header parsing.

use std::io::Write;

use crate::error::RtspError;
use crate::rtsp::client::RtspClient;
use crate::rtsp::message::{RtspMethod, RtspRequest};

/// `RTP-Info:` header from a PLAY response. Tells us the first RTP
/// sequence number and RTP timestamp the server will emit, which
/// callers can use to align demuxer state.
#[derive(Debug, Clone, Default)]
pub struct RtpInfo {
    pub url: Option<String>,
    pub seq: Option<u16>,
    pub rtptime: Option<u32>,
}

impl RtspClient {
    /// Send PLAY. Returns parsed `RTP-Info:` header (if present) so
    /// caller can align demuxer.
    ///
    /// # Errors
    ///
    /// - `RtspError::BadResponse` with `detail: "PLAY before SETUP"` if
    ///   the client has no active session id.
    /// - `RtspError::Io` on socket-level failure.
    /// - `RtspError::Protocol` on non-200 server status.
    pub fn play(&mut self) -> Result<RtpInfo, RtspError> {
        let sid = self
            .session_id
            .as_ref()
            .ok_or(RtspError::BadResponse {
                detail: "PLAY before SETUP",
            })?
            .clone();
        let cseq = self.bump_cseq();
        let req = RtspRequest::new(
            RtspMethod::Play,
            self.url.render_no_credentials(),
            self.url.rtsp_version,
        )
        .header("cseq", cseq.to_string())
        .header("session", sid)
        .header("range", "npt=0.000-")
        .header("user-agent", "tst-rtp/0.1");
        let bytes = req.encode();
        self.stream
            .write_all(&bytes)
            .map_err(|e| RtspError::Io(e.kind()))?;
        let resp = self.read_response()?;
        if resp.status != 200 {
            return Err(RtspError::Protocol {
                code: resp.status,
                reason: resp.reason,
            });
        }
        Ok(parse_rtp_info(
            resp.headers
                .get("rtp-info")
                .map(|s| s.as_str())
                .unwrap_or(""),
        ))
    }

    /// Send PAUSE. Stream stops but session remains valid for a
    /// subsequent PLAY.
    ///
    /// # Errors
    ///
    /// - `RtspError::BadResponse` with `detail: "PAUSE before SETUP"`
    ///   if the client has no active session id.
    /// - `RtspError::Io` on socket-level failure.
    /// - `RtspError::Protocol` on non-200 server status.
    pub fn pause(&mut self) -> Result<(), RtspError> {
        let sid = self
            .session_id
            .as_ref()
            .ok_or(RtspError::BadResponse {
                detail: "PAUSE before SETUP",
            })?
            .clone();
        let cseq = self.bump_cseq();
        let req = RtspRequest::new(
            RtspMethod::Pause,
            self.url.render_no_credentials(),
            self.url.rtsp_version,
        )
        .header("cseq", cseq.to_string())
        .header("session", sid)
        .header("user-agent", "tst-rtp/0.1");
        let bytes = req.encode();
        self.stream
            .write_all(&bytes)
            .map_err(|e| RtspError::Io(e.kind()))?;
        let resp = self.read_response()?;
        if resp.status != 200 {
            return Err(RtspError::Protocol {
                code: resp.status,
                reason: resp.reason,
            });
        }
        Ok(())
    }
}

/// Parse an `RTP-Info:` header value like
/// `url=rtsp://cam/h264/streamid=0;seq=1234;rtptime=5000000`.
pub fn parse_rtp_info(s: &str) -> RtpInfo {
    let mut out = RtpInfo::default();
    for part in s.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("url=") {
            out.url = Some(v.to_string());
        } else if let Some(v) = part.strip_prefix("seq=") {
            out.seq = v.parse().ok();
        } else if let Some(v) = part.strip_prefix("rtptime=") {
            out.rtptime = v.parse().ok();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rtp_info_full() {
        let s = "url=rtsp://cam/h264/streamid=0;seq=1234;rtptime=5000000";
        let info = parse_rtp_info(s);
        assert_eq!(info.seq, Some(1234));
        assert_eq!(info.rtptime, Some(5000000));
        assert_eq!(info.url.as_deref(), Some("rtsp://cam/h264/streamid=0"));
    }

    #[test]
    fn parse_rtp_info_seq_only() {
        let info = parse_rtp_info("seq=42");
        assert_eq!(info.seq, Some(42));
        assert!(info.url.is_none());
        assert!(info.rtptime.is_none());
    }

    #[test]
    fn parse_rtp_info_empty() {
        let info = parse_rtp_info("");
        assert_eq!(info.seq, None);
    }
}
