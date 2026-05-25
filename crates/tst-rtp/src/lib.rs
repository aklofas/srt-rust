//! TS Transformer RTP transport — RTP-over-UDP per RFC 3550 carrying an
//! MPEG-TS bytestream per RFC 2250.
//!
//! This crate provides the RTP-specific concrete types. The
//! [`Transport`](tst_core::transport::Transport) /
//! [`RecvTransport`](tst_core::transport::RecvTransport) traits themselves
//! live in [`tst_core`]; the transport-agnostic Sender/Receiver shells
//! live in `tst_pipeline`.
//!
//! Phase 1 ships the RTP data plane only — RTSP signaling lands in
//! Phase 2 and is not exposed here.

#![warn(rustdoc::broken_intra_doc_links)]

pub mod clock;
pub mod init;

pub use clock::RtpClock;
