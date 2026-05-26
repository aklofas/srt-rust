//! RTSP server — accepts client connections, manages sessions, fans out
//! one Muxer's TS bytes to N connected peers.
//!
//! Phase 3 — populated across Waves A through G. This file ships:
//! - Module declarations for all server submodules.
//! - Public `RtspServer` shell (real Runtime wiring in Task 7).
//! - `bind()` / `bind_with()` convenience constructors that route
//!   through `RtspServerBuilder`.

pub mod auth;
pub mod builder;
pub mod fanout;
pub mod handlers;
pub mod interleaved_pump;
pub mod listener;
pub mod mount;
pub mod multicast;
pub mod runtime;
pub mod session;
#[cfg(feature = "tls")]
pub mod tls;

use crate::builder::RtspServerBuilder;
use crate::error::RtspServerError;

/// RTSP server — accepts client connections, manages sessions, fans out
/// one Muxer's TS bytes to N connected peers.
///
/// Phase 3 implementation. The Task 3 stub returns
/// [`RtspServerError::NotStarted`] from `from_builder` (pub(crate)); the
/// real implementation lands in Task 7 (Wave B).
#[derive(Debug)]
pub struct RtspServer {
    // State added by Tasks 7-19.
}

impl RtspServer {
    /// Internal — called from [`crate::builder::RtspServerBuilder::build`].
    /// Real impl lands in Task 7 (runtime.rs + listener.rs).
    pub(crate) fn from_builder(b: RtspServerBuilder) -> Result<Self, RtspServerError> {
        // `bind_url`, `auth`, and the timing/cap knobs are stored on the
        // builder for Task 7 (Wave B) to consume when wiring up the tokio
        // Runtime + tcp listener. Reading them here keeps the dead-code
        // lint quiet without changing visibility.
        let _ = (
            &b.bind_url,
            &b.auth,
            b.max_sessions,
            b.session_timeout,
            b.fanout_capacity,
            b.graceful_shutdown_drain,
        );
        #[cfg(feature = "tls")]
        let _ = (&b.tls_cert_path, &b.tls_key_path);
        Err(RtspServerError::NotStarted)
    }

    /// Convenience: `RtspServerBuilder::new(url)?.build()`.
    pub fn bind(url: &str) -> Result<Self, RtspServerError> {
        RtspServerBuilder::new(url)?.build()
    }

    /// Convenience: `RtspServerBuilder::with_url(url).build()`.
    pub fn bind_with(url: crate::url::RtspUrl) -> Result<Self, RtspServerError> {
        RtspServerBuilder::with_url(url).build()
    }
}
