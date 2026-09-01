//! RTSP client C ABI — `tst_rtsp_client_builder_*` and `tst_rtsp_session_*`.
//!
//! Wraps `tst_rtp::RtspClientBuilder` and `tst_rtp::RtspClient` (and the
//! resulting `RtspSession`) with a sync C ABI.
//!
//! The builder + auth entry points (`builder`):
//! - `tst_rtsp_client_builder_new` — allocate and parse URL
//! - `tst_rtsp_client_builder_transport_pref` — set preferred transport
//! - `tst_rtsp_client_builder_keepalive` — enable/disable auto-keepalive
//! - `tst_rtsp_client_builder_tls_root_cert_pem` — supply CA certificate
//! - `tst_rtsp_client_builder_auth_basic` — Basic credentials
//! - `tst_rtsp_client_builder_auth_digest_md5` — Digest MD5 credentials
//! - `tst_rtsp_client_builder_auth_digest_sha256` — Digest SHA-256 credentials
//! - `tst_rtsp_client_builder_free` — discard without connecting
//!
//! The session entry points (`session`):
//! - `tst_rtsp_client_builder_connect` — consume builder, run DESCRIBE+SETUP
//! - `tst_rtsp_session_play` — send PLAY
//! - `tst_rtsp_session_pause` — send PAUSE
//! - `tst_rtsp_session_teardown_and_free` — send TEARDOWN + free
//! - `tst_rtsp_session_cancel` — cancel blocking control-plane I/O
//! - `tst_rtsp_session_into_demux_receiver` — bridge to TstRtpDemuxReceiver

pub(crate) mod auth;
pub(crate) mod builder;
pub(crate) mod session;
