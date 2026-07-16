//! Background thread sending OPTIONS at `session_timeout / 2` intervals.
//!
//! - Holds an `Arc<Mutex<Stream>>` clone — the SAME stream the main
//!   thread uses (since T21). Works uniformly for plain TCP and TLS;
//!   the pre-T21 path tried to `try_clone` the stream FD which failed
//!   on rustls `ClientConnection` (silently disabling keepalive for
//!   `rtsps://` sessions).
//! - On cancel (or write error), the thread exits.
//! - On RTSP session timeout (server stops responding), the thread sets
//!   a shared "session_dead" flag the main thread can poll via
//!   `RtspClient::is_session_alive`.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use secrecy::SecretString;

use crate::rtsp::client::Stream;
use crate::rtsp::message::{RtspMethod, RtspRequest};
use crate::url::RtspVersion;

/// Spawn the rtsp-keepalive background thread.
///
/// `cancel` flips to true when the [`crate::rtsp::client::RtspCancelHandle::cancel`]
/// is invoked (or when the `RtspClient` is dropped). The thread polls it
/// every 200 ms and exits cleanly at the next wake.
///
/// `session_dead` flips to true when a write to the control TCP fails;
/// the main thread can poll it (via `RtspClient::is_session_alive`) and
/// take recovery action.
///
/// `interval` is the OPTIONS-ping cadence — typically
/// `session_timeout / 2` (so a 60 s server timeout pings every 30 s).
///
/// `session_id` is a shared cell the main thread updates when SETUP
/// returns a new ID; the keepalive emits `Session: <id>` when present.
/// Pre-SETUP keepalives (used as connectivity probes) omit the header.
///
/// CSeq starts at `1_000_000` to avoid colliding with the main thread's
/// counter (which starts at 1 and increments per request).
///
/// Returns `Err` if the OS refuses to spawn the thread (resource
/// exhaustion). This MUST be propagated rather than `.expect()`'d — the
/// keepalive is started on the RTSP connect path and the JVM/C bindings do
/// not catch unwinds across the FFI boundary, so a panic here would abort
/// the host process. The caller maps the `io::Error` to a typed
/// `RtspError`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn(
    write_half: Arc<Mutex<Stream>>,
    cancel: Arc<AtomicBool>,
    session_dead: Arc<AtomicBool>,
    interval: Duration,
    url: String,
    version: RtspVersion,
    session_id: Arc<Mutex<Option<String>>>,
    user_agent: String,
    auth_challenge: Arc<Mutex<Option<String>>>,
    username: Option<String>,
    password: Option<SecretString>,
    auth_nc: Arc<AtomicU32>,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("rtsp-keepalive".to_string())
        .spawn(move || {
            let mut cseq = 1_000_000u32; // far above main-thread cseqs to avoid collision
            loop {
                // Wake every 200 ms to check the cancel flag — keeps
                // teardown latency bounded even when `interval` is large
                // (e.g., 30 s for a default 60 s session timeout).
                let deadline = std::time::Instant::now() + interval;
                while std::time::Instant::now() < deadline {
                    if cancel.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
                // One more cancel check before we burn the bytes on the wire.
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                cseq += 1;
                let mut req = RtspRequest::new(RtspMethod::Options, url.clone(), version)
                    .header("cseq", cseq.to_string())
                    .header("user-agent", user_agent.as_str());
                if let Some(sid) = session_id
                    .lock()
                    .expect("session id mutex poisoned")
                    .clone()
                {
                    req = req.header("session", sid);
                }
                // Attach cached credentials pre-emptively so servers that
                // require auth on OPTIONS accept the ping and refresh the
                // session. No-op until the challenge is learned (at DESCRIBE).
                if let (Some(www), Some(user), Some(pass)) = (
                    auth_challenge
                        .lock()
                        .expect("auth challenge mutex poisoned")
                        .clone(),
                    username.as_ref(),
                    password.as_ref(),
                ) {
                    let nc = auth_nc.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
                    if let Ok(authz) = crate::rtsp::auth::build_authorization(
                        RtspMethod::Options,
                        &url,
                        &www,
                        user,
                        pass,
                        nc,
                    ) {
                        req = req.header("authorization", authz);
                    }
                }
                // Validate against header injection before writing. The
                // User-Agent was already validated at connect time and the
                // session id is server-issued, so this should never fail; if
                // it somehow does, treat the session as dead rather than
                // smuggling bytes onto the wire.
                let Ok(bytes) = req.encode_checked() else {
                    session_dead.store(true, Ordering::Relaxed);
                    return;
                };
                // If the stream mutex is poisoned the main thread
                // panicked mid-request — propagate by panicking the
                // keepalive thread too (per T21 policy).
                let mut g = write_half.lock().expect("stream mutex poisoned");
                if g.write_all(&bytes).is_err() {
                    session_dead.store(true, Ordering::Relaxed);
                    return;
                }
                // The main thread's `read_response` loop will pick up
                // the 200 OK response from this keepalive (interleaved
                // with any other in-flight responses by CSeq matching).
            }
        })
}
