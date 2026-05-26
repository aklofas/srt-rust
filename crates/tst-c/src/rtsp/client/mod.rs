//! RTSP client C ABI — `tst_rtsp_client_builder_*` and `tst_rtsp_session_*`.
//!
//! Wraps `tst_rtp::RtspClientBuilder` and `tst_rtp::RtspClient` (and the
//! resulting `RtspSession`) with a sync C ABI.
//!
//! Task 5 (Wave A) ships the builder + auth entry points:
//! - `tst_rtsp_client_builder_new` — allocate and parse URL
//! - `tst_rtsp_client_builder_transport_pref` — set preferred transport
//! - `tst_rtsp_client_builder_keepalive` — enable/disable auto-keepalive
//! - `tst_rtsp_client_builder_tls_root_cert_pem` — supply CA certificate
//! - `tst_rtsp_client_builder_auth_basic` — Basic credentials
//! - `tst_rtsp_client_builder_auth_digest_md5` — Digest MD5 credentials
//! - `tst_rtsp_client_builder_auth_digest_sha256` — Digest SHA-256 credentials
//! - `tst_rtsp_client_builder_free` — discard without connecting
//!
//! Task 6 (Wave B) adds the connect + session entry points.

pub(crate) mod auth;
pub(crate) mod builder;
// session.rs lands in Task 6 (Wave B).
