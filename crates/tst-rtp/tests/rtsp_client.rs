//! Domain harness: RTSP client: SETUP/PLAY/TEARDOWN, auth, fallback, TLS, keepalive, interleaved
//! (consolidated from the former per-file tests/*.rs — see tests/MOVEMENT_MAP.md).
//!
//! Each `mod` below is one former top-level integration-test file, now
//! compiled into this single binary. Test bodies are unchanged; only the
//! module path gained a `rtsp_client::<file>::` prefix.

// Shared fixtures (loopback RTSP server, self-signed TLS certs), declared
// once at the binary root so `crate::fixtures::*` resolves for every member.
#[path = "fixtures/mod.rs"]
mod fixtures;
#[path = "rtsp_client/auth.rs"]
mod auth;
#[path = "rtsp_client/builder_timeouts.rs"]
mod builder_timeouts;
#[path = "rtsp_client/fallback.rs"]
mod fallback;
#[path = "rtsp_client/interleaved_e2e.rs"]
mod interleaved_e2e;
#[path = "rtsp_client/keepalive.rs"]
mod keepalive;
#[path = "rtsp_client/keepalive_overflow.rs"]
mod keepalive_overflow;
#[path = "rtsp_client/keepalive_retune.rs"]
mod keepalive_retune;
#[path = "rtsp_client/setup_play.rs"]
mod setup_play;
#[path = "rtsp_client/teardown.rs"]
mod teardown;
#[path = "rtsp_client/tls.rs"]
mod tls;
#[path = "rtsp_client/tls_keepalive.rs"]
mod tls_keepalive;
