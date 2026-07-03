//! `RtspClient::teardown` + `Drop` impl (best-effort TEARDOWN on drop).

use crate::error::RtspError;
use crate::rtsp::client::RtspClient;
use crate::rtsp::message::RtspMethod;

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
        self.teardown_with_deadline(None)
    }

    /// Variant of [`Self::teardown`] with an optional response deadline.
    /// When `deadline` elapses with no TEARDOWN response, returns
    /// [`RtspError::Io`] with [`std::io::ErrorKind::TimedOut`] but still
    /// clears `session_id` (so callers know not to retry teardown).
    ///
    /// Called from `Drop for RtspClient` with a ~500 ms deadline so the
    /// destructor stays bounded even when the peer silently half-closed
    /// (no FIN on the wire). The wire bytes still go out (write_all
    /// succeeds against the kernel send buffer); we just don't wait
    /// forever for an answer.
    pub(crate) fn teardown_with_deadline(
        &mut self,
        deadline: Option<std::time::Instant>,
    ) -> Result<(), RtspError> {
        let sid = match &self.session_id {
            Some(s) => s.clone(),
            None => return Ok(()), // nothing to tear down
        };
        let uri = self.url.render_no_credentials();
        let req = self
            .base_request(RtspMethod::Teardown, uri)
            .header("session", sid);
        let bytes = req.encode_checked()?;
        // Use the deadline-aware send if a pump is active; the non-pump
        // path's `send_and_read` already bounds via cancel-poll. With a
        // deadline, route through the pump variant unconditionally.
        let r = if self.pump_state.is_some() {
            self.send_and_read_via_pump_with_deadline(&bytes, deadline)
        } else {
            self.send_and_read(&bytes)
        };
        // Always clear session_id — caller asked us to tear down; if it
        // failed, retrying won't help and we want Drop to skip on a
        // second pass.
        self.session_id = None;
        r.map(|_| ())
    }
}

// `Drop for RtspClient` is implemented in `client/mod.rs` (combines
// teardown + keepalive-thread join). This file holds only the
// teardown method itself.
