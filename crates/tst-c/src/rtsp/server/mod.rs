//! RTSP server C ABI — `tst_rtsp_server_builder_*` and mount handle methods.
//!
//! Wraps `tst_rtp::RtspServerBuilder`, `tst_rtp::RtspServer`, and
//! `tst_rtp::MountHandle` with a sync C ABI.
//!
//! Task breakdown:
//! - T7 (Wave B): `tst_rtsp_server_builder_new` + setter chain
//!   (`_auth_basic`, `_auth_digest_md5`, `_auth_digest_sha256`,
//!    `_max_sessions`, `_fanout_capacity`, etc.) + `_free`.
//! - T8 (Wave B, this task): `tst_rtsp_server_builder_start` + mount creation
//!   (`_add_unicast_mount`, `_add_multicast_mount`) + opaque scaffolds for
//!   `TstRtspServer` and `TstRtspMountHandle`.
//! - T9 (Wave C): push methods on `TstRtspMountHandle`
//!   (`push_video`, `push_video_to`, `push_klv`, `push_klv_to`, `push_audio`,
//!    `push_audio_to`, `push_subtitle`, `push_subtitle_to`).
//! - T10 (Wave C): server-level stats, stop, cancel, free.

pub(crate) mod mount;
pub(crate) mod start;
pub(crate) mod types;
