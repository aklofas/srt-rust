//! `RtspClient::teardown` + `Drop` impl (best-effort TEARDOWN on drop).

use std::io::Write;

use crate::error::RtspError;
use crate::rtsp::client::RtspClient;
use crate::rtsp::message::{RtspMethod, RtspRequest};

impl RtspClient {
    /// Send TEARDOWN. After this returns, no further methods succeed.
    ///
    /// If no session is currently established (no prior successful
    /// SETUP, or [`Self::teardown`] already ran), this is a no-op that
    /// returns `Ok(())`.
    ///
    /// # Errors
    ///
    /// - [`RtspError::Io`] on socket-level failure writing the request
    ///   or reading the response.
    /// - [`RtspError::BadResponse`] (or another variant) surfaced by
    ///   the response reader if the server replies malformed bytes.
    pub fn teardown(&mut self) -> Result<(), RtspError> {
        let sid = match &self.session_id {
            Some(s) => s.clone(),
            None => return Ok(()), // nothing to tear down
        };
        let cseq = self.bump_cseq();
        let req = RtspRequest::new(
            RtspMethod::Teardown,
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
        let _resp = self.read_response()?;
        self.session_id = None;
        Ok(())
    }
}

// `Drop for RtspClient` is implemented in `client/mod.rs` (combines
// teardown + keepalive-thread join). This file holds only the
// teardown method itself.
