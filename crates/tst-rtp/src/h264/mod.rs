//! RFC 6184 H.264 RTP payload format support.
//!
//! This module provides SDP attribute parsing, the depacketizer state
//! machine for reassembling H.264 NALUs from RTP packets, and the
//! blocking I/O receiver shell.
//!
//! # SDP parsing
//!
//! Use [`parse_rtpmap_h264`] to discover the dynamic payload type assigned
//! to H.264 from the `a=rtpmap` lines, then [`H264FmtpParams::parse`] to
//! extract packetization mode, out-of-band parameter sets (SPS/PPS), and
//! profile information from `a=fmtp`.
//!
//! # Depacketizer
//!
//! Use [`H264Depacketizer`] to reassemble Access Units from RTP packets:
//! call [`H264Depacketizer::feed`] for each received packet then drain
//! [`H264Depacketizer::next_au`] until it returns `None`.
//!
//! # Receiver
//!
//! Use [`H264Receiver`] for a blocking I/O shell that wraps a UDP socket
//! or TCP-interleaved mpsc channel, drives the depacketizer, and returns
//! [`H264Au`]s via [`H264Receiver::recv_au`].

pub mod depacketizer;
pub mod fmtp;
pub mod receiver;

pub use depacketizer::{
    H264Au, H264Depacketizer, H264DepayConfig, H264DepayStats, ParameterSetInjection,
};
pub use fmtp::{H264FmtpParams, parse_rtpmap_h264};
pub use receiver::H264Receiver;
