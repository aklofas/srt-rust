//! Domain harness: RTSP server: mounts, auth, multicast, TLS, shutdown, interleaved/UDP transports
//! (consolidated from the former per-file tests/*.rs — see tests/MOVEMENT_MAP.md).
//!
//! Each `mod` below is one former top-level integration-test file, now
//! compiled into this single binary. Test bodies are unchanged; only the
//! module path gained a `rtsp_server::<file>::` prefix.

// Shared fixtures (loopback RTSP server, self-signed TLS certs), declared
// once at the binary root so `crate::fixtures::*` resolves for every member.
#[path = "fixtures/mod.rs"]
mod fixtures;
#[path = "rtsp_server/auth_basic.rs"]
mod auth_basic;
#[path = "rtsp_server/auth_digest.rs"]
mod auth_digest;
#[path = "rtsp_server/bind.rs"]
mod bind;
#[path = "rtsp_server/concurrent.rs"]
mod concurrent;
#[path = "rtsp_server/lagging_peer.rs"]
mod lagging_peer;
#[path = "rtsp_server/loopback_interleaved.rs"]
mod loopback_interleaved;
#[path = "rtsp_server/loopback_udp.rs"]
mod loopback_udp;
#[path = "rtsp_server/mixed_transports.rs"]
mod mixed_transports;
#[path = "rtsp_server/mount.rs"]
mod mount;
#[path = "rtsp_server/multicast.rs"]
mod multicast;
#[path = "rtsp_server/notice_5402.rs"]
mod notice_5402;
#[path = "rtsp_server/session_keepalive.rs"]
mod session_keepalive;
#[path = "rtsp_server/shutdown.rs"]
mod shutdown;
#[path = "rtsp_server/tls.rs"]
mod tls;
