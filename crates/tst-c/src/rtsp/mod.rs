//! `tst_rtsp_*` C ABI entry points. Gated on `feature = "rtp"` because
//! RTSP requires the underlying RTP data plane.
//!
//! Provides RTSP client (camera ingest) and server (publishing) C ABI.
//! Entry points land in Tasks 5-9 (Waves A-C). This stub satisfies the
//! module declaration in `lib.rs` for the bootstrap commit.

pub(crate) mod client;
pub(crate) mod server;
