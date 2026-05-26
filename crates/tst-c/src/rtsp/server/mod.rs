//! RTSP server C ABI — `tst_rtsp_server_builder_*` and mount handle methods.
//!
//! Wraps `tst_rtp::RtspServerBuilder`, `tst_rtp::RtspServer`, and
//! `tst_rtp::MountHandle` with a sync C ABI. Full implementation lands
//! in Tasks 7-9 (Waves B-C).
//!
//! Task 7 (Wave B) ships the builder + auth + TLS + lifecycle-config entry points:
//! - `tst_rtsp_server_builder_new` — allocate and parse bind URL
//! - `tst_rtsp_server_builder_bind` — override the bind address
//! - `tst_rtsp_server_builder_auth_basic` — Basic auth credentials
//! - `tst_rtsp_server_builder_auth_digest_md5` — Digest MD5 credentials
//! - `tst_rtsp_server_builder_auth_digest_sha256` — Digest SHA-256 credentials
//! - `tst_rtsp_server_builder_max_sessions` — cap on concurrent sessions
//! - `tst_rtsp_server_builder_session_timeout` — advertised session timeout
//! - `tst_rtsp_server_builder_fanout_capacity` — broadcast channel capacity
//! - `tst_rtsp_server_builder_graceful_shutdown_drain_ms` — drain window
//! - `tst_rtsp_server_builder_tls_cert_pem` — TLS cert chain + private key
//! - `tst_rtsp_server_builder_free` — discard without starting

pub(crate) mod builder;
